//! Thin binary over the `corral` library.
//!
//! Herdr runs this as the plugin pane command. No subcommands for now — open
//! the host shell and draw the left/right containers.

fn main() -> std::io::Result<()> {
    corral::run()
}
