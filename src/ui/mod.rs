//! Sidebar presentation: palette, icons, layout geometry, activity strip.
//!
//! Shell/feature code draws through this module. Terminal-grid tricks
//! (half-block chips, Nerd Font slack) stay encapsulated here.

pub mod activity;
pub mod icons;
pub mod layout;
pub mod theme;

pub use activity::{draw_activity, hit, ActivityBar, ActivityItem};
pub use icons::{detect as detect_nerd_font, has_nerd_font, NerdFontSupport};
pub use theme::Palette;
