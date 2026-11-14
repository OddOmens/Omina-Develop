//! The start screen, Develop's Home, laid out like Studio's and Canvas's: shown over the editor
//! on launch (unless Settings says to reopen the last project), from File ▸ Start Screen, a
//! click on the mark in the top left corner, and after Close.
//!
//! Home has a welcome with the two ways to begin (open a photo, open a project), then the
//! recent projects as cards, each with the preview saved inside its file. A photo dropped
//! anywhere on it opens.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, InteractiveElement, IntoElement, ParentElement, RenderImage, StatefulInteractiveElement, Styled, Subscription, TestSupportExt as _,
    Window, div, linear_color_stop, linear_gradient, prelude::FluentBuilder as _, px,
};

use crate::app_settings::{self, RecentProject};
use crate::theme::{color, hsla, size};
use crate::workspace::Workspace;

/// The start screen while it is showing.
pub struct StartScreen {
    /// Narrows the recent list by name or folder.
    pub filter: Entity<InputState>,
    /// Whether Home lists every recent project, not just the latest few.
    pub all_recent: bool,
    _filter_changes: Subscription,
}

/// The previews read from recent project files, by path, with the file's modified time they
/// were read at (a newer save reads it again). `None` while reading, or for a file without one.
#[derive(Default)]
pub struct RecentPreviews(HashMap<PathBuf, (Option<SystemTime>, Option<Arc<RenderImage>>)>);

/// Shows the start screen.
pub fn show(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let filter = cx.new(|cx| InputState::new(window, cx).placeholder("Filter recent projects"));
    // Typing in the filter redraws the list.
    let changes = cx.subscribe_in(&filter, window, |_, _, event: &InputEvent, _, cx| {
        if matches!(event, InputEvent::Change) {
            cx.notify();
        }
    });
    workspace.ui.start_screen = Some(StartScreen { filter, all_recent: false, _filter_changes: changes });
    cx.notify();
}

/// Goes to the start screen from the editor (the mark in the top left, or File ▸ Start
/// Screen). The photo stays open behind it; Esc or the close button goes back to it.
pub fn go_to(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    show(workspace, window, cx);
}

/// Back to the editor. With no photo open there is nothing behind it, so it stays.
pub fn hide(workspace: &mut Workspace) {
    if workspace.state.photo().is_some() {
        workspace.ui.start_screen = None;
    }
}

pub fn render(workspace: &mut Workspace, cx: &mut Context<Workspace>) -> Option<AnyElement> {
    workspace.ui.start_screen.as_ref()?;
    let recent = app_settings::load().recent_projects;
    load_previews(workspace, &recent, cx);
    let screen = workspace.ui.start_screen.as_ref()?;
    let page = home_page(screen, &recent, &workspace.ui.recent_previews, workspace.state.photo().is_some(), cx);
    let body = div().relative().flex_1().min_h_0().flex().flex_col().child(accent_glow(320.0)).child(page);
    let screen = div()
        .id("start-screen")
        .absolute()
        .inset_0()
        .flex()
        .flex_col()
        .bg(hsla(color::BACKGROUND))
        .text_color(hsla(color::TEXT))
        .text_size(px(size::TEXT))
        .occlude()
        // Photos or projects dropped from the Finder open, each in a tab of its own.
        .drag_over::<gpui_kit::ExternalPaths>(|style, _, _, _| style.bg(hsla(color::HOVER)))
        .on_drop(cx.listener(|this, paths: &gpui_kit::ExternalPaths, window, cx| {
            this.open_paths(paths.paths().to_vec(), window, cx);
        }))
        .child(top_bar(workspace.state.photo().is_some(), cx))
        .child(body)
        .test_support();
    Some(screen.into_any_element())
}

/// Starts reading the previews of recent projects not read yet (or saved again since).
fn load_previews(workspace: &mut Workspace, recent: &[RecentProject], cx: &mut Context<Workspace>) {
    for project in recent.iter().take(40) {
        let modified = std::fs::metadata(&project.path).and_then(|metadata| metadata.modified()).ok();
        if workspace.ui.recent_previews.0.get(&project.path).is_some_and(|(seen, _)| *seen == modified) {
            continue;
        }
        workspace.ui.recent_previews.0.insert(project.path.clone(), (modified, None));
        let path = project.path.clone();
        cx.spawn(async move |this, cx| {
            let reading = path.clone();
            let picture = cx.background_spawn(async move { develop_core::file::read_preview(&reading).map(|bitmap| crate::preview::to_render_image(&bitmap, 0)) }).await;
            this.update(cx, |this, cx| {
                if let Some(entry) = this.ui.recent_previews.0.get_mut(&path) {
                    entry.1 = picture;
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }
}

/// A soft wash of the accent color fading down from the top of the screen, drawn behind its
/// content so a full-window page isn't a flat field of one color.
pub(crate) fn accent_glow(height: f32) -> gpui_kit::Div {
    div().absolute().top_0().left_0().right_0().h(px(height)).bg(linear_gradient(
        180.0,
        linear_color_stop(hsla(color::ACCENT).opacity(0.14), 0.0),
        linear_color_stop(hsla(color::ACCENT).opacity(0.0), 1.0),
    ))
}

/// Develop's mark: an aperture on an accent tile, `side` points square. (Studio's is a
/// clapperboard and Canvas's a palette on the same tile.)
pub(crate) fn brand_mark(side: f32) -> gpui_kit::Div {
    div()
        .size(px(side))
        .flex_none()
        .rounded(px(side * 0.24))
        .flex()
        .items_center()
        .justify_center()
        .bg(linear_gradient(135.0, linear_color_stop(hsla(color::ACCENT), 0.0), linear_color_stop(hsla(color::ACCENT).opacity(0.7), 1.0)))
        .child(Icon::new(IconName::Aperture).size(px(side * 0.56)).text_color(gpui_kit::white()))
}

/// Develop's name on the left, and a button back to the photo on the right (when one is open).
fn top_bar(photo_open: bool, cx: &mut Context<Workspace>) -> impl IntoElement {
    use gpui_kit::component::button::{Button, ButtonVariants as _};
    let close = photo_open.then(|| {
        Button::new("start-close").ghost().icon(Icon::new(IconName::X)).tooltip("Back to the photo (Esc)").on_click(cx.listener(|this, _, _, cx| {
            hide(this);
            cx.notify();
        }))
    });
    div()
        .h(px(56.0))
        .flex_none()
        .flex()
        .items_center()
        .justify_between()
        .px_5()
        .bg(hsla(color::PANEL))
        .border_b_1()
        .border_color(hsla(color::HAIRLINE))
        .child(div().flex().items_center().gap_2().child(brand_mark(30.0)).child(div().text_size(px(16.0)).font_weight(gpui_kit::FontWeight::SEMIBOLD).child("Omina Develop")))
        .children(close)
}

/// How many recent projects Home lists until "View all" is pressed (a filter searches all).
const RECENT_SHOWN: usize = 8;

/// A section's title on the Home page.
fn section_title(title: &'static str) -> gpui_kit::Div {
    div().text_size(px(20.0)).font_weight(gpui_kit::FontWeight::BOLD).child(title)
}

/// The recent projects still on disk whose name or folder contains `filter` (any case).
pub fn shown_recent(recent: &[RecentProject], filter: &str) -> Vec<RecentProject> {
    let filter = filter.trim().to_lowercase();
    recent
        .iter()
        .filter(|project| project.path.exists())
        .filter(|project| filter.is_empty() || project.path.to_string_lossy().to_lowercase().contains(&filter))
        .cloned()
        .collect()
}

/// The Home page: the welcome, then the recent projects (or, before any, a note saying where
/// they will be listed).
fn home_page(screen: &StartScreen, recent: &[RecentProject], previews: &RecentPreviews, photo_open: bool, cx: &mut Context<Workspace>) -> impl IntoElement {
    let filter = screen.filter.read(cx).value().to_string();
    let on_disk = recent.iter().filter(|project| project.path.exists()).count();
    let matching = shown_recent(recent, &filter);
    // The latest few, unless a filter is narrowing the list or all were asked for.
    let limited = filter.trim().is_empty() && !screen.all_recent && matching.len() > RECENT_SHOWN;
    let shown = if limited { &matching[..RECENT_SHOWN] } else { &matching[..] };
    let now = app_settings::now_seconds();
    let cards: Vec<AnyElement> = shown
        .iter()
        .enumerate()
        .map(|(index, project)| {
            let picture = previews.0.get(&project.path).and_then(|(_, picture)| picture.clone());
            recent_card(index, project, picture, now, cx).into_any_element()
        })
        .collect();
    // "View all", beside the heading, while only the latest few are showing.
    let view_all = limited.then(|| {
        div()
            .id("start-recent-all")
            .text_color(hsla(color::MUTED))
            .cursor_pointer()
            .hover(|style| style.text_color(hsla(color::TEXT)))
            .on_click(cx.listener(|this, _, _, cx| {
                if let Some(screen) = this.ui.start_screen.as_mut() {
                    screen.all_recent = true;
                }
                cx.notify();
            }))
            .child(format!("View all {}", matching.len()))
            .test_support()
    });
    let list = div()
        .id("start-recent")
        .flex_none()
        .flex()
        .flex_wrap()
        .gap_4()
        .when(cards.is_empty(), |list| list.child(div().py_4().text_color(hsla(color::MUTED)).child("No recent projects match the filter.")))
        .children(cards);
    let filter_field = div().w(px(240.0)).child(Input::new(&screen.filter).prefix(Icon::new(IconName::Search).size(px(14.0)).text_color(hsla(color::MUTED))).cleanable(true));
    let recent_section = div()
        .flex()
        .flex_col()
        .gap_3()
        .child(div().flex().items_center().justify_between().child(section_title("Recent projects")).child(div().flex().items_center().gap_4().children(view_all).child(filter_field)))
        .child(list);
    // With nothing saved or opened yet, a note says where projects will be listed.
    let first_run_note = div()
        .id("start-recent-empty")
        .flex_none()
        .p_5()
        .flex()
        .items_center()
        .gap_3()
        .rounded(px(10.0))
        .border_1()
        .border_color(hsla(color::HAIRLINE))
        .bg(hsla(color::PANEL))
        .child(div().size(px(34.0)).flex_none().rounded_full().flex().items_center().justify_center().bg(hsla(color::ACCENT).opacity(0.15)).child(Icon::new(IconName::Image).size(px(16.0)).text_color(hsla(color::ACCENT))))
        .child(div().text_color(hsla(color::MUTED)).child("Projects you save will be listed here. Open a photo (JPEG, PNG, TIFF, WebP and more) to start editing, or drop one anywhere on this screen."))
        .test_support();
    div()
        .id("start-home-page")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .flex()
        .flex_col()
        .gap_6()
        .px(px(48.0))
        .pt(px(28.0))
        .pb_8()
        .child(hero(on_disk, photo_open, cx))
        .when(on_disk > 0, |page| page.child(recent_section))
        .when(on_disk == 0, |page| page.child(first_run_note))
}

/// The height of the Home page's hero, in points.
const HERO_HEIGHT: f32 = 300.0;

/// Bands of a photo's tonal range drawn across the hero's foot, dark to light, as the
/// histogram's zones read: a hint of what the app is for, behind the welcome.
fn hero_tones() -> gpui_kit::Div {
    let shades = [0x101012, 0x26262b, 0x3d3d44, 0x585862, 0x777782, 0x9a9aa5, 0xc2c2cb, 0xe8e8ee];
    div().absolute().left_0().right_0().bottom_0().h(px(8.0)).flex().children(shades.map(|shade| div().flex_1().h_full().bg(gpui_kit::rgb(shade)).opacity(0.6)))
}

/// One of the hero's large actions: an icon over its name, on a dark tile that lights up in
/// the accent color under the pointer.
fn hero_action(id: &'static str, icon: IconName, label: &'static str, on_click: impl Fn(&mut Workspace, &mut Window, &mut Context<Workspace>) + 'static, cx: &mut Context<Workspace>) -> impl IntoElement {
    div()
        .id(id)
        .w(px(184.0))
        .h(px(92.0))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_2()
        .rounded(px(16.0))
        .bg(hsla(color::BACKGROUND).opacity(0.72))
        .border_1()
        .border_color(gpui_kit::white().opacity(0.1))
        .cursor_pointer()
        .hover(|style| style.bg(hsla(color::ACCENT).opacity(0.28)).border_color(hsla(color::ACCENT)))
        .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
        .child(Icon::new(icon).size(px(20.0)))
        .child(div().text_size(px(14.0)).font_weight(gpui_kit::FontWeight::SEMIBOLD).child(label))
        .test_support()
}

/// The hero across the top of Home: a welcome in large type with a glow of the accent color
/// rising from its foot, and the ways to begin on a tray under it.
fn hero(recent: usize, photo_open: bool, cx: &mut Context<Workspace>) -> impl IntoElement {
    let accent = hsla(color::ACCENT);
    let (title, about) = if recent == 0 { ("Welcome to Omina Develop", "Bring out the best in every photo: light, color, detail and crop.") } else { ("Welcome back", "Pick up where you left off, or open another photo.") };
    let tray = div()
        .flex()
        .gap_3()
        .p_3()
        .rounded(px(24.0))
        .bg(accent.opacity(0.1))
        .border_1()
        .border_color(accent.opacity(0.4))
        .child(hero_action("start-open", IconName::ImagePlus, "Open Photo…", |this, window, cx| this.open(window, cx), cx))
        .when(photo_open, |tray| {
            tray.child(hero_action("start-back", IconName::SlidersHorizontal, "Keep Editing", |this, _, cx| {
                hide(this);
                cx.notify();
            }, cx))
        });
    div()
        .id("start-hero")
        .relative()
        .flex_none()
        .h(px(HERO_HEIGHT))
        .rounded(px(18.0))
        .overflow_hidden()
        .border_1()
        .border_color(hsla(color::HAIRLINE))
        .bg(hsla(color::PANEL))
        // The glow, rounded at the foot like the hero so it doesn't square off its corners.
        .child(div().absolute().left_0().right_0().bottom_0().h(px(HERO_HEIGHT * 0.8)).rounded_b(px(18.0)).bg(linear_gradient(0.0, linear_color_stop(accent.opacity(0.5), 0.0), linear_color_stop(accent.opacity(0.0), 1.0))))
        .child(hero_tones())
        .child(
            div()
                .absolute()
                .inset_0()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .child(div().text_size(px(52.0)).line_height(px(60.0)).font_weight(gpui_kit::FontWeight::BOLD).child(title))
                .child(div().text_size(px(16.0)).text_color(hsla(color::TEXT)).opacity(0.8).child(about))
                .child(div().pt_5().child(tray)),
        )
        .test_support()
}

/// A project's name: its file name without `.develop`.
fn display_name(path: &Path) -> String {
    path.file_stem().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned())
}

/// The folder a project is in, with the home folder written as `~`.
fn display_folder(path: &Path) -> String {
    let folder = path.parent().map(Path::to_path_buf).unwrap_or_default();
    match std::env::var_os("HOME").map(PathBuf::from) {
        Some(home) if folder.starts_with(&home) => format!("~/{}", folder.strip_prefix(&home).unwrap_or(&folder).display()).trim_end_matches('/').to_owned(),
        _ => folder.display().to_string(),
    }
}

/// How long ago `then` was, from `now` (both in seconds since 1970), in words: "just now",
/// "5 minutes ago", "9 hours ago", "2 days ago" and so on.
pub fn time_ago(then: u64, now: u64) -> String {
    let seconds = now.saturating_sub(then);
    let (count, unit) = match seconds {
        0..60 => return "just now".into(),
        60..3_600 => (seconds / 60, "minute"),
        3_600..86_400 => (seconds / 3_600, "hour"),
        86_400..604_800 => (seconds / 86_400, "day"),
        604_800..2_592_000 => (seconds / 604_800, "week"),
        2_592_000..31_536_000 => (seconds / 2_592_000, "month"),
        _ => (seconds / 31_536_000, "year"),
    };
    if count == 1 { format!("1 {unit} ago") } else { format!("{count} {unit}s ago") }
}

/// A file's size in words: kilobytes under a megabyte (rather than "0 MB"), else megabytes.
fn file_size(bytes: u64) -> String {
    if bytes < 1_000_000 { format!("{} KB", bytes.div_ceil(1000).max(1)) } else { format!("{:.1} MB", bytes as f64 / 1_000_000.0) }
}

/// The size of a recent project's card picture, in points: 3:2, a photo's usual shape.
const RECENT_PICTURE: (f32, f32) = (232.0, 154.7);

/// One recent project, as a card: the preview saved in it (an image icon until it's read),
/// then its name, when it was last used and its size. Its folder shows in a tooltip. A click
/// opens it; a right-click offers to show it in the Finder or take it off the list.
fn recent_card(index: usize, project: &RecentProject, picture: Option<Arc<RenderImage>>, now: u64, cx: &mut Context<Workspace>) -> impl IntoElement {
    use gpui_kit::component::menu::{ContextMenuExt as _, PopupMenuItem};
    let path = project.path.clone();
    let bytes = std::fs::metadata(&path).map_or(0, |metadata| metadata.len());
    let target = cx.weak_entity();
    let menu_path = path.clone();
    let folder: gpui_kit::SharedString = display_folder(&path).into();
    // The whole photo shows (not cropped to the card), over a soft wash.
    let picture = div()
        .w(px(RECENT_PICTURE.0))
        .h(px(RECENT_PICTURE.1))
        .flex_none()
        .rounded_t(px(11.0))
        .overflow_hidden()
        .flex()
        .items_center()
        .justify_center()
        .bg(linear_gradient(135.0, linear_color_stop(hsla(color::INSET), 0.0), linear_color_stop(hsla(color::ACCENT).opacity(0.16), 1.0)))
        .child(match picture {
            Some(picture) => gpui_kit::StyledImage::object_fit(gpui_kit::img(picture).size_full(), gpui_kit::ObjectFit::Contain).into_any_element(),
            None => Icon::new(IconName::Image).size(px(28.0)).text_color(hsla(color::MUTED)).into_any_element(),
        });
    div()
        .id(("start-recent-row", index))
        .w(px(RECENT_PICTURE.0))
        .flex_none()
        .flex()
        .flex_col()
        .rounded(px(12.0))
        .bg(hsla(color::PANEL))
        .border_1()
        .border_color(hsla(color::HAIRLINE))
        .cursor_pointer()
        .hover(|style| style.border_color(hsla(color::ACCENT)).bg(hsla(color::HOVER)))
        .tooltip(move |window, cx| gpui_kit::component::tooltip::Tooltip::new(folder.clone()).build(window, cx))
        .on_click(cx.listener({
            let path = path.clone();
            move |this, _, window, cx| this.open_path(path.clone(), window, cx)
        }))
        .child(picture)
        .child(
            div()
                .px_3()
                .py_2p5()
                .flex()
                .flex_col()
                .gap_0p5()
                .child(div().truncate().font_weight(gpui_kit::FontWeight::SEMIBOLD).child(display_name(&path)))
                .child(div().truncate().text_size(px(size::SMALL_TEXT)).text_color(hsla(color::MUTED)).child(format!("{} · {}", time_ago(project.used, now), file_size(bytes)))),
        )
        .test_support()
        .context_menu(move |menu, _, _| {
            let (reveal, forget, target) = (menu_path.clone(), menu_path.clone(), target.clone());
            menu.item(PopupMenuItem::new("Show in Finder").on_click(move |_, window, cx| {
                use gpui_kit::component::WindowExt as _;
                if let Err(error) = std::process::Command::new("open").arg("-R").arg(&reveal).spawn() {
                    window.push_notification(format!("Couldn't show the project in the Finder: {error}").as_str(), cx);
                }
            }))
            .item(PopupMenuItem::new("Remove from Recent").on_click(move |_, _, cx| {
                app_settings::forget_recent(&forget);
                crate::workspace::refresh_menus(cx);
                target.update(cx, |_, cx| cx.notify()).ok();
            }))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_project_files_are_sized_in_kilobytes() {
        assert_eq!(file_size(0), "1 KB");
        assert_eq!(file_size(41_200), "42 KB");
        assert_eq!(file_size(3_400_000), "3.4 MB");
    }

    #[test]
    fn times_read_in_words() {
        assert_eq!(time_ago(1_000, 1_030), "just now");
        assert_eq!(time_ago(0, 60), "1 minute ago");
        assert_eq!(time_ago(0, 9 * 3_600 + 59), "9 hours ago");
        assert_eq!(time_ago(0, 2 * 86_400), "2 days ago");
        assert_eq!(time_ago(0, 3 * 604_800), "3 weeks ago");
        assert_eq!(time_ago(0, 40_000_000), "1 year ago");
        assert_eq!(time_ago(100, 50), "just now", "a clock that went back");
    }

    #[test]
    fn the_filter_matches_names_and_folders_of_files_still_there() {
        let folder = std::env::temp_dir().join(format!("develop-recent-test-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let (beach, city) = (folder.join("Beach.develop"), folder.join("City Lights.develop"));
        std::fs::write(&beach, "{}").unwrap();
        std::fs::write(&city, "{}").unwrap();
        let recent = [&beach, &city, &folder.join("Gone.develop")].map(|path| RecentProject { path: path.clone(), used: 0 });
        assert_eq!(shown_recent(&recent, "").len(), 2, "the missing one is left out");
        let found = shown_recent(&recent, "  lights ");
        assert_eq!(found.iter().map(|project| project.path.clone()).collect::<Vec<_>>(), [city]);
        assert_eq!(shown_recent(&recent, "develop-recent-test").len(), 2, "by folder too");
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
