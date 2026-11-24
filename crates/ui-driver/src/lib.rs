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
    mast_harness::soak::preflight()?;
    // A timeout or a disconnected host does not prove that a sandbox denied
    // network access. Require the explicit denial that startup_guard checks.
    let target = std::net::SocketAddr::from(([192, 0, 2, 1], 443));
    match std::net::TcpStream::connect_timeout(&target, std::time::Duration::from_secs(1)) {
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => Ok(()),
        _ => Err("driver requires an explicit sandbox network denial".into()),
    }
}
