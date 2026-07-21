//! Thin binary over the `corral` library.
//!
//! - standalone: `./target/release/corral` or `cargo run --release`
//! - plugin: Herdr runs the same binary as the workbench pane command

fn main() -> std::io::Result<()> {
    corral::run()
}
