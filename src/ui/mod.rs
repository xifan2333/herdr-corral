//! Shared UI primitives for the sidebar shell.
//!
//! Terminal-grid tricks (half-block chips, double-width glyph slack) live here
//! so feature/app code only passes data, not `▄` / `▀`.

pub mod activity;

pub use activity::{ActivityBar, ActivityItem, draw_activity, hit};
