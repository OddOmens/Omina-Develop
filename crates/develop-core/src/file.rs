//! The .develop project file: a zip holding `photo.json` (the edit and snapshots), the original
//! picture file exactly as it was imported, and a small JPEG preview of the result for the
//! start screen. Files are untrusted input: `load` checks the format and version, refuses
//! oversized entries, decodes the original and repairs any setting out of range.

use std::io::{Cursor, Read, Write};
use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::bitmap::{Bitmap, MAX_SIDE};
use crate::edit::Edit;
use crate::image_io::{ExportFormat, encode};
use crate::photo::{MAX_SNAPSHOTS, Photo, Snapshot, Source, sanitize};

/// The file format's version. Files from a newer version are refused rather than misread.
pub const FORMAT_VERSION: u32 = 1;

/// The extension of project files.
pub const EXTENSION: &str = "develop";

/// The name the format is marked with inside the file.
const FORMAT_NAME: &str = "develop";

/// The longest side of the preview saved inside the file.
pub const PREVIEW_SIDE: u32 = 640;

/// The largest original accepted, in bytes (far beyond any camera's JPEG or TIFF).
const MAX_ORIGINAL_BYTES: u64 = 1 << 31;

#[derive(Serialize, Deserialize)]
struct Envelope {
    format: String,
    version: u32,
    name: String,
    /// The original's file name inside the zip, under `original/`.
    original: String,
    edit: Edit,
    #[serde(default)]
    snapshots: Vec<Snapshot>,
}

/// A file name safe to keep inside the zip: no folders, nothing hidden, never empty.
fn safe_file_name(name: &str) -> String {
    let base = Path::new(name).file_name().map_or_else(String::new, |base| base.to_string_lossy().into_owned());
    let cleaned: String = base.chars().filter(|c| !c.is_control() && !matches!(c, '/' | '\\' | ':')).collect();
    // Leading dots and spaces go together, so cleaning a name twice gives the same name (the
    // loader cleans the stored name again to find the original).
    let cleaned = cleaned.trim_start_matches(|c: char| c == '.' || c.is_whitespace()).trim_end().to_owned();
    if cleaned.is_empty() { "original".to_owned() } else { cleaned }
}

/// A small copy of `bitmap`, at most `PREVIEW_SIDE` on its longer side.
pub fn preview_of(bitmap: &Bitmap) -> Bitmap {
    let longest = bitmap.width.max(bitmap.height).max(1);
    if longest <= PREVIEW_SIDE {
        return bitmap.clone();
    }
    let scale = PREVIEW_SIDE as f32 / longest as f32;
    bitmap.resized(((bitmap.width as f32 * scale).round() as u32).max(1), ((bitmap.height as f32 * scale).round() as u32).max(1))
}

/// Writes a photo to bytes. `preview` is a rendered picture of the result (any size; it is
/// shrunk to fit).
pub fn save_to_bytes(photo: &Photo, preview: &Bitmap) -> Result<Vec<u8>, String> {
    let failed = |error: zip::result::ZipError| format!("The project couldn't be written: {error}");
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let compressed = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    // Picture files are compressed already; storing them as they are saves time.
    let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored).large_file(true);
    let original = safe_file_name(&photo.source.file_name);
    let envelope = Envelope { format: FORMAT_NAME.to_owned(), version: FORMAT_VERSION, name: photo.name.clone(), original: original.clone(), edit: photo.edit.clone(), snapshots: photo.snapshots.clone() };
    let json = serde_json::to_vec_pretty(&envelope).map_err(|error| format!("The project couldn't be written: {error}"))?;
    zip.start_file("photo.json", compressed).map_err(failed)?;
    zip.write_all(&json).map_err(|error| error.to_string())?;
    zip.start_file(format!("original/{original}"), stored).map_err(failed)?;
    zip.write_all(&photo.source.bytes).map_err(|error| error.to_string())?;
    zip.start_file("preview.jpg", stored).map_err(failed)?;
    zip.write_all(&encode(&preview_of(preview), ExportFormat::Jpeg, 82, [255; 3])?).map_err(|error| error.to_string())?;
    Ok(zip.finish().map_err(failed)?.into_inner())
}

/// Saves a photo to `path` (written beside it first, then renamed into place).
pub fn save(photo: &Photo, preview: &Bitmap, path: &Path) -> Result<(), String> {
    crate::image_io::write_atomically(path, &save_to_bytes(photo, preview)?)
}

/// Reads one entry of the zip, refusing any that claims to be bigger than `limit`.
fn read_entry(archive: &mut zip::ZipArchive<Cursor<&[u8]>>, name: &str, limit: u64) -> Result<Vec<u8>, String> {
    let damaged = |detail: String| format!("This project file is damaged ({detail}).");
    let file = archive.by_name(name).map_err(|_| damaged(format!("{name} is missing")))?;
    if file.size() > limit {
        return Err(damaged(format!("{name} is too large")));
    }
    let mut contents = Vec::with_capacity(file.size() as usize);
    // `take` guards against an entry that holds more than its header says.
    file.take(limit + 1).read_to_end(&mut contents).map_err(|error| damaged(error.to_string()))?;
    if contents.len() as u64 > limit {
        return Err(damaged(format!("{name} is too large")));
    }
    Ok(contents)
}

/// Reads a photo from a project file's bytes, checking it thoroughly.
pub fn load_from_bytes(bytes: &[u8]) -> Result<Photo, String> {
    let not_ours = || "This isn't an Omina Develop project file.".to_owned();
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| not_ours())?;
    let json = read_entry(&mut archive, "photo.json", 64 << 20)?;
    let envelope: Envelope = serde_json::from_slice(&json).map_err(|error| format!("This project file is damaged ({error})."))?;
    if envelope.format != FORMAT_NAME {
        return Err(not_ours());
    }
    if envelope.version > FORMAT_VERSION {
        return Err("This project was saved by a newer version of Omina Develop. Update Omina Develop to open it.".to_owned());
    }
    let original_name = safe_file_name(&envelope.original);
    // Projects saved before names were cleaned the same way twice may keep the original under
    // the stored name itself (" .beach.png" was kept as ".beach.png"), so that is looked for too.
    let original = read_entry(&mut archive, &format!("original/{original_name}"), MAX_ORIGINAL_BYTES)
        .or_else(|error| if envelope.original != original_name { read_entry(&mut archive, &format!("original/{}", envelope.original), MAX_ORIGINAL_BYTES) } else { Err(error) })?;
    let source = Source::decode(original_name, original).map_err(|error| format!("This project's photo can't be read: {error}"))?;
    if source.bitmap.width > MAX_SIDE || source.bitmap.height > MAX_SIDE {
        return Err("This project's photo is too large.".to_owned());
    }
    let size = source.size();
    let mut edit = envelope.edit;
    sanitize(&mut edit, size);
    let mut snapshots = envelope.snapshots;
    snapshots.truncate(MAX_SNAPSHOTS);
    for snapshot in &mut snapshots {
        sanitize(&mut snapshot.edit, size);
        snapshot.name = snapshot.name.chars().take(200).collect();
    }
    let name: String = envelope.name.chars().take(200).collect();
    Ok(Photo { name: if name.trim().is_empty() { "Untitled".to_owned() } else { name }, source: Arc::new(source), edit, snapshots })
}

/// Reads a project file.
pub fn load(path: &Path) -> Result<Photo, String> {
    let bytes = std::fs::read(path).map_err(|error| format!("{} can't be opened: {error}", path.display()))?;
    load_from_bytes(&bytes)
}

/// The preview saved in a project file, without reading the rest (for the start screen).
pub fn read_preview(path: &Path) -> Option<Bitmap> {
    let bytes = std::fs::read(path).ok()?;
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes.as_slice())).ok()?;
    let preview = read_entry(&mut archive, "preview.jpg", 32 << 20).ok()?;
    crate::image_io::decode(&preview).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Rgba;
    use crate::masks::{Mask, MaskShape};
    use crate::geom::Vec2;

    fn photo() -> Photo {
        let bitmap = Bitmap::filled(32, 24, Rgba::rgb(0.3, 0.5, 0.7));
        let bytes = encode(&bitmap, ExportFormat::Png, 100, [255; 3]).unwrap();
        let mut photo = Photo::new(Source::decode("../../etc/Beach Day.png", bytes).unwrap());
        photo.edit.develop.exposure = 0.7;
        photo.edit.masks.push(Mask::new("Sky", MaskShape::linear(Vec2::new(0.5, 0.0), Vec2::new(0.5, 0.6))));
        photo.snapshots.push(Snapshot { name: "First pass".into(), edit: photo.edit.clone() });
        photo
    }

    #[test]
    fn an_original_named_with_a_space_then_a_dot_opens_again() {
        let bytes = encode(&Bitmap::filled(8, 8, Rgba::rgb(0.3, 0.5, 0.7)), ExportFormat::Png, 100, [255; 3]).unwrap();
        let photo = Photo::new(Source::decode(" .beach.png", bytes).unwrap());
        let saved = save_to_bytes(&photo, &photo.source.bitmap).unwrap();
        let back = load_from_bytes(&saved).expect("the project opens again");
        assert_eq!(back.source.bytes, photo.source.bytes);
        assert_eq!(safe_file_name(&safe_file_name(" . .x.png ")), safe_file_name(" . .x.png "), "cleaning a name twice changes nothing");

        // A project saved before the fix keeps the original as ".beach.png" and still opens.
        let envelope = Envelope { format: FORMAT_NAME.to_owned(), version: FORMAT_VERSION, name: "beach".into(), original: ".beach.png".into(), edit: Edit::default(), snapshots: Vec::new() };
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default();
        zip.start_file("photo.json", options).unwrap();
        zip.write_all(&serde_json::to_vec(&envelope).unwrap()).unwrap();
        zip.start_file("original/.beach.png", options).unwrap();
        zip.write_all(&photo.source.bytes).unwrap();
        let older = zip.finish().unwrap().into_inner();
        assert_eq!(load_from_bytes(&older).expect("an older project opens").source.bytes, photo.source.bytes);
    }

    #[test]
    fn photos_round_trip_with_their_original_untouched() {
        let photo = photo();
        let bytes = save_to_bytes(&photo, &photo.source.bitmap).unwrap();
        let back = load_from_bytes(&bytes).unwrap();
        assert_eq!(back.name, "Beach Day");
        assert_eq!(back.edit, photo.edit);
        assert_eq!(back.snapshots, photo.snapshots);
        assert_eq!(back.source.bytes, photo.source.bytes, "the original is kept byte for byte");
        assert_eq!(back.source.bitmap, photo.source.bitmap);
        // The original's name never carries folders into the zip.
        assert_eq!(back.source.file_name, "Beach Day.png");
    }

    #[test]
    fn damaged_and_foreign_files_are_refused_with_a_message() {
        assert!(load_from_bytes(b"not a zip").unwrap_err().contains("isn't an Omina Develop project"));
        let photo = photo();
        let bytes = save_to_bytes(&photo, &photo.source.bitmap).unwrap();
        // A newer version is refused.
        let newer = rewrite_json(&bytes, |json| json.replace("\"version\": 1", "\"version\": 99"));
        assert!(load_from_bytes(&newer).unwrap_err().contains("newer version"));
        // Settings out of range are repaired, not refused.
        let wild = rewrite_json(&bytes, |json| json.replace("\"exposure\": 0.7", "\"exposure\": 1e30"));
        assert_eq!(load_from_bytes(&wild).unwrap().edit.develop.exposure, 5.0);
    }

    /// The same project file with its JSON changed by `change`.
    fn rewrite_json(bytes: &[u8], change: impl Fn(String) -> String) -> Vec<u8> {
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for index in 0..archive.len() {
            let mut file = archive.by_index(index).unwrap();
            let name = file.name().to_owned();
            let mut contents = Vec::new();
            file.read_to_end(&mut contents).unwrap();
            if name == "photo.json" {
                contents = change(String::from_utf8(contents).unwrap()).into_bytes();
            }
            zip.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(&contents).unwrap();
        }
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn the_preview_is_small_and_readable() {
        let photo = photo();
        let big = Bitmap::filled(2000, 1000, Rgba::rgb(0.5, 0.5, 0.5));
        let folder = std::env::temp_dir().join(format!("develop-file-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("photo.develop");
        save(&photo, &big, &path).unwrap();
        let preview = read_preview(&path).unwrap();
        assert_eq!((preview.width, preview.height), (PREVIEW_SIDE, PREVIEW_SIDE / 2));
        assert_eq!(load(&path).unwrap().edit, photo.edit);
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
