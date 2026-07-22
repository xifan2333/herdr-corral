//! corral — VS Code-style sidebar for the terminal.
//!
//! Shape (aligned with herdr-sidebar):
//! - **one left-docked pane** hosts Explorer / SCM / GitHub
//! - feature switch is in-process (top activity icons)
//! - behaviour is configured in shell (`config.sh`: binds + functions)
//! - also runs standalone (`corral`) without Herdr
//!
//! Modules:
//! - [`host`] — plugin vs standalone launch context
//! - [`config`] — `config.sh` binds + shell actions
//! - [`herdr`] — host CLI / future RPC
//! - [`ui`] — palette, icons, layout, activity strip
//! - [`feature`] — Explorer / SCM / GitHub + [`feature::FeatureView`]
//! - [`app`] — sidebar event loop / key routing

pub mod app;
pub mod config;
pub mod diffview;
pub mod feature;
pub mod git;
pub mod github;
pub mod herdr;
pub mod host;
pub mod ui;

pub use feature::{Feature, FeatureView, KeyOutcome};
pub use host::{LaunchContext, Mode};
pub use ui::{has_nerd_font, NerdFontSupport, Palette};

/// Entry point invoked by the binary (plugin pane or standalone).
pub fn run() -> std::io::Result<()> {
    let ctx = host::from_env();
    app::run(ctx)
}
