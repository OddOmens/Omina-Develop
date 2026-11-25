//! Headless UI tests, as in Studio: the real editor window rendered offscreen and driven with
//! simulated clicks and keys. They check that controls are wired to what they promise, without
//! touching the real mouse or screen. Settings are read and written in a scratch folder per
//! test thread (see `app_settings::folder`), never the real settings file.

use gpui_kit::base::Root;
use gpui_kit::component::WindowExt as _;
use gpui_kit::test::TestWindowExt as _;
use gpui_kit::{AppContext as _, ElementId, Entity, MenuItem, ScrollDelta, TestAppContext, VisualTestContext, point, px, size};

use crate::features::Feature;
use crate::panels::onboarding::Step;
use crate::theme::{Accent, ThemeName};
use crate::workspace::Workspace;

/// Opens the editor in a tall offscreen window (so every choice on the setup's pages is on
/// screen without scrolling) and renders the first frames.
fn open(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    // Iris's shared folder for this test thread starts empty: no conversation, recipes or
    // presets left by an earlier test.
    let _ = std::fs::remove_dir_all(crate::panels::iris::tests_support::folder());
    cx.update(|cx| {
        gpui_kit::init(cx);
        crate::theme::apply(ThemeName::Obsidian, Accent::Lime, cx);
        crate::workspace::bind_keys(cx);
    });
    let mut workspace = None;
    let (_, visual) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| Workspace::new(window, cx));
        workspace = Some(view.clone());
        Root::new(view, window, cx)
    });
    visual.simulate_resize(size(px(1600.0), px(1500.0)));
    render(visual);
    (workspace.expect("the workspace was built"), visual)
}

fn render(visual: &mut VisualTestContext) {
    for _ in 0..3 {
        visual.update(|window, cx| window.render_frame(cx));
    }
}

fn click(visual: &mut VisualTestContext, id: impl Into<ElementId>) {
    let id = id.into();
    visual.update(|window, cx| window.click(id, cx));
    render(visual);
}

fn press(visual: &mut VisualTestContext, keys: &str) {
    visual.update(|window, cx| window.press(keys, cx));
    render(visual);
}

fn showing(visual: &mut VisualTestContext, id: impl Into<ElementId>) -> bool {
    let id = id.into();
    visual.update(|window, _| window.try_find(id).is_some())
}

/// The names of a menu's items, as the menu bar would list them now.
fn menu_names(menu: &str) -> Vec<String> {
    let menus = crate::workspace::menus();
    let found = menus.into_iter().find(|each| each.name.as_ref() == menu).expect("the menu exists");
    found
        .items
        .into_iter()
        .filter_map(|item| match item {
            MenuItem::Action { name, .. } => Some(name.to_string()),
            _ => None,
        })
        .collect()
}

#[gpui_kit::test]
fn the_welcome_setup_runs_on_first_launch_and_applies_each_choice(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let step = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).ui.onboarding);
    let settings = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).ui.settings.clone());
    // A first launch: the setup shows over the start screen.
    assert!(!crate::app_settings::load().onboarded);
    visual.update(|window, cx| workspace.update(cx, |ws, cx| ws.launch(Vec::new(), window, cx)));
    render(visual);
    assert_eq!(step(visual), Some(Step::Look));
    assert!(visual.update(|_, cx| workspace.read(cx).ui.start_screen.is_some()), "the start screen waits underneath");
    assert!(showing(visual, "onboarding-preview"));
    // Escape doesn't skip it by accident, and the editor's shortcuts wait too.
    press(visual, "escape");
    press(visual, "r");
    assert_eq!(step(visual), Some(Step::Look));
    assert_eq!(visual.update(|_, cx| workspace.read(cx).state.tool), crate::state::Tool::View);

    // 1. Look: Winter, teal. Each applies straight away.
    click(visual, ("theme", ThemeName::Winter as usize));
    click(visual, ("accent", Accent::Teal as usize));
    assert_eq!((settings(visual).theme, settings(visual).accent), (ThemeName::Winter, Accent::Teal));
    click(visual, "onboarding-next");
    assert_eq!(step(visual), Some(Step::Features));

    // 2. Tools & features: "Just the basics", then the masks back on by hand.
    click(visual, ("onboarding-focus", 3_usize));
    assert!(!settings(visual).feature_on(Feature::Masks) && !settings(visual).feature_on(Feature::WhiteBalanceSelector));
    assert!(settings(visual).feature_on(Feature::Crop));
    click(visual, ElementId::Name("onboarding-feature-masks".into()));
    assert!(settings(visual).feature_on(Feature::Masks));
    click(visual, "onboarding-next");

    // 3. Iris: on by default; No Iris hides her choices, and Use Iris brings them back.
    assert_eq!(step(visual), Some(Step::Iris));
    click(visual, "onboarding-iris-off");
    assert!(!settings(visual).iris_on);
    assert!(!showing(visual, ("onboarding-check", 0_usize)), "no choice of checks while she's off");
    click(visual, "onboarding-iris-on");
    click(visual, ("onboarding-check", 0_usize));
    let first = develop_core::assistant::Check::ALL[0];
    assert!(settings(visual).iris_on && !settings(visual).assistant.runs(first));
    // The side panel's steps go back too.
    click(visual, ("onboarding-step", 1_usize));
    assert_eq!(step(visual), Some(Step::Features), "a done step can be gone back to");
    click(visual, "onboarding-next");
    click(visual, "onboarding-back");
    assert_eq!(step(visual), Some(Step::Features), "Back goes back");
    click(visual, "onboarding-next");
    click(visual, "onboarding-iris-off");
    click(visual, "onboarding-next");

    // 4. Ready: a summary, then Start Editing closes it for good.
    assert_eq!(step(visual), Some(Step::Done));
    assert!(showing(visual, "onboarding-summary"));
    click(visual, "onboarding-next");
    assert_eq!(step(visual), None);
    let saved = crate::app_settings::load();
    assert!(saved.onboarded);
    assert_eq!((saved.theme, saved.accent), (ThemeName::Winter, Accent::Teal));
    assert!(!saved.feature_on(Feature::Snapshots) && saved.feature_on(Feature::Masks) && !saved.iris_on && !saved.assistant.runs(first), "every choice is remembered");

    // The editor behind follows: its buttons, menus and keys.
    for hidden in ["rail-iris", "rail-snapshots", "tool-white-balance", "tool-copy"] {
        assert!(!showing(visual, hidden), "{hidden} is hidden");
    }
    for kept in ["rail-presets", "rail-history", "tool-crop", "tool-linear"] {
        assert!(showing(visual, kept), "{kept} stays");
    }
    let tools = menu_names("Tools");
    assert!(tools.contains(&"Linear Gradient Mask".to_owned()) && !tools.contains(&"White Balance Selector".to_owned()), "{tools:?}");
    assert!(!menu_names("Settings").contains(&"New Snapshot".to_owned()));
    assert!(!menu_names("Help").contains(&"Ask Iris…".to_owned()));
    let tool = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).state.tool);
    press(visual, "w");
    assert_eq!(tool(visual), crate::state::Tool::View, "a turned-off tool's key does nothing");
    press(visual, "r");
    assert_eq!(tool(visual), crate::state::Tool::Crop, "the keys of tools still on work");
    press(visual, "r");

    // The next launch goes straight to the start screen.
    visual.update(|window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.ui.start_screen = None;
            ws.launch(Vec::new(), window, cx);
        })
    });
    render(visual);
    assert_eq!(step(visual), None);

    // Settings ▸ General can show it again; Skip Setup keeps what's there.
    press(visual, "cmd-,");
    click(visual, "general-welcome");
    assert_eq!(step(visual), Some(Step::Look));
    click(visual, "onboarding-skip");
    assert_eq!(step(visual), None);
    assert_eq!(settings(visual).theme, ThemeName::Winter);
}

#[gpui_kit::test]
fn turning_a_feature_off_in_settings_hides_its_card_and_leaves_its_edit(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    visual.update(|_, cx| {
        workspace.update(cx, |ws, cx| {
            ws.state.open(crate::state::test_support::photo(), None, "Import");
            ws.state.edit_develop("Tone Curve", |develop| develop.curve = vec![[0.0, 0.1], [1.0, 1.0]]);
            ws.state.compare = crate::state::Compare::Split;
            cx.notify();
        })
    });
    render(visual);
    assert!(showing(visual, "card-develop-curve"));
    assert!(showing(visual, "compare-split"));

    // Settings ▸ Tools & Features.
    press(visual, "cmd-,");
    click(visual, ("settings-section", 3_usize));
    click(visual, ElementId::Name("feature-tone-curve".into()));
    // Before / After is further down the page.
    visual.update(|window, cx| window.scroll("settings-content", ScrollDelta::Pixels(point(px(0.0), px(-800.0))), cx));
    render(visual);
    click(visual, ElementId::Name("feature-before-after".into()));
    click(visual, "settings-done");

    assert!(!showing(visual, "card-develop-curve"), "the Tone Curve card is hidden");
    assert!(!showing(visual, "compare-split") && !showing(visual, "tool-split"), "so are the comparison buttons");
    visual.update(|_, cx| {
        let ws = workspace.read(cx);
        assert_eq!(ws.state.compare, crate::state::Compare::Off, "the split showing is put away");
        assert_eq!(ws.state.edit_settings().unwrap().develop.curve, vec![[0.0, 0.1], [1.0, 1.0]], "the curve stays on the photo");
    });
    press(visual, "y");
    assert_eq!(visual.update(|_, cx| workspace.read(cx).state.compare), crate::state::Compare::Off, "its key does nothing");
    assert!(!crate::app_settings::load().feature_on(Feature::ToneCurve), "the choice is saved");
}

/// A small photo named `name` whose original is a real PNG file, so a project saved from it
/// opens again.
fn encoded_photo(name: &str) -> develop_core::Photo {
    let bitmap = develop_core::Bitmap::filled(12, 8, develop_core::Rgba::rgb(0.6, 0.5, 0.4));
    let bytes = develop_core::image_io::encode(&bitmap, develop_core::image_io::ExportFormat::Png, 100, [255; 3]).unwrap();
    develop_core::Photo::new(develop_core::Source::decode(format!("{name}.png"), bytes).unwrap())
}

#[gpui_kit::test]
fn save_as_writes_the_photo_it_was_asked_for_even_if_another_is_shown_meanwhile(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    let folder = std::env::temp_dir().join(format!("develop-save-as-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("First.develop");
    visual.update(|window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.state.open(encoded_photo("First"), None, "Import");
            ws.state.set_develop(develop_core::develop::DevelopSetting::Exposure, 1.0);
            ws.save_as(window, cx);
            // A photo that finished reading in the background is shown while the dialog is up.
            ws.state.open(encoded_photo("Second"), None, "Import");
        })
    });
    assert!(visual.did_prompt_for_new_path());
    let chosen = path.clone();
    visual.simulate_new_path_selection(move |_| Some(chosen));
    visual.run_until_parked();
    let saved = develop_core::file::load(&path).expect("the project was written");
    assert_eq!(saved.name, "First", "the photo Save As was chosen for is the one written");
    assert_eq!(saved.edit.develop.exposure, 1.0);
    visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        assert_eq!(state.sessions[0].file.as_deref(), Some(path.as_path()));
        assert!(!state.is_unsaved(0), "the first photo counts as saved");
        assert!(state.sessions[1].file.is_none(), "the second photo has no file yet");
    });
    std::fs::remove_dir_all(&folder).unwrap();
}

#[gpui_kit::test]
fn a_photo_opened_after_another_is_closed_gets_its_own_suggestions(cx: &mut TestAppContext) {
    use develop_core::assistant::Check;
    let (workspace, visual) = open(cx);
    // Opens a photo of one flat shade, lets the editor build its preview and Iris measure it,
    // and returns what Iris found.
    let look_at = |visual: &mut VisualTestContext, shade: f32| -> Vec<Check> {
        visual.update(|_, cx| {
            workspace.update(cx, |ws, cx| {
                let bitmap = develop_core::Bitmap::filled(240, 160, develop_core::Rgba::rgb(shade, shade, shade));
                ws.state.open(develop_core::Photo::new(develop_core::Source::new("Flat.png", Vec::new(), bitmap)), None, "Import");
                cx.notify();
            })
        });
        for _ in 0..4 {
            render(visual);
            visual.run_until_parked();
        }
        visual.update(|_, cx| crate::panels::iris::open_suggestions(workspace.read(cx)).into_iter().map(|suggestion| suggestion.check).collect())
    };
    let bright = look_at(visual, 0.97);
    assert!(bright.contains(&Check::Overexposed), "{bright:?}");
    // Closed with nothing to save, then a dark photo opened in its place.
    visual.update(|window, cx| workspace.update(cx, |ws, cx| ws.close_photo(0, window, cx)));
    render(visual);
    let dark = look_at(visual, 0.03);
    assert!(dark.contains(&Check::Underexposed) && !dark.contains(&Check::Overexposed), "the dark photo's own findings, not the closed one's: {dark:?}");
}

/// A scratch folder of the test's own for project files.
fn scratch_folder(name: &str) -> std::path::PathBuf {
    let thread = format!("{:?}", std::thread::current().id()).replace(|c: char| !c.is_ascii_alphanumeric(), "");
    let folder = std::env::temp_dir().join(format!("develop-{name}-{}-{thread}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    folder
}

/// Opens `photo` in a tab, on project `file` if given, and brightens it by `exposure`: an
/// unsaved change.
fn open_and_edit(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, photo: develop_core::Photo, file: Option<std::path::PathBuf>, exposure: f32) {
    visual.update(|_, cx| {
        workspace.update(cx, |ws, cx| {
            ws.state.open(photo, file, "Open");
            ws.state.set_develop(develop_core::develop::DevelopSetting::Exposure, exposure);
            cx.notify();
        })
    });
}

/// Runs the recovery timer's work once and lets the background copy finish.
fn keep_work_safe(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) {
    visual.update(|window, cx| workspace.update(cx, |ws, cx| ws.keep_work_safe(window, cx)));
    visual.run_until_parked();
}

/// The photos the recovery copy lists, by name, or `None` if there is no copy.
fn recovered_names() -> Option<Vec<String>> {
    let text = std::fs::read_to_string(crate::recovery::folder().join("index.json")).ok()?;
    let index: serde_json::Value = serde_json::from_str(&text).unwrap();
    Some(index["photos"].as_array().unwrap().iter().map(|photo| photo["name"].as_str().unwrap().to_owned()).collect())
}

/// As if Develop closed without saving and started again: a fresh editor, then a launch.
fn relaunch(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) {
    visual.update(|window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.state = crate::state::EditorState::new();
            ws.ui.recovery = Default::default();
            ws.ui.start_screen = None;
            ws.launch(Vec::new(), window, cx);
        })
    });
    visual.run_until_parked();
    render(visual);
}

/// Past the welcome setup, as on any launch after the first.
fn onboarded(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) {
    crate::app_settings::update(|settings| settings.onboarded = true);
    visual.update(|_, cx| workspace.update(cx, |ws, _| ws.ui.settings.onboarded = true));
}

#[gpui_kit::test]
fn unsaved_photos_are_recovered_after_develop_closes_without_saving(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    onboarded(&workspace, visual);
    let folder = scratch_folder("recovered");
    let dunes = folder.join("Dunes.develop");
    open_and_edit(&workspace, visual, encoded_photo("Dunes"), Some(dunes.clone()), 1.0);
    open_and_edit(&workspace, visual, encoded_photo("Harbor"), None, -0.5);
    // A third photo with nothing changed has nothing to recover.
    visual.update(|_, cx| workspace.update(cx, |ws, _| ws.state.open(encoded_photo("Untouched"), None, "Import")));
    keep_work_safe(&workspace, visual);
    assert_eq!(recovered_names(), Some(vec!["Dunes".to_owned(), "Harbor".to_owned()]), "each unsaved photo was copied");

    // Only a photo that changed is written again.
    let written = |name: &str| std::fs::metadata(crate::recovery::folder().join(name)).unwrap().modified().unwrap();
    let (dunes_before, harbor_before) = (written("photo-1.develop"), written("photo-2.develop"));
    visual.update(|_, cx| workspace.update(cx, |ws, _| ws.state.activate(1)));
    visual.update(|_, cx| workspace.update(cx, |ws, _| ws.state.set_develop(develop_core::develop::DevelopSetting::Exposure, -0.25)));
    keep_work_safe(&workspace, visual);
    assert_eq!(written("photo-1.develop"), dunes_before, "Dunes is unchanged, so its copy stays as it was");
    assert_ne!(written("photo-2.develop"), harbor_before, "Harbor changed");

    relaunch(&workspace, visual);
    assert!(visual.has_pending_prompt(), "it offers the photos back");
    // While it waits for an answer, the copy is left alone, by the timer and by quitting.
    keep_work_safe(&workspace, visual);
    visual.update(|_, cx| workspace.update(cx, |ws, _| ws.keep_recovery_copy_on_quit()));
    assert!(recovered_names().is_some(), "the copy survives until it is answered");

    visual.simulate_prompt_answer("Restore");
    visual.run_until_parked();
    render(visual);
    visual.update(|_, cx| {
        let ws = workspace.read(cx);
        let state = &ws.state;
        assert_eq!(state.sessions.len(), 2, "each photo is back in a tab");
        assert_eq!((state.sessions[0].photo.name.as_str(), state.sessions[1].photo.name.as_str()), ("Dunes", "Harbor"));
        assert_eq!(state.sessions[0].photo.edit.develop.exposure, 1.0);
        assert_eq!(state.sessions[1].photo.edit.develop.exposure, -0.25, "as last copied");
        assert_eq!(state.sessions[0].file.as_deref(), Some(dunes.as_path()), "still unsaved work on its project");
        assert!(state.sessions[1].file.is_none(), "untitled, as it was");
        assert!(state.is_unsaved(0) && state.is_unsaved(1), "restored work is still unsaved");
        assert!(ws.ui.start_screen.is_none(), "the editor shows them");
    });

    // Saving one takes it out of the copy; closing the other without saving removes the rest.
    visual.update(|window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.state.activate(0);
            ws.save(window, cx);
        })
    });
    visual.run_until_parked();
    assert!(dunes.exists());
    assert_eq!(recovered_names(), Some(vec!["Harbor".to_owned()]), "the saved photo has nothing left to recover");
    visual.update(|window, cx| workspace.update(cx, |ws, cx| ws.close_photo(1, window, cx)));
    visual.run_until_parked();
    visual.simulate_prompt_answer("Don’t Save");
    visual.run_until_parked();
    assert_eq!(recovered_names(), None, "work thrown away on purpose isn't kept");
    assert!(!crate::recovery::folder().exists());
    std::fs::remove_dir_all(&folder).ok();
}

#[gpui_kit::test]
fn the_recovery_copy_is_kept_at_quit_dropped_after_a_save_and_can_be_turned_off(cx: &mut TestAppContext) {
    let (workspace, visual) = open(cx);
    onboarded(&workspace, visual);
    let folder = scratch_folder("saved-while-copying");
    let file = folder.join("Meadow.develop");
    open_and_edit(&workspace, visual, encoded_photo("Meadow"), Some(file.clone()), 0.5);
    // The timer starts a copy in the background, and the photo is saved before it is done.
    visual.update(|window, cx| {
        workspace.update(cx, |ws, cx| {
            ws.keep_work_safe(window, cx);
            ws.save(window, cx);
        })
    });
    visual.run_until_parked();
    assert!(file.exists());
    assert_eq!(recovered_names(), None, "the older copy was dropped rather than put back");

    // The app quitting without asking about the changes (the system logging out, say).
    visual.update(|_, cx| workspace.update(cx, |ws, _| ws.state.set_develop(develop_core::develop::DevelopSetting::Exposure, 0.75)));
    visual.update(|_, cx| workspace.update(cx, |ws, _| ws.keep_recovery_copy_on_quit()));
    assert_eq!(recovered_names(), Some(vec!["Meadow".to_owned()]), "the work is copied on the way out");
    // Discarding it on the next launch removes it, and the launch carries on as usual.
    relaunch(&workspace, visual);
    visual.simulate_prompt_answer("Discard");
    visual.run_until_parked();
    render(visual);
    assert_eq!(recovered_names(), None);
    assert!(visual.update(|_, cx| workspace.read(cx).ui.start_screen.is_some()), "the start screen shows");
    visual.update(|_, cx| assert!(workspace.read(cx).state.sessions.is_empty()));

    // Settings ▸ General has the choice; Never keeps no copy, on the timer or at quit.
    press(visual, "cmd-,");
    assert!(showing(visual, "general-recovery"));
    let mut settings = crate::app_settings::load();
    settings.recovery_seconds = 0;
    visual.update(|_, cx| workspace.update(cx, |ws, cx| ws.use_settings(settings, cx)));
    open_and_edit(&workspace, visual, encoded_photo("Not Kept"), None, 1.0);
    keep_work_safe(&workspace, visual);
    visual.update(|_, cx| workspace.update(cx, |ws, _| ws.keep_recovery_copy_on_quit()));
    assert_eq!(recovered_names(), None, "no copy, as Settings asks");
    std::fs::remove_dir_all(&folder).ok();
}

/// A photo named `name` with color in it, 240 × 160, so edits and measurements have something
/// to work on.
fn colorful_photo(name: &str) -> develop_core::Photo {
    let mut bitmap = develop_core::Bitmap::new(240, 160);
    for x in 0..240 {
        for y in 0..160 {
            bitmap.set_pixel(x, y, [(40 + x / 2) as u8, (60 + y) as u8, 150, 255]);
        }
    }
    develop_core::Photo::new(develop_core::Source::new(format!("{name}.jpg"), Vec::new(), bitmap))
}

/// Opens the editor with photos of these names in tabs, the first of them showing, past the
/// welcome setup.
fn open_with_photos<'a>(cx: &'a mut TestAppContext, names: &[&str]) -> (Entity<Workspace>, &'a mut VisualTestContext) {
    let (workspace, visual) = open(cx);
    onboarded(&workspace, visual);
    // Recipes taught by an earlier test on this thread aren't this test's.
    let _ = std::fs::remove_dir_all(crate::panels::iris::tests_support::folder());
    visual.update(|_, cx| {
        workspace.update(cx, |ws, cx| {
            for name in names {
                ws.state.open(colorful_photo(name), None, "Import");
            }
            ws.state.activate(0);
            cx.notify();
        })
    });
    render(visual);
    (workspace, visual)
}

/// Types `message` to Iris and returns what she said back, her bubbles joined by a blank line.
fn tell(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, message: &str) -> String {
    visual.update(|window, cx| workspace.update(cx, |ws, cx| crate::panels::iris::send(ws, message, window, cx)));
    visual.run_until_parked();
    render(visual);
    visual.update(|_, cx| crate::panels::iris::tests_support::answer(workspace.read(cx)).join("\n\n"))
}

/// The Develop settings of open photo `index`.
fn develop_of(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, index: usize) -> develop_core::develop::Develop {
    visual.update(|_, cx| workspace.read(cx).state.sessions[index].photo.edit.develop.clone())
}

/// The names of open photo `index`'s history steps.
fn steps_of(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, index: usize) -> Vec<String> {
    visual.update(|_, cx| workspace.read(cx).state.sessions[index].history.states().0)
}

fn offered(workspace: &Entity<Workspace>, visual: &mut VisualTestContext) -> Vec<String> {
    visual.update(|_, cx| crate::panels::iris::tests_support::replies(workspace.read(cx)))
}

#[gpui_kit::test]
fn iris_makes_listed_edits_as_one_step_and_follow_ups_build_on_it(cx: &mut TestAppContext) {
    let (workspace, visual) = open_with_photos(cx, &["Beach"]);
    let said = tell(&workspace, visual, "shadows +30, highlights -20 and a bit warmer");
    assert!(said.starts_with("3 changes") && said.contains("Shadows 30") && said.contains("Highlights -20") && said.contains("Warmer"), "{said}");
    let develop = develop_of(&workspace, visual, 0);
    assert_eq!((develop.shadows, develop.highlights, develop.temperature), (30.0, -20.0, 8.0));
    assert_eq!(steps_of(&workspace, visual, 0).len(), 2, "all three are one step in History");
    assert_eq!(offered(&workspace, visual), vec!["Undo that", "More", "Less"]);

    // "More" makes the same moves again from where the photo is now.
    let said = tell(&workspace, visual, "more");
    assert!(said.starts_with("More: Shadows +60"), "{said}");
    let develop = develop_of(&workspace, visual, 0);
    assert_eq!((develop.shadows, develop.highlights, develop.temperature), (60.0, -40.0, 16.0));
    // "Too much" takes half of that back, in place of the step it softens.
    let said = tell(&workspace, visual, "too much");
    assert!(said.starts_with("Less: Shadows +45"), "{said}");
    let develop = develop_of(&workspace, visual, 0);
    assert_eq!((develop.shadows, develop.highlights, develop.temperature), (45.0, -30.0, 12.0));
    assert_eq!(steps_of(&workspace, visual, 0).len(), 3, "the softer change replaced the one it softened");
    // And again halves what is left of it.
    tell(&workspace, visual, "less");
    assert_eq!(develop_of(&workspace, visual, 0).shadows, 38.0);

    // "Undo that" says what it undid; redo puts it back.
    let said = tell(&workspace, visual, "undo that");
    assert!(said.starts_with("Undone: Less: "), "{said}");
    assert_eq!(develop_of(&workspace, visual, 0).shadows, 30.0);
    assert_eq!(offered(&workspace, visual), vec!["Redo"]);
    let said = tell(&workspace, visual, "redo");
    assert!(said.starts_with("Redone: Less: "), "{said}");
    assert_eq!(develop_of(&workspace, visual, 0).shadows, 38.0);

    // "Again" repeats the request that last changed something, not the follow-ups after it.
    tell(&workspace, visual, "again");
    assert_eq!(develop_of(&workspace, visual, 0).shadows, 68.0);
    // With nothing left to build on, a follow-up says so and changes nothing.
    tell(&workspace, visual, "rotate left");
    let said = tell(&workspace, visual, "more");
    assert!(said.contains("follow a slider change"), "{said}");
    // A change that leaves everything where it is isn't claimed as one.
    tell(&workspace, visual, "contrast 100");
    let said = tell(&workspace, visual, "contrast 100");
    assert!(said.contains("already is"), "{said}");
}

#[gpui_kit::test]
fn iris_carries_out_steps_in_order_and_stops_at_one_that_fails(cx: &mut TestAppContext) {
    let (workspace, visual) = open_with_photos(cx, &["Beach"]);
    let said = tell(&workspace, visual, "a bit brighter, then frobnicate the widget, then warmer; rotate left");
    assert!(said.contains("Brighter: Exposure +0.15") && said.contains("I didn't follow that"), "{said}");
    assert!(said.contains("Not run:\n• warmer\n• rotate left"), "{said}");
    let develop = develop_of(&workspace, visual, 0);
    assert_eq!((develop.exposure, develop.temperature), (0.15, 0.0), "the steps after the failed one were not run");
    assert!(offered(&workspace, visual).iter().all(|reply| reply != "Undo that"), "the last word was about what wasn't run");

    // Requests of different kinds joined by "and" are steps too, each answered.
    let said = tell(&workspace, visual, "crop to 16:9 and rotate left");
    assert!(said.contains("Cropped to 16 × 9") && said.contains("Turned a quarter turn left"), "{said}");
    // A list followed only in part changes nothing and offers the part that was followed.
    let before = develop_of(&workspace, visual, 0);
    let said = tell(&workspace, visual, "shadows +30 and frobnicate the widget");
    assert!(said.contains("I followed “shadows +30” but not “frobnicate the widget”"), "{said}");
    assert_eq!(develop_of(&workspace, visual, 0), before);
    assert_eq!(offered(&workspace, visual), vec!["shadows +30"]);

    // A mistyped word is put right, and the reply says how it was read.
    let said = tell(&workspace, visual, "make it brigther");
    assert!(said.starts_with("(I read “brigther” as “brighter”.)\nBrighter: Exposure +0.45"), "{said}");
    // Sums are answered without touching the photo, and nonsense offers the nearest help.
    assert_eq!(tell(&workspace, visual, "1920 / 3"), "1920 / 3 = 640");
    // A request she can't carry out is said to be not followed. With no help anywhere near
    // it, she offers what she can do rather than an unrelated topic.
    let said = tell(&workspace, visual, "make it purple");
    assert!(said.starts_with("I didn't follow that"), "{said}");
    assert_eq!(offered(&workspace, visual), vec!["What can you do?", "What's wrong?"]);
    // When help is near, it is on offer rather than given as if it were the answer.
    let said = tell(&workspace, visual, "make the shortcut purple");
    assert!(said.starts_with("I didn't follow that"), "{said}");
    assert!(offered(&workspace, visual).iter().all(|reply| reply.starts_with("Help: ")) && !offered(&workspace, visual).is_empty());
    let topic = offered(&workspace, visual).remove(0);
    let said = tell(&workspace, visual, &topic);
    assert!(said.starts_with(topic.trim_start_matches("Help: ")), "a click on the offer answers it: {said}");
    // A topic named on its own is answered from the help.
    assert!(tell(&workspace, visual, "tone curve").starts_with("Shape tones with the tone curve"));
    // Layers belong to the other apps, and Iris says so.
    assert!(tell(&workspace, visual, "duplicate the layer").contains("has no layers"));
}

#[gpui_kit::test]
fn iris_runs_recipes_that_are_built_in_and_taught(cx: &mut TestAppContext) {
    let (workspace, visual) = open_with_photos(cx, &["Beach"]);
    // Every built-in recipe is made only of messages Iris follows, start to finish.
    for recipe in crate::panels::iris::tests_support::built_in_recipes() {
        let before = visual.update(|_, cx| workspace.read(cx).state.sessions[0].photo.edit.clone());
        // A recipe that takes a value is run with one.
        let run = if recipe.slots().is_empty() { recipe.name.clone() } else { format!("{} Warm and Soft", recipe.called()) };
        let said = tell(&workspace, visual, &run);
        assert!(!said.contains("didn't follow") && !said.contains("Not run") && !said.contains("turned off"), "{}: {said}", recipe.name);
        // "Web export" exports (here, with nowhere known to put the file yet, through the
        // sheet); the others each change the photo.
        if recipe.name == "web export" || recipe.name.starts_with("proof") {
            assert!(said.contains("JPEG, quality") && said.contains("I don't know where your exports go yet"), "{said}");
            assert!(!recipe.name.starts_with("proof") || said.contains("“Warm and Soft”"), "the value arrives whole: {said}");
            assert!(visual.update(|window, cx| window.has_active_dialog(cx)), "Export is open");
            press(visual, "escape");
        } else {
            assert_ne!(visual.update(|_, cx| workspace.read(cx).state.sessions[0].photo.edit.clone()), before, "{} changed nothing: {said}", recipe.name);
        }
        tell(&workspace, visual, "reset everything");
    }
    // A recipe is run by its command too, with dashes for its spaces.
    tell(&workspace, visual, "/mono-finish");
    assert_eq!(develop_of(&workspace, visual, 0).saturation, -100.0);
    tell(&workspace, visual, "reset");

    // Teaching one keeps it for next time, and it then runs by name.
    let said = tell(&workspace, visual, "teach glow: a bit warmer, then add a subtle vignette");
    assert!(said.starts_with("Got it. “glow” now means:"), "{said}");
    let folder = crate::panels::iris::tests_support::folder();
    assert_eq!(::iris::recipes::load(&folder, ::iris::App::Develop).len(), 1, "saved beside her shared settings");
    tell(&workspace, visual, "glow");
    let develop = develop_of(&workspace, visual, 0);
    assert_eq!((develop.temperature, develop.vignette), (8.0, -13.0));
    // The last change can be saved as one, and listed.
    tell(&workspace, visual, "sharpen");
    tell(&workspace, visual, "save that as crisp");
    let listed = tell(&workspace, visual, "/recipes");
    assert!(listed.contains("glow") && listed.contains("crisp: sharpen") && listed.contains("portrait finish"), "{listed}");
    tell(&workspace, visual, "forget glow");
    assert_eq!(visual.update(|_, cx| crate::panels::iris::tests_support::taught(workspace.read(cx))), vec!["crisp"]);
    assert!(tell(&workspace, visual, "teach check: sharpen").contains("already one of my commands"));
    let _ = std::fs::remove_dir_all(folder);
}

#[gpui_kit::test]
fn iris_changes_several_photos_only_after_a_yes_and_takes_it_back_on_all(cx: &mut TestAppContext) {
    let (workspace, visual) = open_with_photos(cx, &["Beach", "Dunes", "Harbor"]);
    tell(&workspace, visual, "contrast 30, temperature 12");
    visual.update(|_, cx| workspace.update(cx, |ws, _| ws.state.sessions[2].photo.edit.develop.exposure = 1.0));

    // Syncing says what it will do and to how many, and waits.
    let said = tell(&workspace, visual, "sync to the others");
    assert!(said.contains("onto the 2 other photos open: Dunes, Harbor") && said.contains("Go ahead?"), "{said}");
    assert_eq!(offered(&workspace, visual), vec!["Yes, sync them", "No, leave them"]);
    assert_eq!(develop_of(&workspace, visual, 1).contrast, 0.0, "nothing is changed before the yes");
    // A no leaves them alone.
    assert_eq!(tell(&workspace, visual, "No, leave them"), "Left as they are.");
    assert_eq!(develop_of(&workspace, visual, 1).contrast, 0.0);
    assert!(tell(&workspace, visual, "yes").contains("I didn't follow"), "a yes with nothing waiting is not an answer to anything");

    tell(&workspace, visual, "sync to the others");
    let said = tell(&workspace, visual, "Yes, sync them");
    assert!(said.starts_with("Done on 2 photos: Dunes, Harbor."), "{said}");
    for index in [1, 2] {
        let develop = develop_of(&workspace, visual, index);
        assert_eq!((develop.contrast, develop.temperature, develop.exposure), (30.0, 12.0, 0.0), "photo {index} has the first photo's settings");
        assert_eq!(steps_of(&workspace, visual, index).last().map(String::as_str), Some("Iris: Sync Settings"));
    }
    // A change made by hand to one of them since means that one is left alone by the undo.
    visual.update(|_, cx| workspace.update(cx, |ws, _| ws.state.sessions[2].photo.edit.develop.clarity = 5.0));
    let said = tell(&workspace, visual, "undo that");
    assert!(said.starts_with("Undone on 1 photo: Sync Settings.") && said.contains("Harbor changed since"), "{said}");
    assert_eq!(develop_of(&workspace, visual, 1).contrast, 0.0);
    assert_eq!(develop_of(&workspace, visual, 2).contrast, 30.0);
    assert_eq!(develop_of(&workspace, visual, 0).contrast, 30.0, "the photo it was copied from is untouched");

    // Only some parts, by word.
    tell(&workspace, visual, "sync only the white balance");
    tell(&workspace, visual, "yes");
    let develop = develop_of(&workspace, visual, 1);
    assert_eq!((develop.temperature, develop.contrast), (12.0, 0.0));

    // "Same for all the others" reads the last request afresh for each photo.
    tell(&workspace, visual, "a bit brighter");
    assert_eq!(develop_of(&workspace, visual, 0).exposure, 0.15);
    let said = tell(&workspace, visual, "same for all the others");
    assert!(said.contains("“a bit brighter” again on the 2 other photos"), "{said}");
    tell(&workspace, visual, "ok");
    assert_eq!(develop_of(&workspace, visual, 1).exposure, 0.15);
    assert_eq!(develop_of(&workspace, visual, 2).exposure, 0.15);
    // "Same for the next one" shows it and does it there.
    tell(&workspace, visual, "contrast +10");
    let said = tell(&workspace, visual, "same for the next one");
    assert!(said.starts_with("Showing Dunes (photo 2 of 3)."), "{said}");
    assert_eq!(visual.update(|_, cx| workspace.read(cx).state.active), 1);
    assert_eq!(develop_of(&workspace, visual, 1).contrast, 10.0);

    // Auto and a reset for every photo ask first as well.
    assert!(tell(&workspace, visual, "reset all photos").contains("on all 3 photos"));
    tell(&workspace, visual, "yes");
    for index in 0..3 {
        assert!(develop_of(&workspace, visual, index).is_neutral(), "photo {index} is reset");
    }
    tell(&workspace, visual, "auto all photos");
    let said = tell(&workspace, visual, "yes");
    assert!(said.starts_with("Done on 3 photos"), "{said}");
    assert!(!develop_of(&workspace, visual, 2).is_neutral(), "a photo that was never shown is set from its own pixels");
    // Moving between them by word.
    assert!(tell(&workspace, visual, "next photo").starts_with("Showing Harbor (photo 3 of 3)."));
    assert!(tell(&workspace, visual, "go to Beach").starts_with("Showing Beach"));
}

#[gpui_kit::test]
fn iris_runs_menu_commands_by_name_and_keeps_to_the_features_that_are_on(cx: &mut TestAppContext) {
    let (workspace, visual) = open_with_photos(cx, &["Beach"]);
    // A menu command asked for by name is run, and said with its key.
    let said = tell(&workspace, visual, "show clipping");
    assert!(said.starts_with("The clipping warning is on") && said.ends_with("(J)."), "{said}");
    assert!(visual.update(|_, cx| workspace.read(cx).ui.show_clipping));
    tell(&workspace, visual, "hide clipping");
    assert!(!visual.update(|_, cx| workspace.read(cx).ui.show_clipping));
    assert_eq!(tell(&workspace, visual, "white balance selector"), "White Balance Selector (W): done.");
    assert_eq!(visual.update(|_, cx| workspace.read(cx).state.tool), crate::state::Tool::WhiteBalance);
    assert_eq!(tell(&workspace, visual, "show history"), "History is showing beside the rail.");
    // Commands that fit equally are offered rather than picked between.
    assert_eq!(tell(&workspace, visual, "zoom"), "Which do you mean: Zoom In, Zoom Out?");
    assert_eq!(offered(&workspace, visual), vec!["Zoom In", "Zoom Out"]);
    assert_eq!(tell(&workspace, visual, "Zoom In"), "Zoom In (⌘=): done.");
    // Opening a dialog ends the message there.
    let said = tell(&workspace, visual, "open settings then brighter");
    assert!(said.contains("Opening Settings.") && said.contains("Not run:\n• brighter"), "{said}");
    assert!(visual.update(|window, cx| window.has_active_dialog(cx)));
    click(visual, "settings-done");

    // With a feature turned off, what belongs to it isn't done, and Iris says where to turn it on.
    let mut settings = crate::app_settings::load();
    for feature in [Feature::Crop, Feature::ColorMixer, Feature::Effects, Feature::Clipping, Feature::Snapshots] {
        settings.set_feature(feature, false);
    }
    visual.update(|_, cx| workspace.update(cx, |ws, cx| ws.use_settings(settings, cx)));
    assert!(tell(&workspace, visual, "crop to 16:9").starts_with("Crop & Straighten is turned off"));
    assert!(tell(&workspace, visual, "make the sky bluer").starts_with("Color Mixer is turned off"));
    assert!(tell(&workspace, visual, "save a snapshot").starts_with("Snapshots is turned off"));
    assert!(!tell(&workspace, visual, "show clipping").contains("clipping warning is"), "its command isn't on offer");
    visual.update(|_, cx| {
        let ws = workspace.read(cx);
        let edit = ws.state.edit_settings().unwrap();
        assert!(edit.crop.is_neutral() && edit.develop.is_neutral() && !ws.ui.show_clipping && ws.state.photo().unwrap().snapshots.is_empty());
    });
    // A look keeps the parts that are on and names what it left out.
    let said = tell(&workspace, visual, "make it moody");
    assert!(said.contains("I left out the parts that belong to Effects") && !said.contains("Vignette"), "{said}");
    let develop = develop_of(&workspace, visual, 0);
    assert_eq!((develop.contrast, develop.vignette), (15.0, 0.0));
}

#[gpui_kit::test]
fn iris_crops_exports_and_keeps_looks_by_word(cx: &mut TestAppContext) {
    let (workspace, visual) = open_with_photos(cx, &["Beach"]); // 240 × 160
    let said = tell(&workspace, visual, "crop to 4 by 5");
    assert!(said.starts_with("Cropped to 4 × 5, upright: 128 × 160 pixels."), "{said}");
    assert!(tell(&workspace, visual, "swap the crop").starts_with("Turned the crop, on its side: 160 × 128 pixels."));
    assert!(tell(&workspace, visual, "straighten 2 degrees").starts_with("Straighten angle +2.00°"));
    tell(&workspace, visual, "reset the crop");
    assert!(visual.update(|_, cx| workspace.read(cx).state.edit_settings().unwrap().crop.is_neutral()));
    // This photo has no line to level by, so "straighten" says so and opens the tool.
    assert!(tell(&workspace, visual, "straighten it").contains("couldn't find a horizon or upright edge clear enough"));
    assert_eq!(visual.update(|_, cx| workspace.read(cx).state.tool), crate::state::Tool::Crop);
    assert!(visual.update(|_, cx| workspace.read(cx).state.edit_settings().unwrap().crop.is_neutral()), "nothing was guessed");

    // With nowhere known to put the file yet, Export opens with the choices made, sized by
    // the side that was named.
    let said = tell(&workspace, visual, "export as JPEG at 80% quality, 120 pixels wide");
    assert!(said.contains("with JPEG, quality 80, 120 × 80 pixels") && said.contains("I don't know where your exports go yet"), "{said}");
    assert!(visual.update(|window, cx| window.has_active_dialog(cx)));
    assert!(showing(visual, ElementId::Name("export-quality-80".into())) && showing(visual, ElementId::Name("export-size-120 px".into())), "the sheet shows the choices");
    assert_eq!(crate::app_settings::load().export, crate::app_settings::ExportChoices::default(), "nothing is remembered until Export… is pressed");
    press(visual, "escape");
    let said = tell(&workspace, visual, "export as png, quality 50, 4000 px");
    assert!(said.contains("PNG has no quality setting") && said.contains("never enlarges"), "{said}");
    press(visual, "escape");

    // Snapshots and presets by name.
    tell(&workspace, visual, "exposure +0.5");
    assert!(tell(&workspace, visual, "save a snapshot called Before the look").contains("“Before the look”"));
    tell(&workspace, visual, "make it moody");
    let different = tell(&workspace, visual, "what's different from the last snapshot?");
    assert!(different.contains("Exposure: 0.50 → 0.25") && different.contains("Vignette: 0 → -20"), "{different}");
    tell(&workspace, visual, "go back to the snapshot Before the look");
    let develop = develop_of(&workspace, visual, 0);
    assert_eq!((develop.exposure, develop.vignette), (0.5, 0.0));
    assert!(tell(&workspace, visual, "save this as a preset called Golden").starts_with("Saved the preset “Golden”"));
    assert!(crate::app_settings::load().user_presets.iter().any(|preset| preset.name == "Golden" && preset.settings.exposure == 0.5));
    assert!(tell(&workspace, visual, "save this as a preset called Vivid").contains("pick another name"));
    tell(&workspace, visual, "reset");
    assert!(tell(&workspace, visual, "try Golden").starts_with("Applied Golden."));
    assert_eq!(develop_of(&workspace, visual, 0).exposure, 0.5);
    // Copy and paste, which says so when nothing is copied.
    assert!(tell(&workspace, visual, "paste settings").starts_with("There are no copied settings yet"));
    assert!(tell(&workspace, visual, "copy these settings").starts_with("Copied this photo's Develop settings"));
    tell(&workspace, visual, "reset");
    tell(&workspace, visual, "paste settings");
    assert_eq!(develop_of(&workspace, visual, 0).exposure, 0.5);
    // The help lists what every app's Iris understands as well.
    let help = tell(&workspace, visual, "/help");
    assert!(help.contains("/ready") && help.contains("In every app I also understand"), "{help}");
}

#[gpui_kit::test]
fn iris_answers_what_she_can_without_a_photo(cx: &mut TestAppContext) {
    let (workspace, visual) = open_with_photos(cx, &[]);
    assert!(tell(&workspace, visual, "a bit brighter").starts_with("Open a photo and I'll look it over."));
    assert_eq!(tell(&workspace, visual, "15% of 240"), "15% of 240 = 36");
    assert!(tell(&workspace, visual, "how do I crop a photo?").starts_with("Crop and straighten a photo"));
    assert!(tell(&workspace, visual, "/recipes").contains("portrait finish"));
    assert_eq!(tell(&workspace, visual, "zoom in"), "Zoom In needs a photo. Open one first (⌘O).");
    assert!(tell(&workspace, visual, "undo that").contains("nothing to undo"));
    assert!(tell(&workspace, visual, "/frobnicate").starts_with("Open a photo"));
}

/// A 300 × 200 photo of sky over darker sea, its horizon tilted `tilt` degrees clockwise.
fn seascape(name: &str, tilt: f32) -> develop_core::Photo {
    let (width, height) = (300, 200);
    let (sin, cos) = tilt.to_radians().sin_cos();
    let mut bitmap = develop_core::Bitmap::new(width, height);
    for y in 0..height as i32 {
        for x in 0..width as i32 {
            let (across, down) = (x as f32 - width as f32 * 0.5, y as f32 - height as f32 * 0.5);
            let below = ((down * cos - across * sin) / 1.5).clamp(-1.0, 1.0) * 0.5 + 0.5;
            let value = (200.0 - 120.0 * below) as u8;
            bitmap.set_pixel(x, y, [value, value, value.saturating_add(20), 255]);
        }
    }
    develop_core::Photo::new(develop_core::Source::new(format!("{name}.jpg"), Vec::new(), bitmap))
}

/// The edit of open photo `index`.
fn edit_of(workspace: &Entity<Workspace>, visual: &mut VisualTestContext, index: usize) -> develop_core::Edit {
    visual.update(|_, cx| workspace.read(cx).state.sessions[index].photo.edit.clone())
}

#[gpui_kit::test]
fn iris_reads_requests_of_different_kinds_joined_by_and(cx: &mut TestAppContext) {
    let (workspace, visual) = open_with_photos(cx, &["Beach", "Dunes"]);
    // An edit and a crop: both are made, each by its own reader, as one step in History.
    let said = tell(&workspace, visual, "warmer and crop to square");
    assert!(said.contains("Warmer: Temperature +15") && said.contains("Cropped to 1 × 1"), "{said}");
    let edit = edit_of(&workspace, visual, 0);
    assert!(edit.develop.temperature == 15.0 && edit.crop.rect.is_some());
    assert_eq!(steps_of(&workspace, visual, 0).len(), 2, "one message, one Undo");
    // An edit and a snapshot whose name is kept whole, "and" and all.
    let said = tell(&workspace, visual, "sharpen and save a snapshot called Sharp and Warm");
    assert!(said.contains("Sharper: Sharpening 30") && said.contains("“Sharp and Warm”"), "{said}");
    visual.update(|_, cx| {
        let photo = &workspace.read(cx).state.sessions[0].photo;
        assert_eq!(photo.edit.develop.sharpen_amount, 30.0);
        assert_eq!(photo.snapshots.iter().map(|snapshot| snapshot.name.as_str()).collect::<Vec<_>>(), vec!["Sharp and Warm"]);
        assert_eq!(photo.snapshots[0].edit.develop.sharpen_amount, 30.0, "the snapshot was taken after the sharpening");
    });
    // Two edits, then something for every photo: the edits are made, and the rest is asked about.
    let said = tell(&workspace, visual, "make it moody and add a vignette and sync to the others");
    assert!(said.contains("Moody look") && said.contains("Vignette -45") && said.contains("Go ahead?"), "{said}");
    assert_eq!(develop_of(&workspace, visual, 0).vignette, -45.0);
    assert_eq!(develop_of(&workspace, visual, 1).vignette, 0.0, "nothing reaches the other photo before the yes");
    tell(&workspace, visual, "yes");
    assert_eq!(develop_of(&workspace, visual, 1).vignette, -45.0);
    // A menu command joined to an edit, and an edit joined to a recipe.
    let said = tell(&workspace, visual, "show clipping and a bit brighter");
    assert!(said.contains("The clipping warning is on") && said.contains("Brighter"), "{said}");
    let said = tell(&workspace, visual, "reset everything and mono finish");
    assert!(said.contains("Everything is back as it came") && said.contains("Dramatic Black & White"), "{said}");
    // A list one reader takes whole is still one change that says each part.
    assert!(tell(&workspace, visual, "shadows +30, highlights -20 and a bit warmer").starts_with("3 changes"));
    // A step that fails stops the rest, which are named.
    let said = tell(&workspace, visual, "paste settings and rotate left and brighter");
    assert!(said.contains("no copied settings yet") && said.contains("Not run:\n• rotate left\n• brighter"), "{said}");
}

#[gpui_kit::test]
fn iris_gives_vague_requests_a_stated_default_that_can_be_tuned(cx: &mut TestAppContext) {
    let (workspace, visual) = open_with_photos(cx, &["Beach"]); // 240 × 160
    // "Make it look good" is Auto for the light and whatever the rules would fix, each named.
    let said = tell(&workspace, visual, "make it look good");
    assert!(said.starts_with("Here's what I did:\n• Auto for the light: ") && said.contains("One Undo takes it all back."), "{said}");
    assert_ne!(develop_of(&workspace, visual, 0).exposure, 0.0);
    assert_eq!(steps_of(&workspace, visual, 0), vec!["Import", "Iris: Make It Look Good"]);
    // With the light set by hand it is left alone, and the reply says so.
    tell(&workspace, visual, "undo that");
    tell(&workspace, visual, "exposure +0.2");
    let said = tell(&workspace, visual, "improve this");
    assert!(!said.contains("Auto for the light"), "{said}");
    assert_eq!(develop_of(&workspace, visual, 0).exposure, 0.2);
    tell(&workspace, visual, "reset everything");

    // "Make it pop" says what it takes that to mean, and "more" and "less" go on from there.
    let said = tell(&workspace, visual, "make it pop");
    assert_eq!(said, "Pop, which I take as more contrast, clarity and color: Contrast +15, Clarity +10, Vibrance +15.");
    assert_eq!(offered(&workspace, visual), vec!["Undo that", "More", "Less"]);
    tell(&workspace, visual, "more");
    assert_eq!(develop_of(&workspace, visual, 0).contrast, 30.0);
    tell(&workspace, visual, "less");
    assert_eq!(develop_of(&workspace, visual, 0).contrast, 23.0);
    assert!(tell(&workspace, visual, "fix the light").starts_with("Auto set the light from the photo: "));
    tell(&workspace, visual, "reset everything");

    // The crop tightens by a stated share about its middle; "more" goes on, "less" halves it.
    let steps_before = steps_of(&workspace, visual, 0).len();
    let said = tell(&workspace, visual, "crop it tighter");
    assert!(said.starts_with("Cropped in by 10% each way, same shape and middle, on its side: 216 × 144 pixels."), "{said}");
    assert!(tell(&workspace, visual, "more").contains("194 × 130 pixels"));
    let said = tell(&workspace, visual, "too much");
    assert!(said.starts_with("Cropped in by 5% each way") && said.contains("205 × 137 pixels"), "{said}");
    assert_eq!(steps_of(&workspace, visual, 0).len(), steps_before + 2, "the gentler crop replaced the one it eased");
    let rect = edit_of(&workspace, visual, 0).crop.rect.unwrap();
    assert!((rect.center().x - 120.0).abs() < 0.5 && (rect.center().y - 80.0).abs() < 0.5 && (rect.width / rect.height - 1.5).abs() < 0.01, "{rect:?}");

    // Not saying what to change gets a question with choices.
    let said = tell(&workspace, visual, "change it");
    assert!(said.starts_with("What would you like changed?"), "{said}");
    assert!(offered(&workspace, visual).contains(&"Make it look good".to_owned()));
}

#[gpui_kit::test]
fn level_straightens_by_the_photos_own_horizon_from_iris_the_crop_bar_and_the_menu(cx: &mut TestAppContext) {
    let (workspace, visual) = open_with_photos(cx, &[]);
    visual.update(|_, cx| {
        workspace.update(cx, |ws, cx| {
            ws.state.open(seascape("Tilted", 3.0), None, "Import");
            ws.state.open(seascape("Other way", -4.0), None, "Import");
            ws.state.open(colorful_photo("No line"), None, "Import");
            ws.state.activate(0);
            cx.notify();
        })
    });
    render(visual);
    let angle = |visual: &mut VisualTestContext, index: usize| edit_of(&workspace, visual, index).crop.angle;
    // Iris measures the tilt and takes it out.
    let said = tell(&workspace, visual, "straighten it");
    assert!(said.starts_with("Leveled: straighten angle -"), "{said}");
    assert!((angle(visual, 0) + 3.0).abs() < 0.34, "{}", angle(visual, 0));
    assert_eq!(steps_of(&workspace, visual, 0), vec!["Import", "Level"]);
    assert!(tell(&workspace, visual, "level it").starts_with("It already looks level"));
    // The same for the others: each by its own horizon, and none guessed where there is no line.
    tell(&workspace, visual, "same for all the others");
    let said = tell(&workspace, visual, "yes");
    assert!(said.starts_with("Done on 1 photo: Other way.") && said.contains("No line (no clear line to level by)"), "{said}");
    assert!((angle(visual, 1) - 4.0).abs() < 0.34, "{}", angle(visual, 1));
    assert_eq!(angle(visual, 2), 0.0);

    // The Photo menu has it, and the crop bar's Level button does the same.
    assert!(menu_names("Photo").contains(&"Level".to_owned()));
    tell(&workspace, visual, "reset the crop");
    press(visual, "r");
    click(visual, "crop-level");
    visual.run_until_parked();
    render(visual);
    assert!((angle(visual, 0) + 3.0).abs() < 0.34, "{}", angle(visual, 0));
    // With Crop & Straighten turned off, Level goes with it.
    let mut settings = crate::app_settings::load();
    settings.set_feature(Feature::Crop, false);
    // The menu bar reads the saved settings, as it does when Settings changes them.
    crate::app_settings::save(&settings);
    visual.update(|_, cx| workspace.update(cx, |ws, cx| ws.use_settings(settings, cx)));
    assert!(!menu_names("Photo").contains(&"Level".to_owned()));
    assert!(tell(&workspace, visual, "straighten it").starts_with("Crop & Straighten is turned off"));
}

#[gpui_kit::test]
fn syncing_carries_the_crop_to_each_photo_in_proportion(cx: &mut TestAppContext) {
    let (workspace, visual) = open_with_photos(cx, &["Beach", "Dunes"]); // each 240 × 160
    // A third photo that stands upright.
    visual.update(|_, cx| {
        workspace.update(cx, |ws, cx| {
            ws.state.open(develop_core::Photo::new(develop_core::Source::new("Tower.jpg", Vec::new(), develop_core::Bitmap::filled(160, 240, develop_core::Rgba::rgb(0.5, 0.5, 0.5)))), None, "Import");
            ws.state.activate(0);
            cx.notify();
        })
    });
    // A photo that isn't cropped has no framing to give, so the others keep theirs.
    let said = tell(&workspace, visual, "sync to the others");
    assert!(said.contains("everything but Crop") && said.contains("isn't cropped or straightened, so each keeps its own crop"), "{said}");
    tell(&workspace, visual, "no");
    tell(&workspace, visual, "crop to square and straighten 2 degrees and contrast 20");
    let from = edit_of(&workspace, visual, 0);
    let said = tell(&workspace, visual, "sync to the others");
    assert!(said.contains("every Develop setting, the masks and the crop") && said.contains("in proportion"), "{said}");
    let said = tell(&workspace, visual, "yes");
    assert!(said.contains("Tower stands the other way up from this photo, so the crop was turned a quarter to match."), "{said}");
    // The photo of the same size gets the very same crop; the upright one the same shape and angle, on itself.
    let same = edit_of(&workspace, visual, 1);
    assert_eq!((same.crop.rect, same.crop.angle, same.develop.contrast), (from.crop.rect, 2.0, 20.0));
    let upright = edit_of(&workspace, visual, 2);
    let rect = upright.crop.rect.unwrap();
    assert!((rect.width - rect.height).abs() < 0.5 && upright.crop.angle == 2.0 && upright.crop.fits(rect, (160, 240)), "{rect:?}");
    // "Undo that" takes the crop back with the rest.
    tell(&workspace, visual, "undo that");
    assert!(edit_of(&workspace, visual, 1).crop.is_neutral());
    // Left out by word, or the only thing copied.
    tell(&workspace, visual, "sync everything but the crop");
    tell(&workspace, visual, "yes");
    let other = edit_of(&workspace, visual, 1);
    assert!(other.crop.is_neutral() && other.develop.contrast == 20.0);
    tell(&workspace, visual, "contrast 0");
    tell(&workspace, visual, "sync only the crop");
    tell(&workspace, visual, "yes");
    let other = edit_of(&workspace, visual, 1);
    assert_eq!((other.crop.rect, other.develop.contrast), (from.crop.rect, 20.0));

    // Paste Settings leaves the crop alone, unless the crop is asked for by name.
    tell(&workspace, visual, "copy these settings");
    tell(&workspace, visual, "next photo");
    tell(&workspace, visual, "reset the crop");
    assert!(tell(&workspace, visual, "paste settings").contains("its crop stays its own"));
    assert!(edit_of(&workspace, visual, 1).crop.is_neutral());
    let said = tell(&workspace, visual, "paste only the crop");
    assert!(said.starts_with("Pasted the copied crop. The crop was carried over in proportion"), "{said}");
    assert_eq!(edit_of(&workspace, visual, 1).crop.rect, from.crop.rect);
}

#[gpui_kit::test]
fn iris_exports_straight_to_files_one_or_many_and_never_over_another(cx: &mut TestAppContext) {
    let (workspace, visual) = open_with_photos(cx, &["Beach", "Dunes", "Harbor"]); // each 240 × 160
    let folder = scratch_folder("iris-exports");
    let _ = std::fs::remove_dir_all(&folder);
    let files = |folder: &std::path::Path| {
        let mut names: Vec<String> = std::fs::read_dir(folder).map(|entries| entries.filter_map(|entry| Some(entry.ok()?.file_name().to_string_lossy().into_owned())).collect()).unwrap_or_default();
        names.sort();
        names
    };
    let size_of = |path: &std::path::Path| {
        let picture = develop_core::image_io::decode(&std::fs::read(path).unwrap()).unwrap();
        (picture.width, picture.height)
    };
    // A photo saved as a project exports to an "Exports" folder beside it, made on the way.
    visual.update(|_, cx| workspace.update(cx, |ws, _| ws.state.sessions[0].file = Some(folder.join("Beach.develop"))));
    let exports = folder.join("Exports");
    let said = tell(&workspace, visual, "export as JPEG at 80% quality, 120 pixels wide");
    assert!(said.starts_with(&format!("Exporting Beach as JPEG, quality 80 at 120 × 80 pixels to {}.", exports.display())), "{said}");
    assert!(said.ends_with(&format!("Exported: {}", exports.join("Beach.jpg").display())), "the reply says the file's full path once it is written: {said}");
    assert_eq!(size_of(&exports.join("Beach.jpg")), (120, 80));
    assert!(!visual.update(|window, cx| window.has_active_dialog(cx)), "no sheet");
    visual.update(|_, cx| assert_eq!(workspace.read(cx).ui.settings.export_folder.as_deref(), Some(exports.as_path()), "the folder is remembered"));
    assert_eq!(crate::app_settings::load().export.quality, 80, "and so are the choices");

    // Again: a new name, and the first file exactly as it was.
    let first = std::fs::read(exports.join("Beach.jpg")).unwrap();
    tell(&workspace, visual, "contrast 40");
    let said = tell(&workspace, visual, "export now");
    assert!(said.ends_with(&format!("Exported: {}", exports.join("Beach 2.jpg").display())), "{said}");
    assert_eq!(std::fs::read(exports.join("Beach.jpg")).unwrap(), first, "nothing is written over");
    assert_ne!(std::fs::read(exports.join("Beach 2.jpg")).unwrap(), first, "the new file has the new edit");

    // Two formats at once, and every open photo, each with its own edit and shape.
    let said = tell(&workspace, visual, "export as PNG and TIFF");
    assert!(said.contains("as PNG and TIFF") && said.contains("Exported 2 files to") && said.contains("• Beach.png\n• Beach.tif"), "{said}");
    visual.update(|_, cx| workspace.update(cx, |ws, _| ws.state.sessions[1].photo.edit.crop.rect = Some(develop_core::Rect::new(0.0, 0.0, 100.0, 160.0))));
    let said = tell(&workspace, visual, "export every open photo as jpeg, 100 pixels wide");
    assert!(said.starts_with("Exporting all 3 open photos as JPEG, quality 80") && said.contains("Exported 3 files to") && said.contains("• Beach 3.jpg\n• Dunes.jpg\n• Harbor.jpg"), "{said}");
    assert_eq!(size_of(&exports.join("Dunes.jpg")), (100, 160), "the cropped photo is as wide as asked, in its own shape");
    assert_eq!(size_of(&exports.join("Harbor.jpg")), (100, 67));
    assert_eq!(files(&exports), vec!["Beach 2.jpg", "Beach 3.jpg", "Beach.jpg", "Beach.png", "Beach.tif", "Dunes.jpg", "Harbor.jpg"]);

    // Joined to an edit, both happen: the light is set, then the file is written with it.
    let said = tell(&workspace, visual, "auto and export as JPEG");
    assert!(said.starts_with("Auto set the light from the photo: ") && said.contains("Exported: "), "{said}");
    // The recipe ends in a real export, and a plain "export" still opens the sheet.
    let said = tell(&workspace, visual, "web export");
    assert!(said.contains("Exported: ") && files(&exports).contains(&"Beach 5.jpg".to_owned()), "{said}");
    let said = tell(&workspace, visual, "export");
    assert!(said.starts_with("Export is open for Beach") && said.contains("name a format or say “export now”"), "{said}");
    assert!(visual.update(|window, cx| window.has_active_dialog(cx)));
    press(visual, "escape");

    // A photo that can't be written is named, and the others still are.
    visual.update(|_, cx| workspace.update(cx, |ws, _| ws.ui.settings.export_folder = Some(exports.join("Beach.jpg").join("not-a-folder"))));
    let said = tell(&workspace, visual, "export all as png");
    assert!(said.contains("Nothing was exported.") && said.contains("Couldn't export these 3:") && said.contains("• Dunes: "), "{said}");
    std::fs::remove_dir_all(&folder).ok();
}

/// Writes a small PNG of `shade` gray called `name` into `folder`, and returns its path.
fn picture_file(folder: &std::path::Path, name: &str, shade: f32) -> std::path::PathBuf {
    let mut bitmap = develop_core::Bitmap::filled(60, 40, develop_core::Rgba::rgb(shade, shade * 0.9, shade * 0.8));
    bitmap.set_pixel(0, 0, [10, 200, 30, 255]);
    let path = folder.join(name);
    std::fs::write(&path, develop_core::image_io::encode(&bitmap, develop_core::image_io::ExportFormat::Png, 100, [255; 3]).unwrap()).unwrap();
    path
}

#[gpui_kit::test]
fn recipes_run_from_their_own_key_and_button_and_by_themselves(cx: &mut TestAppContext) {
    let (workspace, visual) = open_with_photos(cx, &["Beach"]);
    // A key the app uses is refused; one free is bound, and pressing it runs the recipe.
    assert!(tell(&workspace, visual, "give mono finish the shortcut cmd s").contains("already does something here"));
    assert!(tell(&workspace, visual, "give mono finish the shortcut ctrl alt m").starts_with("⌃⌥M now runs “mono finish”."));
    press(visual, "ctrl-alt-m");
    visual.run_until_parked();
    assert_eq!(develop_of(&workspace, visual, 0).saturation, -100.0, "the key ran the recipe");
    assert_eq!(steps_of(&workspace, visual, 0).last().map(String::as_str), Some("Recipe: mono finish"), "as one Undo");
    assert!(visual.update(|_, cx| crate::panels::iris::tests_support::answer(workspace.read(cx)).join(" ")).contains("Dramatic Black & White"), "the result is in the chat");
    // Taken away, the key does nothing.
    tell(&workspace, visual, "reset everything");
    tell(&workspace, visual, "take the shortcut off mono finish");
    press(visual, "ctrl-alt-m");
    assert_eq!(develop_of(&workspace, visual, 0).saturation, 0.0);

    // A button at the top of the chat.
    click(visual, "rail-iris");
    assert!(!showing(visual, ("iris-recipe-button", 0_usize)));
    tell(&workspace, visual, "put fresh start on the toolbar");
    assert!(showing(visual, ("iris-recipe-button", 0_usize)));
    click(visual, ("iris-recipe-button", 0_usize));
    visual.run_until_parked();
    assert_ne!(develop_of(&workspace, visual, 0).exposure, 0.0, "Auto ran from the button");

    // What a recipe would do is said without doing it.
    let before = develop_of(&workspace, visual, 0);
    let said = tell(&workspace, visual, "what would portrait finish do?");
    assert!(said.contains("would do this, in order") && said.contains("Nothing has been changed."), "{said}");
    assert_eq!(develop_of(&workspace, visual, 0), before);

    // Before an export: the recipe runs first, as an Undo of its own, and what it did is exported.
    tell(&workspace, visual, "reset everything");
    let folder = scratch_folder("trigger-exports");
    let _ = std::fs::remove_dir_all(&folder);
    visual.update(|_, cx| workspace.update(cx, |ws, _| ws.ui.settings.export_folder = Some(folder.clone())));
    tell(&workspace, visual, "run mono finish before export");
    let said = tell(&workspace, visual, "export as png");
    assert!(said.contains("Running “mono finish” by itself, before each export:") && said.contains("Exported: "), "{said}");
    assert!(steps_of(&workspace, visual, 0).contains(&"Recipe: mono finish".to_owned()));
    let written = develop_core::image_io::decode(&std::fs::read(folder.join("Beach.png")).unwrap()).unwrap();
    let [r, g, b, _] = written.pixel(10, 10);
    assert!(r == g && g == b, "the exported photo is black and white");
    // A recipe whose step fails doesn't stop the export, and its own export doesn't set off
    // the trigger again.
    tell(&workspace, visual, "stop running mono finish before export");
    tell(&workspace, visual, "teach broken: frobnicate the widget, then export as jpeg");
    tell(&workspace, visual, "run broken before export");
    let said = tell(&workspace, visual, "export as tiff");
    assert!(said.contains("I didn't follow that") && said.contains("The export goes ahead anyway.") && said.contains("Beach.tif"), "{said}");
    assert!(!folder.join("Beach.jpg").exists(), "the triggered recipe stopped at its failed step");
    std::fs::remove_dir_all(&folder).ok();

    // Before a save, and when a photo is opened.
    tell(&workspace, visual, "stop running broken before export");
    tell(&workspace, visual, "reset everything");
    tell(&workspace, visual, "run fresh start before saving");
    let project = scratch_folder("trigger-save").join("Beach.develop");
    visual.update(|window, cx| workspace.update(cx, |ws, cx| {
        ws.state.sessions[0].file = Some(project.clone());
        ws.save(window, cx);
    }));
    visual.run_until_parked();
    // The recipe's change is in what was saved: the photo counts as saved with it.
    visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        assert!(project.exists() && state.sessions[0].photo.edit.develop.exposure != 0.0 && !state.is_unsaved(0), "the recipe ran before the photo was written");
    });
    tell(&workspace, visual, "stop running fresh start before saving");
    tell(&workspace, visual, "run mono finish when I open");
    let picture = picture_file(&scratch_folder("trigger-open"), "Opened.png", 0.6);
    visual.update(|window, cx| workspace.update(cx, |ws, cx| ws.open_path(picture.clone(), window, cx)));
    visual.run_until_parked();
    render(visual);
    visual.update(|_, cx| {
        let state = &workspace.read(cx).state;
        assert_eq!((state.sessions.len(), state.photo().unwrap().edit.develop.saturation), (2, -100.0), "the opened photo got the recipe");
    });
}

#[gpui_kit::test]
fn the_message_field_completes_recalls_and_the_conversation_is_kept(cx: &mut TestAppContext) {
    let (workspace, visual) = open_with_photos(cx, &["Beach"]);
    click(visual, "rail-iris");
    let input = visual.update(|_, cx| crate::panels::iris::tests_support::field_state(workspace.read(cx))).unwrap();
    visual.update(|window, cx| input.update(cx, |input, cx| input.focus(window, cx)));
    let field = |visual: &mut VisualTestContext| visual.update(|_, cx| crate::panels::iris::tests_support::field(workspace.read(cx), cx));
    // Completions as it is typed; Tab takes the first.
    visual.simulate_input("/re");
    render(visual);
    let offered = visual.update(|_, cx| crate::panels::iris::tests_support::completions(workspace.read(cx)));
    assert!(offered.contains(&"/recipes".to_owned()) && offered.contains(&"/reset".to_owned()) && offered.len() <= 5, "{offered:?}");
    assert!(showing(visual, ("iris-completion", 0_usize)));
    press(visual, "tab");
    assert_eq!(field(visual), offered[0]);
    visual.update(|window, cx| input.update(cx, |input, cx| input.set_value("", window, cx)));
    // Recipes complete by name too.
    visual.simulate_input("port");
    render(visual);
    assert_eq!(visual.update(|_, cx| crate::panels::iris::tests_support::completions(workspace.read(cx))), vec!["portrait finish".to_owned()]);
    visual.update(|window, cx| input.update(cx, |input, cx| input.set_value("", window, cx)));

    // The arrows go back through what was sent, and forward again.
    tell(&workspace, visual, "a bit brighter");
    tell(&workspace, visual, "warmer");
    press(visual, "up");
    assert_eq!(field(visual), "warmer");
    press(visual, "up");
    assert_eq!(field(visual), "a bit brighter");
    press(visual, "down");
    assert_eq!(field(visual), "warmer");
    press(visual, "down");
    assert_eq!(field(visual), "");
    // With a message being written, the arrows are the field's own.
    visual.simulate_input("shadows");
    press(visual, "up");
    assert_eq!(field(visual), "shadows");

    // The conversation is there after a restart, above "Earlier"; /clear forgets it.
    let (_, before) = visual.update(|_, cx| crate::panels::iris::tests_support::restored(workspace.read(cx)));
    visual.update(|_, cx| workspace.update(cx, |ws, _| crate::panels::iris::tests_support::restart(ws)));
    render(visual);
    assert_eq!(visual.update(|_, cx| crate::panels::iris::tests_support::restored(workspace.read(cx))), (before, before));
    assert!(showing(visual, "iris-earlier"));
    tell(&workspace, visual, "contrast 10");
    assert!(showing(visual, "iris-earlier"), "the line stays between then and now");
    visual.update(|window, cx| input.update(cx, |input, cx| input.set_value("", window, cx)));
    press(visual, "up");
    assert_eq!(field(visual), "contrast 10", "what was typed is kept too");
    tell(&workspace, visual, "/clear");
    visual.update(|_, cx| workspace.update(cx, |ws, _| crate::panels::iris::tests_support::restart(ws)));
    assert_eq!(visual.update(|_, cx| crate::panels::iris::tests_support::restored(workspace.read(cx))), (0, 0));
}

#[gpui_kit::test]
fn shortcuts_and_recipe_keys_are_changed_in_settings(cx: &mut TestAppContext) {
    let (workspace, visual) = open_with_photos(cx, &["Beach"]);
    // Keyboard Shortcuts: Crop takes C, and the key it had does nothing now.
    press(visual, "cmd-,");
    click(visual, ("settings-section", 7_usize));
    click(visual, ElementId::Name("shortcut-change:crop-tool".into()));
    press(visual, "c");
    assert_eq!(crate::app_settings::load().shortcuts.custom.get("crop-tool"), Some(&Some("c".to_owned())));
    assert_eq!(crate::keymap::key_for("crop-tool"), Some("C".to_owned()));
    // Taking a key another command has leaves that one without.
    click(visual, ElementId::Name("shortcut-change:linear-mask-tool".into()));
    press(visual, "w");
    assert_eq!(crate::keymap::key_for("white-balance-tool"), None);
    click(visual, "settings-done");
    let tool = |visual: &mut VisualTestContext| visual.update(|_, cx| workspace.read(cx).state.tool);
    press(visual, "r");
    assert_eq!(tool(visual), crate::state::Tool::View, "R is free now");
    press(visual, "c");
    assert_eq!(tool(visual), crate::state::Tool::Crop);
    press(visual, "escape");
    // Iris answers with the new key, and won't give a recipe one a command has.
    assert!(tell(&workspace, visual, "give mono finish the shortcut ctrl c").starts_with("⌃C now runs"), "ctrl-c is free");
    assert!(tell(&workspace, visual, "give fresh start the shortcut cmd shift e").contains("already does something here"));

    // Recipes: a key recorded for one, a button, a moment; a command can't take a recipe's key.
    press(visual, "cmd-,");
    click(visual, ("settings-section", 6_usize));
    assert!(showing(visual, ElementId::Name("recipe-card:portrait finish".into())));
    click(visual, ElementId::Name("recipe-key:portrait finish".into()));
    press(visual, "ctrl-alt-p");
    assert_eq!(visual.update(|_, cx| crate::panels::iris::tests_support::recipe_key(workspace.read(cx), "portrait finish")), Some("ctrl-alt-p".to_owned()));
    click(visual, ElementId::Name("recipe-button:portrait finish".into()));
    click(visual, ElementId::Name("recipe-when-0:portrait finish".into()));
    let recipe = visual.update(|_, cx| crate::panels::iris::recipes_page_data(workspace.read(cx)).1.into_iter().find(|recipe| recipe.name == "portrait finish").unwrap());
    assert!(recipe.button && recipe.when == vec![::iris::recipes::Trigger::BeforeExport]);
    let saved = ::iris::recipes::load(&crate::panels::iris::tests_support::folder(), ::iris::App::Develop);
    assert!(saved.iter().any(|recipe| recipe.name == "portrait finish" && recipe.key.as_deref() == Some("ctrl-alt-p")), "saved");
    click(visual, ("settings-section", 7_usize));
    click(visual, ElementId::Name("shortcut-change:radial-mask-tool".into()));
    press(visual, "ctrl-alt-p");
    assert!(visual.update(|window, _| window.try_find(ElementId::Name("settings-note".into())).is_some()));
    assert_ne!(crate::keymap::key_for("radial-mask-tool"), Some("⌃⌥P".to_owned()), "the recipe keeps its key");
    // Put back: every shortcut is Lightroom's again.
    click(visual, "shortcuts-reset-all");
    assert_eq!(crate::keymap::key_for("crop-tool"), Some("R".to_owned()));
    click(visual, "settings-done");
}

#[gpui_kit::test]
fn taught_recipes_are_edited_ordered_deleted_and_shared_from_settings(cx: &mut TestAppContext) {
    use crate::panels::iris::RecipeChange;
    let (workspace, visual) = open_with_photos(cx, &["Beach"]);
    tell(&workspace, visual, "teach warm up: warmer");
    tell(&workspace, visual, "teach cool down: cooler");
    let change = |visual: &mut VisualTestContext, change: RecipeChange| visual.update(|_, cx| workspace.update(cx, |ws, cx| crate::panels::iris::change_recipes(ws, change, cx)));
    let taught = |visual: &mut VisualTestContext| visual.update(|_, cx| crate::panels::iris::recipes_page_data(workspace.read(cx)).0.into_iter().map(|recipe| (recipe.name, recipe.steps)).collect::<Vec<_>>());
    // Edited: a new name and steps, one per line; a name taken or empty steps won't do.
    assert!(change(visual, RecipeChange::Edit { name: "warm up".into(), new_name: "Golden".into(), steps: vec!["warmer".into(), "".into(), "add a subtle vignette".into()] }).is_ok());
    assert!(change(visual, RecipeChange::Edit { name: "golden".into(), new_name: "portrait finish".into(), steps: vec!["warmer".into()] }).is_err());
    assert!(change(visual, RecipeChange::Edit { name: "golden".into(), new_name: "golden".into(), steps: vec![] }).is_err());
    assert!(change(visual, RecipeChange::Edit { name: "golden".into(), new_name: "golden".into(), steps: vec!["golden".into()] }).is_err(), "it can't run itself");
    assert_eq!(taught(visual)[0], ("golden".to_owned(), vec!["warmer".to_owned(), "add a subtle vignette".to_owned()]));
    // Ordered, deleted and put back.
    change(visual, RecipeChange::Move { name: "cool down".into(), up: true }).unwrap();
    assert_eq!(taught(visual).iter().map(|(name, _)| name.as_str()).collect::<Vec<_>>(), vec!["cool down", "golden"]);
    let golden = visual.update(|_, cx| crate::panels::iris::recipes_page_data(workspace.read(cx)).0[1].clone());
    change(visual, RecipeChange::Delete { name: "golden".into() }).unwrap();
    assert_eq!(taught(visual).len(), 1);
    change(visual, RecipeChange::Restore { recipe: golden, at: 1 }).unwrap();
    assert_eq!(taught(visual).len(), 2);
    // Shared as a file and read back: a file from another app is refused, and the import says
    // what was added.
    let text = visual.update(|_, cx| crate::panels::iris::recipes_share_text(workspace.read(cx)));
    change(visual, RecipeChange::Delete { name: "cool down".into() }).unwrap();
    let said = change(visual, RecipeChange::Import { text }).unwrap();
    assert!(said.starts_with("Added 1 recipe and replaced 1."), "{said}");
    assert!(change(visual, RecipeChange::Import { text: ::iris::recipes::share_text(::iris::App::Studio, &[]) }).unwrap_err().contains("Omina Motion"));
    assert!(tell(&workspace, visual, "golden").contains("Warmer"), "the edited recipe runs");

    // The page shows them, and its editor saves.
    press(visual, "cmd-,");
    click(visual, ("settings-section", 6_usize));
    click(visual, ElementId::Name("recipe-edit:golden".into()));
    click(visual, ElementId::Name("recipe-save:golden".into()));
    assert!(visual.update(|window, _| window.try_find(ElementId::Name("settings-note".into())).is_some()));
    click(visual, ElementId::Name("recipe-delete:golden".into()));
    assert!(showing(visual, "recipes-undo-delete"));
    click(visual, "recipes-undo-delete");
    assert_eq!(taught(visual).len(), 2);
    click(visual, "settings-done");
}

#[gpui_kit::test]
fn export_presets_are_used_saved_and_offered_in_the_sheet(cx: &mut TestAppContext) {
    let (workspace, visual) = open_with_photos(cx, &["Beach"]); // 240 × 160
    let folder = scratch_folder("preset-exports");
    let _ = std::fs::remove_dir_all(&folder);
    visual.update(|_, cx| workspace.update(cx, |ws, _| ws.ui.settings.export_folder = Some(folder.clone())));
    let listed = tell(&workspace, visual, "list export presets");
    assert!(listed.contains("Web: JPEG at quality 80, 2048 pixels on the long edge") && listed.contains("Print: TIFF, full size") && listed.contains("Email") && listed.contains("Instagram"), "{listed}");
    // "Instagram" is 1080 wide, and says it is the size only.
    let said = tell(&workspace, visual, "export for Instagram");
    assert!(said.contains("width only") && said.contains("Exported: "), "{said}");
    let size = |path: &std::path::Path| {
        let picture = develop_core::image_io::decode(&std::fs::read(path).unwrap()).unwrap();
        (picture.width, picture.height)
    };
    // A photo smaller than the preset stays its own size: Export never enlarges.
    assert_eq!(size(&folder.join("Beach.jpg")), (240, 160));
    tell(&workspace, visual, "export for print");
    assert!(folder.join("Beach.tif").exists());
    // The choices Export last used are kept under a name, and used by it.
    let said = tell(&workspace, visual, "save these export settings as Proofs");
    assert!(said.starts_with("Saved the export preset “Proofs”: TIFF"), "{said}");
    assert!(tell(&workspace, visual, "export for proofs").contains("Exported: "));
    assert!(folder.join("Beach 2.tif").exists(), "nothing written over");
    assert!(tell(&workspace, visual, "forget the Web export preset").contains("comes with"));
    assert!(tell(&workspace, visual, "forget the Proofs export preset").starts_with("Forgotten"));
    // The sheet has the presets and Save as Preset.
    press(visual, "cmd-shift-e");
    assert!(showing(visual, "export-preset") && showing(visual, "export-save-preset"));
    press(visual, "escape");
    std::fs::remove_dir_all(&folder).ok();
}

#[gpui_kit::test]
fn photos_are_handed_to_the_other_apps_and_theirs_are_brought_in(cx: &mut TestAppContext) {
    let (workspace, visual) = open_with_photos(cx, &["Beach"]);
    let root = crate::panels::iris::tests_support::folder();
    tell(&workspace, visual, "contrast 30");
    let said = tell(&workspace, visual, "send this to Edit");
    assert!(said.contains("Sent to Omina Edit: a PNG of the photo with your edits, 240 × 160 pixels."), "{said}");
    let sent = ::iris::handoff::take_ready(&root, ::iris::App::Canvas);
    assert_eq!(sent.len(), 1);
    assert_eq!((sent[0].from, sent[0].title.as_str()), (::iris::App::Develop, "the photo “Beach”"));
    assert!(develop_core::image_io::decode(&std::fs::read(&sent[0].files[0]).unwrap()).is_ok());
    assert!(tell(&workspace, visual, "put this photo in Motion").contains("Sent to Omina Motion"));
    assert_eq!(::iris::handoff::take_ready(&root, ::iris::App::Studio).len(), 1);

    // A frame Omina Motion sends opens as a new photo tab, and the chat says so.
    let frames = scratch_folder("handoff-in");
    let frame = picture_file(&frames, "Shot 1.png", 0.4);
    ::iris::handoff::send(&root, ::iris::App::Studio, ::iris::App::Develop, "the frame at 00:12", &[frame]).unwrap();
    visual.update(|window, cx| workspace.update(cx, |ws, cx| crate::panels::iris::receive_handoffs(ws, window, cx)));
    visual.run_until_parked();
    render(visual);
    visual.update(|_, cx| {
        let ws = workspace.read(cx);
        assert_eq!(ws.state.sessions.len(), 2);
        assert_eq!(ws.state.photo().unwrap().name, "Shot 1");
        let said = crate::panels::iris::tests_support::answer(ws).join(" ");
        assert!(said.contains("From Omina Motion: the frame at 00:12. Opening it as a new photo tab."), "{said}");
    });
    std::fs::remove_dir_all(&frames).ok();
}

#[gpui_kit::test]
fn a_folder_of_photos_is_edited_and_exported_without_opening_them(cx: &mut TestAppContext) {
    let (workspace, visual) = open_with_photos(cx, &["Beach"]);
    let folder = scratch_folder("folder-batch");
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    let pictures: Vec<std::path::PathBuf> = ["a.png", "b.png", "c.png"].iter().enumerate().map(|(index, name)| picture_file(&folder, name, 0.2 + index as f32 * 0.1)).collect();
    std::fs::write(folder.join("notes.txt"), "not a photo").unwrap();
    let originals: Vec<Vec<u8>> = pictures.iter().map(|path| std::fs::read(path).unwrap()).collect();
    // It asks first, saying how many, what, and that Undo can't take it back.
    let said = tell(&workspace, visual, &format!("auto every photo in {}", folder.display()));
    assert!(said.contains("make “auto” on each") && said.contains("3 photos") && said.contains("1 file that isn't one of the photos") && said.contains("Undo can't take this back") && said.contains("Go ahead?"), "{said}");
    assert!(!folder.join("a.omid").exists(), "nothing before the yes");
    tell(&workspace, visual, "Yes, do all 3");
    visual.run_until_parked();
    render(visual);
    let said = visual.update(|_, cx| crate::panels::iris::tests_support::answer(workspace.read(cx)).join("\n"));
    assert!(said.starts_with("Done: 3 of 3 photos") && said.contains("3 new projects"), "{said}");
    for (path, original) in pictures.iter().zip(&originals) {
        assert_eq!(&std::fs::read(path).unwrap(), original, "the photo itself is never written to");
        let project = develop_core::file::load(&path.with_extension(develop_core::file::EXTENSION)).unwrap();
        assert_ne!(project.edit.develop.exposure, 0.0, "Auto was made and kept in the project");
    }
    // Again: each photo's project is used, saved with the change, and exported.
    let said = tell(&workspace, visual, &format!("make it black and white for every photo in {} and export as JPEG", folder.display()));
    assert!(said.contains("3 saved over with the change") && said.contains("export each as JPEG to"), "{said}");
    tell(&workspace, visual, "yes");
    visual.run_until_parked();
    render(visual);
    let exports = folder.join("Exports");
    let mut written: Vec<String> = std::fs::read_dir(&exports).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
    written.sort();
    assert_eq!(written, vec!["a.jpg", "b.jpg", "c.jpg"]);
    let project = develop_core::file::load(&folder.join("b.omid")).unwrap();
    assert!(project.edit.develop.exposure != 0.0 && project.edit.develop.saturation == -100.0, "the earlier edit stays, with the new one on it");
    // Export only writes no projects; a request that can't be made on a closed photo is refused.
    let projects_before = std::fs::read(folder.join("a.omid")).unwrap();
    tell(&workspace, visual, &format!("export everything in {} as png", folder.display()));
    tell(&workspace, visual, "yes");
    visual.run_until_parked();
    assert_eq!(std::fs::read(folder.join("a.omid")).unwrap(), projects_before);
    assert!(exports.join("a.png").exists());
    assert!(tell(&workspace, visual, &format!("next photo for every photo in {}", folder.display())).contains("can't do “next photo”"));
    assert!(tell(&workspace, visual, "auto every photo in ~/No Such Folder Here").starts_with("There's no folder at"));
    // "No" leaves them; "stop" with nothing running is not a stop.
    tell(&workspace, visual, &format!("reset every photo in {}", folder.display()));
    assert_eq!(tell(&workspace, visual, "no"), "Left as they are.");
    std::fs::remove_dir_all(&folder).ok();
}
