//! Tools and features that can be turned off (Settings ▸ Tools & Features), so Develop shows
//! only what someone uses. A feature that is off is hidden everywhere it would appear: the top
//! bar, the rail, the menu bar, the Develop panel's cards and the photo's status bar. Its
//! keyboard shortcut does nothing. Whatever it already did to a photo stays and keeps working
//! (a tone curve stays on the photo with the Tone Curve card hidden, say).
//!
//! A few things are always on, because Develop can't be used without them: Open, Save, Export,
//! Settings, the Hand, the Basic and Histogram cards, rotating and flipping, and the History
//! panel (where the side panel goes when every other one is off).
//!
//! Iris keeps her own switch (`AppSettings::iris_on`, the same one as Settings ▸ Iris), so it
//! reads the same on her page and here.

use std::collections::BTreeSet;

use serde::{Deserialize, Deserializer, Serialize};

use crate::app_settings::AppSettings;
use crate::state::Tool;
use crate::workspace::LeftPanel;

/// Something that can be turned off. The names are saved in settings, so they never change.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Feature {
    // Tools in the top bar.
    Crop,
    WhiteBalanceSelector,
    Masks,
    // Panels on the rail.
    Presets,
    Snapshots,
    // Cards in the Develop panel.
    ToneCurve,
    ColorMixer,
    ColorGrading,
    Detail,
    Effects,
    // Looking and comparing.
    BeforeAfter,
    Clipping,
    CopyPaste,
    // Iris, stored in `AppSettings::iris_on`.
    Iris,
}

/// A feature as Settings lists it.
pub struct FeatureInfo {
    pub feature: Feature,
    pub name: &'static str,
    /// Where it shows up, so it's clear what turning it off hides.
    pub about: &'static str,
}

const fn info(feature: Feature, name: &'static str, about: &'static str) -> FeatureInfo {
    FeatureInfo { feature, name, about }
}

/// Every feature that can be turned off, in groups, in the order Settings lists them.
pub const GROUPS: &[(&str, &[FeatureInfo])] = &[
    (
        "Tools",
        &[
            info(Feature::Crop, "Crop & Straighten", "The Crop tool (R), the Crop card, Level and Reset Crop."),
            info(Feature::WhiteBalanceSelector, "White Balance Selector", "The eyedropper (W) that makes a clicked spot neutral."),
            info(Feature::Masks, "Masks", "Linear and radial gradients (M, ⇧M) and the Masks card."),
        ],
    ),
    (
        "Panels",
        &[
            info(Feature::Presets, "Presets", "The Presets panel on the rail, and New Preset."),
            info(Feature::Snapshots, "Snapshots", "The Snapshots panel on the rail, and New Snapshot (⌘N)."),
        ],
    ),
    (
        "Develop cards",
        &[
            info(Feature::ToneCurve, "Tone Curve", "The curve for the whole photo and for red, green and blue."),
            info(Feature::ColorMixer, "Color Mixer", "Hue, saturation and luminance of eight colors."),
            info(Feature::ColorGrading, "Color Grading", "Tints for the shadows, midtones and highlights."),
            info(Feature::Detail, "Detail", "Sharpening and noise reduction."),
            info(Feature::Effects, "Effects", "The post-crop vignette and grain."),
        ],
    ),
    (
        "Viewing",
        &[
            info(Feature::BeforeAfter, "Before / After", "Showing the photo as it came (\\) and the split (Y)."),
            info(Feature::Clipping, "Clipping Warning", "Painting clipped highlights and shadows on the photo (J)."),
            info(Feature::CopyPaste, "Copy & Paste Settings", "Giving one photo's look to another (⇧⌘C, ⇧⌘V)."),
        ],
    ),
    (IRIS_GROUP, &[info(Feature::Iris, "Iris", "The assistant: suggestions for the photo, fixes in a click, and a chat. Plain rules, not AI.")]),
];

/// The group that holds Iris, who has a page of her own in the welcome setup.
pub const IRIS_GROUP: &str = "Assistant";

/// Every feature outside Iris's group: what the starting points choose between.
pub fn tool_features() -> impl Iterator<Item = Feature> {
    GROUPS.iter().filter(|(group, _)| *group != IRIS_GROUP).flat_map(|(_, features)| features.iter().map(|info| info.feature))
}

/// Starting points for the welcome setup's "How do you edit?": each turns on every tool and
/// feature except its own short list. Iris is chosen separately.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Focus {
    Everything,
    Corrections,
    CreativeLooks,
    Basics,
}

impl Focus {
    pub const ALL: [Focus; 4] = [Self::Everything, Self::Corrections, Self::CreativeLooks, Self::Basics];

    pub fn name(self) -> &'static str {
        match self {
            Self::Everything => "Everything",
            Self::Corrections => "Careful corrections",
            Self::CreativeLooks => "Creative looks",
            Self::Basics => "Just the basics",
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Self::Everything => "Every tool and feature Omina Develop has.",
            Self::Corrections => "True color and clean detail: curves, masks and the clipping warning, without presets or effects.",
            Self::CreativeLooks => "Presets, color grading, curves and effects, without the technical checks.",
            Self::Basics => "Light and color sliders, crop, presets, export. Fewer buttons to learn.",
        }
    }

    /// What this starting point turns off.
    pub fn off(self) -> &'static [Feature] {
        use Feature::*;
        match self {
            Self::Everything => &[],
            Self::Corrections => &[Presets, ColorGrading, Effects],
            Self::CreativeLooks => &[WhiteBalanceSelector, Detail, Clipping],
            Self::Basics => &[WhiteBalanceSelector, Masks, Snapshots, ToneCurve, ColorMixer, ColorGrading, Detail, Effects, Clipping, CopyPaste],
        }
    }

    /// Turns every tool and feature on, except this starting point's list.
    pub fn apply(self, settings: &mut AppSettings) {
        for feature in tool_features() {
            settings.set_feature(feature, !self.off().contains(&feature));
        }
    }

    /// The starting point the current choices match exactly, if any.
    pub fn matching(settings: &AppSettings) -> Option<Self> {
        Self::ALL.into_iter().find(|focus| tool_features().all(|feature| settings.feature_on(feature) != focus.off().contains(&feature)))
    }
}

impl Feature {
    /// The setting's id, for its switch and checkbox.
    pub fn id(self) -> String {
        serde_json::to_value(self).ok().and_then(|value| value.as_str().map(str::to_owned)).unwrap_or_default()
    }

    /// The feature a tool belongs to, or `None` for the Hand, which is always on.
    pub fn of_tool(tool: Tool) -> Option<Self> {
        Some(match tool {
            Tool::View => return None,
            Tool::Crop => Self::Crop,
            Tool::WhiteBalance => Self::WhiteBalanceSelector,
            Tool::Masks => Self::Masks,
        })
    }

    /// The feature a side panel belongs to, or `None` for History, which is always on.
    pub fn of_panel(panel: LeftPanel) -> Option<Self> {
        Some(match panel {
            LeftPanel::History => return None,
            LeftPanel::Presets => Self::Presets,
            LeftPanel::Snapshots => Self::Snapshots,
            LeftPanel::Iris => Self::Iris,
        })
    }

    /// The feature a command belongs to (by its id in `keymap::SHORTCUTS`, or a menu-only id
    /// such as "reset-crop"), if any.
    pub fn of_command(id: &str) -> Option<Self> {
        Some(match id {
            "crop-tool" | "reset-crop" | "level" => Self::Crop,
            "white-balance-tool" => Self::WhiteBalanceSelector,
            "linear-mask-tool" | "radial-mask-tool" | "delete-mask" => Self::Masks,
            "new-preset" => Self::Presets,
            "new-snapshot" => Self::Snapshots,
            "before" | "split" => Self::BeforeAfter,
            "clipping" => Self::Clipping,
            "copy-settings" | "paste-settings" => Self::CopyPaste,
            _ => return None,
        })
    }
}

impl AppSettings {
    /// Whether `feature` is on.
    pub fn feature_on(&self, feature: Feature) -> bool {
        match feature {
            Feature::Iris => self.iris_on,
            _ => !self.features_off.contains(&feature),
        }
    }

    /// Turns `feature` on or off.
    pub fn set_feature(&mut self, feature: Feature, on: bool) {
        match feature {
            Feature::Iris => self.iris_on = on,
            _ if on => {
                self.features_off.remove(&feature);
            }
            _ => {
                self.features_off.insert(feature);
            }
        }
    }

    /// Whether a command can be used now (its feature, if it has one, is on).
    pub fn command_on(&self, id: &str) -> bool {
        Feature::of_command(id).is_none_or(|feature| self.feature_on(feature))
    }

    /// Whether a tool can be picked now.
    pub fn tool_on(&self, tool: Tool) -> bool {
        Feature::of_tool(tool).is_none_or(|feature| self.feature_on(feature))
    }

    /// Whether a side panel can be shown now.
    pub fn panel_on(&self, panel: LeftPanel) -> bool {
        Feature::of_panel(panel).is_none_or(|feature| self.feature_on(feature))
    }
}

/// Reads the saved list of features that are off, skipping any name this copy of Develop
/// doesn't know (one from a newer version, say) instead of losing the whole list.
pub fn read_features_off<'de, D: Deserializer<'de>>(deserializer: D) -> Result<BTreeSet<Feature>, D::Error> {
    let names = Vec::<serde_json::Value>::deserialize(deserializer)?;
    Ok(names.into_iter().filter_map(|name| serde_json::from_value(name).ok()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_feature_is_listed_once() {
        let listed: Vec<Feature> = GROUPS.iter().flat_map(|(_, features)| features.iter().map(|info| info.feature)).collect();
        let unique: BTreeSet<Feature> = listed.iter().copied().collect();
        assert_eq!(listed.len(), unique.len(), "no feature is listed twice");
        // Adding a variant without listing it would leave it with no switch.
        assert_eq!(unique.len(), 14);
    }

    #[test]
    fn turning_features_off_and_on_is_remembered_by_name() {
        let mut settings = AppSettings::default();
        assert!(settings.feature_on(Feature::Masks));
        settings.set_feature(Feature::Masks, false);
        assert!(!settings.command_on("delete-mask"), "the mask commands go with the tool");
        assert!(!settings.tool_on(Tool::Masks));
        assert!(settings.tool_on(Tool::View), "the Hand is always on");
        settings.set_feature(Feature::Iris, false);
        assert!(!settings.iris_on, "Iris shares her switch with Settings ▸ Iris");
        assert!(!settings.panel_on(LeftPanel::Iris) && settings.panel_on(LeftPanel::History));

        let text = serde_json::to_string(&settings).unwrap();
        assert!(text.contains("\"masks\""));
        let read: AppSettings = serde_json::from_str(&text).unwrap();
        assert!(!read.feature_on(Feature::Masks));

        settings.set_feature(Feature::Masks, true);
        assert!(settings.feature_on(Feature::Masks));
    }

    #[test]
    fn starting_points_set_the_tools_and_are_recognized() {
        let mut settings = AppSettings::default();
        assert_eq!(Focus::matching(&settings), Some(Focus::Everything));
        settings.set_feature(Feature::Iris, false);
        Focus::Basics.apply(&mut settings);
        assert_eq!(Focus::matching(&settings), Some(Focus::Basics));
        assert!(!settings.feature_on(Feature::Masks) && settings.feature_on(Feature::Crop) && settings.feature_on(Feature::Presets));
        assert!(!settings.iris_on, "Iris is left alone");
        settings.set_feature(Feature::Masks, true);
        assert_eq!(Focus::matching(&settings), None, "a hand-picked mix matches none");
        Focus::Everything.apply(&mut settings);
        assert!(settings.features_off.is_empty());
    }

    #[test]
    fn an_unknown_name_in_the_saved_list_is_skipped() {
        let read: AppSettings = serde_json::from_str(r#"{"features_off": ["masks", "a-feature-from-the-future"]}"#).unwrap();
        assert!(!read.feature_on(Feature::Masks));
        assert!(read.feature_on(Feature::Crop));
    }
}
