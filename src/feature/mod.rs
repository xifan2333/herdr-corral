//! Feature identity + view mount points.
//!
//! The shell switches which [`Feature`] is active; each feature implements
//! [`view::FeatureView`] for body draw / key / click handling. Activity-bar
//! icons and digit shortcuts stay on the id enum.

mod explorer;
mod placeholder;
mod scm;
mod view;

pub use view::{FeatureView, KeyOutcome};

use crate::config::Config;
use explorer::ExplorerView;
use placeholder::PlaceholderView;
use scm::ScmView;
use std::sync::Arc;

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

    pub fn id(self) -> &'static str {
        match self {
            Feature::Explorer => "explorer",
            Feature::Scm => "scm",
            Feature::GitHub => "github",
        }
    }

    pub fn icon(self, nerd_font: bool) -> &'static str {
        if nerd_font {
            match self {
                Feature::Explorer => "\u{f07b}",
                Feature::Scm => "\u{f126}",
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

    pub fn icon_double_width(self, nerd_font: bool) -> bool {
        nerd_font
    }

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
    explorer: ExplorerView,
    scm: ScmView,
    github: PlaceholderView,
}

impl Views {
    pub fn new(cwd: &std::path::Path, nerd_font: bool, config: Arc<Config>) -> Self {
        Self {
            explorer: ExplorerView::new(cwd.to_path_buf(), nerd_font, Arc::clone(&config)),
            scm: ScmView::new(cwd.to_path_buf(), nerd_font, config),
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
