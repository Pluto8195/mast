//! Containment crate for the agent-only headless UI driver.
#![allow(clippy::disallowed_methods)] // test-only tool; spawns scratch helpers directly

pub mod environment;
pub mod materialize;
pub mod scenario;
pub mod serve;
pub mod session;
pub mod shape;
pub mod template;
pub mod tree;
pub mod world;

/// Reuse the engine's soak gate before assembling the in-process driver.
pub fn preflight() -> Result<(), String> {
    if !mast_harness::soak::enabled() {
        return Err("driver requires MAST_SOAK=1".to_owned());
    }
    mast_harness::soak::preflight()
}
