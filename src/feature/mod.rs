//! Feature identity + view mount points.
//!
//! The shell switches which [`Feature`] is active; each feature implements
//! [`view::FeatureView`] for body draw / key / click handling. Activity-bar
//! icons and digit shortcuts stay on the id enum.

mod explorer;
mod github;
mod scm;
mod view;

pub use view::{FeatureView, KeyOutcome};

use crate::config::Config;
use explorer::ExplorerView;
use github::GitHubView;
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
    pub const ALL: [Self; 3] = [Self::Explorer, Self::Scm, Self::GitHub];

    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Explorer => "Explorer",
            Self::Scm => "Source Control",
            Self::GitHub => "GitHub",
        }
    }

    #[must_use]
    pub const fn id(self) -> &'static str {
        match self {
            Self::Explorer => "explorer",
            Self::Scm => "scm",
            Self::GitHub => "github",
        }
    }

    #[must_use]
    pub const fn icon(self, nerd_font: bool) -> &'static str {
        if nerd_font {
            match self {
                Self::Explorer => "\u{f07b}",
                Self::Scm => "\u{f126}",
                Self::GitHub => "\u{f09b}",
            }
        } else {
            match self {
                Self::Explorer => "E",
                Self::Scm => "S",
                Self::GitHub => "G",
            }
        }
    }

    #[must_use]
    pub const fn icon_double_width(self, nerd_font: bool) -> bool {
        nerd_font
    }

    #[must_use]
    pub const fn from_digit(c: char) -> Option<Self> {
        match c {
            '1' => Some(Self::Explorer),
            '2' => Some(Self::Scm),
            '3' => Some(Self::GitHub),
            _ => None,
        }
    }

    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Explorer => 0,
            Self::Scm => 1,
            Self::GitHub => 2,
        }
    }

    #[must_use]
    pub fn from_index(i: usize) -> Option<Self> {
        Self::ALL.get(i).copied()
    }
}

/// All feature view instances owned by the shell.
pub struct Views {
    explorer: ExplorerView,
    scm: ScmView,
    github: GitHubView,
}

impl Views {
    #[must_use]
    pub fn new(cwd: &std::path::Path, nerd_font: bool, config: Arc<Config>) -> Self {
        Self {
            explorer: ExplorerView::new(cwd.to_path_buf(), nerd_font, Arc::clone(&config)),
            scm: ScmView::new(cwd.to_path_buf(), nerd_font, Arc::clone(&config)),
            github: GitHubView::new(cwd.to_path_buf(), config),
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
