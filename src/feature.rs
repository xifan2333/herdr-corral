//! Workbench features shown on the left (Explorer / SCM / GitHub).
//!
//! The shell only switches which feature is active. Real content mounts later;
//! this module owns identity, labels, icons, and key bindings for the activity
//! bar.

/// A left-side workbench feature (activity-bar item).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Feature {
    #[default]
    Explorer,
    Scm,
    GitHub,
}

impl Feature {
    pub const ALL: [Feature; 3] = [Feature::Explorer, Feature::Scm, Feature::GitHub];

    pub fn title(self) -> &'static str {
        match self {
            Feature::Explorer => "Explorer",
            Feature::Scm => "Source Control",
            Feature::GitHub => "GitHub",
        }
    }

    /// Short id for status / keys.
    pub fn id(self) -> &'static str {
        match self {
            Feature::Explorer => "explorer",
            Feature::Scm => "scm",
            Feature::GitHub => "github",
        }
    }

    /// Activity-bar glyph. Prefer Nerd Font; plain ASCII when unavailable.
    ///
    /// Material icons match herdr-sidebar's FA set (often **two cells** wide in
    /// non-Mono Nerd Fonts — callers should reserve a slack cell).
    pub fn icon(self, nerd_font: bool) -> &'static str {
        if nerd_font {
            match self {
                // nf-fa-folder
                Feature::Explorer => "\u{f07b}",
                // nf-fa-code_fork
                Feature::Scm => "\u{f126}",
                // nf-fa-github
                Feature::GitHub => "\u{f09b}",
            }
        } else {
            match self {
                Feature::Explorer => "E",
                Feature::Scm => "S",
                Feature::GitHub => "G",
            }
        }
    }

    /// True when the Nerd glyph is typically double-width (needs a slack cell).
    pub fn icon_double_width(self, nerd_font: bool) -> bool {
        nerd_font
    }

    /// Digit shortcut: `1` / `2` / `3`.
    pub fn from_digit(c: char) -> Option<Feature> {
        match c {
            '1' => Some(Feature::Explorer),
            '2' => Some(Feature::Scm),
            '3' => Some(Feature::GitHub),
            _ => None,
        }
    }

    pub fn index(self) -> usize {
        match self {
            Feature::Explorer => 0,
            Feature::Scm => 1,
            Feature::GitHub => 2,
        }
    }

    pub fn from_index(i: usize) -> Option<Feature> {
        Feature::ALL.get(i).copied()
    }

    pub fn next(self) -> Feature {
        Feature::from_index((self.index() + 1) % Feature::ALL.len()).unwrap()
    }

    pub fn prev(self) -> Feature {
        let n = Feature::ALL.len();
        Feature::from_index((self.index() + n - 1) % n).unwrap()
    }
}
