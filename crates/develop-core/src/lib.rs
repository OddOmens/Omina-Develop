//! Develop's engine: everything about editing a photo, with no UI. The Develop settings
//! (white balance, tone, presence, tone curve, color mixer, color grading, detail, effects),
//! crop and straighten, masks for local adjustments, presets, and the .develop project file.
//! Edits never change the photo's pixels; `edit::Edit::render` makes the result from the
//! original every time.

pub mod adjust;
pub mod analysis;
pub mod assistant;
pub mod bitmap;
pub mod blur;
pub mod chat;
pub mod color;
pub mod crop;
pub mod develop;
pub mod edit;
pub mod file;
pub mod geom;
pub mod image_io;
pub mod masks;
pub mod photo;
pub mod presets;
pub mod straighten;

pub use bitmap::Bitmap;
pub use color::Rgba;
pub use edit::{Edit, View};
pub use geom::{Affine, IRect, Rect, Vec2};
pub use photo::{Photo, Snapshot, Source};
