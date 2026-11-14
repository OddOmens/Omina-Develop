# Develop

Lightroom-style photo editing in Rust, beside Studio and Canvas: each open photo in a tab of its
own (with its own file and undo history), edited without ever changing its pixels. White balance,
tone, presence, tone curve, color mixer, color grading, detail and effects; crop and straighten;
linear and radial gradient masks; presets with live thumbnails and hover previews; snapshots;
copy and paste settings; before / after; a clipping warning; and export at full resolution.

Iris, the assistant shared with Studio (where it was called Tween), looks over each photo with
plain rules (no AI): overexposed or underexposed, blown highlights, crushed shadows, too much
contrast or too little, colors too strong or dull, a color cast, a sky much brighter than the
land, lifted shadows showing noise, and settings pushed far enough to cause halos. Each finding
says what it measured and has a fix to preview by hovering and make with a click. Its chat
answers from the photo and makes edits asked for in words ("shadows +30", "a bit warmer",
"try Vivid", "rotate left"), and answers how-to questions from the help Studio, Canvas and
Develop share. Iris can be turned off in Settings ▸ Iris.

Iris also works on a folder of photos without opening them (keeping each edit in a project
beside its photo), runs recipes from their own keys, buttons and moments (Settings ▸ Recipes),
exports with presets, and hands photos to Omina Edit and Omina Motion and brings theirs in.
Keyboard shortcuts can be changed in Settings ▸ Keyboard Shortcuts.

Iris's shared parts (her name and blinking, her help knowledge, and how questions find answers)
come from the `iris` crate in Studio's repository (`../Omina Motion/crates/iris`), linked by
path. Develop's own help topics live there too, in `src/knowledge/develop.toml`, tagged `develop`.

It uses Studio's stack (GPUI + gpui-kit) and Studio's look: the same themes and
accents, top bar of tools, movable dock panels, folding cards, value fields, sliders, dialogs,
start screen and settings. It began as the Develop part of Canvas (`../Omina Edit`).

The first launch opens a short welcome setup, as Studio's and Canvas's do: the look, the tools
and features to keep (from a starting point such as "Just the basics"), and whether to use Iris.
Each choice applies at once; Settings ▸ General ▸ Welcome setup shows it again. A tool or
feature turned off in Settings ▸ Tools & Features is hidden everywhere (top bar, rail, Develop
cards, menus, shortcuts), and whatever it already did to a photo stays.

## Layout

```
crates/develop-core   Everything about editing a photo, with no UI. The Develop settings and the
                      engine that applies them (`develop`), crop and straighten (`crop`, with
                      `straighten` for Level, which measures a photo's tilt from its own lines), masks
                      (`masks`), the edit and its rendering from the original (`edit`), presets
                      (`presets`), the photo and its snapshots (`photo`, which also repairs
                      untrusted settings), Iris's rules (`assistant`) and chat (`chat`),
                      the .develop project file (`file`: a zip of the edit
                      as JSON, the original file byte for byte, and a preview), histograms,
                      picture files (EXIF orientation applied on the way in) and bitmaps.
apps/develop          Desktop app (GPUI + gpui-kit). UI only: panels read `EditorState` and
                      forward input.
```

Inside `apps/develop/src`:

- `state/`: every editing action, unit-tested without a window. `mod.rs` holds the open photos
  (one shown at a time), each with its own history, snapshots, masks, crop commands, presets and copy / paste; `tools.rs` what the
  tools do on press, drag and release (crop handles, moving the crop, straightening by dragging
  outside it, drawing and reshaping masks); `history.rs` undo and the History panel's named steps.
- `preview.rs`: rendering for the screen off the UI thread. The original is turned once and
  halved into a pyramid; each render starts from the smallest level still sharp at the zoom (a
  smaller one while dragging), and the last finished picture shows until the next arrives.
- `workspace.rs`: the window, menu bar, shortcuts and dock; `commands.rs` what each command
  does; `project_files.rs` open, save, close and quit; `features.rs` the tools and features
  that can be turned off, and the welcome setup's starting points.
- `panels/`: one module per panel. `viewer.rs` is the photo with the tools' marks (crop box and
  grid, mask outlines and handles, the before / after divider), the tool options strip and the
  status bar. `develop.rs` is the Develop panel; `presets.rs`, `snapshots.rs`, `history.rs`;
  `iris.rs` Iris's panel, its measuring in the background and its badge on the photo;
  `toolbar.rs` the top bar and the rail; `start_screen.rs`, `onboarding.rs` (the welcome setup),
  `export.rs`, `settings.rs`, `dialogs.rs`.
  `chrome.rs`, `dock.rs`, `fields.rs`, `curves.rs` and `theme.rs` come from Studio and Canvas,
  so the apps look and behave alike.
- `ui_tests.rs`: headless tests of the real window, driven with simulated clicks and keys. Tests
  keep settings in a scratch folder per test thread, never the real settings file.
- `keymap.rs`: Lightroom's keys (R crop, W white balance, M / ⇧M masks, \ before, Y split,
  Z zoom, J clipping, ⌘U auto, ⇧⌘C / ⇧⌘V copy and paste settings, ⌘[ ⌘] rotate, ⌃Tab next photo…). Tooltips read keys from here.

## Commands

```bash
cargo run -p develop --release               # run the editor
cargo run -p develop --release -- photo.jpg  # open a photo or .develop project straight away
cargo test --workspace                       # run all tests
cargo clippy --workspace --all-targets
```

## Conventions

- The original is never changed. Every preview and export is `Edit::render` from it, in this
  order: turn and flip, develop, masks, straighten and crop, then vignette and grain.
- Photo changes go through `EditorState::edit` (or a helper built on it), which bumps `revision`
  and names the step. Undo needs nothing from new edits: the workspace records a step whenever
  the photo changed and no drag is in progress.
- Masks are placed in fractions of the image (the photo turned and flipped, before
  straightening and cropping), so they stay on the same part of the photo whatever the crop.
- Project files and settings are untrusted: `photo::sanitize` repairs every value read.
- Every visible control works or is not shown. `unsafe` is forbidden workspace-wide.

## License

GPL-3.0-or-later, as Studio and Canvas. See `LICENSE`.
