//! Feature identity + view mount points.
//!
//! The shell switches which [`Feature`] is active; each feature implements
//! [`view::FeatureView`] for body draw / key / click handling. Activity-bar
//! icons and digit shortcuts stay on the id enum.

mod placeholder;
mod view;

pub use view::{FeatureView, KeyOutcome};

use placeholder::PlaceholderView;

/// A sidebar feature (activity-bar item).
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
                Feature::Explorer => "\u{f07b}", // nf-fa-folder
                Feature::Scm => "\u{f126}",      // nf-fa-code_fork
                Feature::GitHub => "\u{f09b}",   // nf-fa-github
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

    /// Digit shortcut: `1` / `2` / `3` (shell-owned feature switch).
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
}

/// All feature view instances owned by the shell.
pub struct Views {
    explorer: PlaceholderView,
    scm: PlaceholderView,
    github: PlaceholderView,
}

impl Views {
    pub fn new(cwd: &std::path::Path) -> Self {
        Self {
            explorer: PlaceholderView::new(
                Feature::Explorer,
                format!("file tree goes here\n{}", cwd.display()),
            ),
            scm: PlaceholderView::new(Feature::Scm, "git changes go here".into()),
            github: PlaceholderView::new(Feature::GitHub, "issues / PRs go here".into()),
        }
    }

    pub fn get(&self, feature: Feature) -> &dyn FeatureView {
        match feature {
            Feature::Explorer => &self.explorer,
            Feature::Scm => &self.scm,
            Feature::GitHub => &self.github,
        }
    }

    pub fn get_mut(&mut self, feature: Feature) -> &mut dyn FeatureView {
        match feature {
            Feature::Explorer => &mut self.explorer,
            Feature::Scm => &mut self.scm,
            Feature::GitHub => &mut self.github,
        }
    }
}
