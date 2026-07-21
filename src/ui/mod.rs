//! Sidebar presentation: palette, icons, layout geometry, activity strip.
//!
//! Shell/feature code draws through this module. Terminal-grid tricks
//! (half-block chips, Nerd Font slack) stay encapsulated here.

pub mod activity;
pub mod icons;
pub mod layout;
pub mod theme;

pub use activity::{ActivityBar, ActivityItem, draw_activity, hit};
pub use icons::{NerdFontSupport, detect as detect_nerd_font, has_nerd_font};
pub use theme::Palette;
