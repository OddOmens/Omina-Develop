//! Sending a photo to Omina Edit or Omina Motion, and bringing in what they send, through the
//! hand-off folders every app shares (`iris::handoff`). A photo goes as a PNG of the finished
//! photo, developed from the original at full size, so nothing is lost on the way. What comes
//! in (a picture from Omina Edit, a frame from Omina Motion) opens as a new photo tab.

use std::path::PathBuf;

use develop_core::chat::OtherApp;
use develop_core::image_io::{ExportFormat, encode, write_atomically};
use gpui_kit::{AppContext as _, Context, Window};
use ::iris::App;

use super::conversation::Outcome;
use super::{Message, show_newest};
use crate::workspace::Workspace;

/// The app a request names, as the shared crate knows it.
fn app_of(to: OtherApp) -> App {
    match to {
        OtherApp::Edit => App::Canvas,
        OtherApp::Motion => App::Studio,
    }
}

/// Opens the app a hand-off went to. Not in tests, which must never start another app.
fn open_app(to: App) -> Result<(), String> {
    if cfg!(test) { Ok(()) } else { ::iris::handoff::open_app(to) }
}

/// Sends the photo showing, as edited, to `to`: develops it at full size off the UI thread,
/// writes it into a hand-off package and opens the other app, saying what was sent once it has
/// been.
pub fn send_photo(workspace: &mut Workspace, to: OtherApp, window: &mut Window, cx: &mut Context<Workspace>) -> Outcome {
    let Some(photo) = workspace.state.photo().cloned() else { return Outcome::failed() };
    let Some(root) = super::shared_folder() else {
        workspace.ui.iris.messages.push(Message::from_iris("There's no shared folder to hand the photo over through."));
        return Outcome::failed();
    };
    let app = app_of(to);
    workspace.ui.iris.messages.push(Message::from_iris(format!("Sending “{}” to {} as it is edited now…", photo.name, to.name())));
    cx.spawn_in(window, async move |this, cx| {
        let sent = cx
            .background_spawn(async move {
                let finished = photo.edit.render_full(&photo.source.bitmap);
                let bytes = encode(&finished, ExportFormat::Png, 100, [255; 3])?;
                let scratch = std::env::temp_dir().join(format!("develop-handoff-{}", std::process::id()));
                std::fs::create_dir_all(&scratch).map_err(|error| format!("A file to send couldn't be made: {error}"))?;
                let file: PathBuf = scratch.join(format!("{}.png", photo.name.replace(['/', '\\', ':'], "-")));
                write_atomically(&file, &bytes)?;
                let title = format!("the photo “{}”", photo.name);
                let sent = ::iris::handoff::send(&root, App::Develop, app, &title, std::slice::from_ref(&file));
                let _ = std::fs::remove_file(&file);
                sent.map(|_| (finished.width, finished.height))
            })
            .await;
        this.update_in(cx, |this, window, cx| {
            let text = match sent {
                Ok((width, height)) => match open_app(app) {
                    Ok(()) => format!("Sent to {}: a PNG of the photo with your edits, {width} × {height} pixels. {} is opening and brings it in.", to.name(), to.name()),
                    Err(why) => format!("The photo is ready for {}, a PNG with your edits ({width} × {height} pixels), but {why}", to.name()),
                },
                Err(why) => format!("The photo couldn't be sent to {}: {why}", to.name()),
            };
            this.ui.iris.messages.push(Message::from_iris(text));
            super::conversation::keep_conversation(this);
            show_newest(window, cx);
        })
        .ok();
    })
    .detach();
    Outcome::default()
}

/// Brings in what Omina Edit and Omina Motion have sent, each file in a photo tab of its own,
/// and says so in the chat. Called when Develop starts and every few seconds while it runs.
pub fn receive(workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>) {
    let Some(root) = super::shared_folder() else { return };
    let packages = ::iris::handoff::take_ready(&root, App::Develop);
    if packages.is_empty() {
        return;
    }
    for package in packages {
        let (opened, other): (Vec<PathBuf>, Vec<PathBuf>) = package.files.into_iter().partition(|file| develop_core::image_io::is_picture(file) || file.extension().is_some_and(|extension| extension == develop_core::file::EXTENSION));
        let mut text = format!("From {}: {}.", package.from.name(), package.title);
        if !opened.is_empty() {
            text.push_str(&format!(" Opening {} as a new photo tab.", if opened.len() == 1 { "it".to_owned() } else { format!("its {} pictures each", opened.len()) }));
        }
        if !other.is_empty() {
            let names: Vec<String> = other.iter().filter_map(|file| Some(file.file_name()?.to_string_lossy().into_owned())).collect();
            text.push_str(&format!(" I can't open {} here: Omina Develop opens photos.", names.join(", ")));
        }
        workspace.ui.iris.messages.push(Message::from_iris(text));
        workspace.ui.iris.chat_chosen = Some(true);
        workspace.open_paths(opened, window, cx);
    }
    super::conversation::keep_conversation(workspace);
    show_newest(window, cx);
}
