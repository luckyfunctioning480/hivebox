//! Sandbox cleanup and garbage collection.
//!
//! Handles tearing down all sandbox resources after a sandbox exits or is destroyed:
//! - Kill remaining processes in the cgroup
//! - Remove the cgroup directory
//! - Unmount and remove the sandbox filesystem
//!
//! Cleanup is designed to be idempotent — calling it multiple times on the same
//! sandbox is safe and won't cause errors.

use anyhow::Result;
use tracing::{info, warn};

use crate::sandbox::cgroup::CgroupManager;
use crate::sandbox::filesystem;

/// Performs full cleanup of a sandbox.
///
/// This is the main cleanup entry point. It handles:
/// 1. Killing all remaining processes in the sandbox's cgroup
/// 2. Removing the cgroup directory
/// 3. Unmounting and removing the sandbox filesystem
///
/// Called after the sandbox's main process exits (for one-shot) or when
/// explicitly destroyed (for persistent sandboxes in Phase 4).
pub fn cleanup_sandbox(sandbox_id: &str) -> Result<()> {
    info!(sandbox = sandbox_id, "starting cleanup");

    // Step 1: Kill all processes in the cgroup (if any are still running).
    // This handles cases where the main process exited but child processes are orphaned.
    match CgroupManager::create(sandbox_id) {
        Ok(cgroup) => {
            if let Err(e) = cgroup.kill_all() {
                warn!(
                    sandbox = sandbox_id,
                    error = %e,
                    "failed to kill cgroup processes (may already be dead)"
                );
            }

            // Give processes a moment to die after SIGKILL.
            // The kernel delivers SIGKILL asynchronously, so a brief wait ensures
            // the cgroup is empty before we try to remove it.
            std::thread::sleep(std::time::Duration::from_millis(100));

            // Step 2: Remove the cgroup directory.
            if let Err(e) = cgroup.cleanup() {
                warn!(
                    sandbox = sandbox_id,
                    error = %e,
                    "failed to remove cgroup (processes may still be dying)"
                );
            }
        }
        Err(e) => {
            warn!(
                sandbox = sandbox_id,
                error = %e,
                "could not access cgroup for cleanup (may already be removed)"
            );
        }
    }

    // Step 3: Unmount and remove the sandbox filesystem.
    if let Err(e) = filesystem::cleanup_rootfs(sandbox_id) {
        warn!(
            sandbox = sandbox_id,
            error = %e,
            "failed to clean up filesystem (may already be removed)"
        );
    }

    info!(sandbox = sandbox_id, "cleanup complete");
    Ok(())
}
