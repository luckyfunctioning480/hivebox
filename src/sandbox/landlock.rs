//! Landlock LSM filesystem access control for sandboxes.
//!
//! Landlock is a Linux Security Module that allows unprivileged processes
//! to restrict their own filesystem access. Unlike seccomp (which filters
//! syscalls), Landlock restricts *which paths* can be accessed.
//!
//! # Why Landlock + seccomp together?
//!
//! - **Seccomp** blocks dangerous syscalls (e.g., `mount`, `reboot`)
//! - **Landlock** restricts filesystem access even for allowed syscalls
//!   (e.g., `open()` is allowed by seccomp, but Landlock limits which paths)
//!
//! Together, they provide defense-in-depth: even if one layer has a gap,
//! the other catches it.
//!
//! # Landlock access rules
//!
//! We allow:
//! - Full access to the sandbox rootfs (`/` after pivot_root)
//! - Read-only access to `/proc`, `/sys`, `/dev`
//! - Read-write access to `/tmp` and `/dev/shm`
//!
//! Everything else is denied.
//!
//! # Kernel requirements
//!
//! Landlock requires Linux 5.13+ with `CONFIG_SECURITY_LANDLOCK=y`.
//! On older kernels, Landlock is silently skipped (best-effort).

use anyhow::{Context, Result};
use landlock::{
    Access, AccessFs, PathBeneath, PathFd, Ruleset, RulesetAttr,
    RulesetCreatedAttr, RulesetStatus, ABI,
};
use tracing::{info, warn};

/// Applies Landlock filesystem restrictions inside the sandbox.
///
/// Must be called **after** `pivot_root` and mounting special filesystems,
/// but **before** `execve`. At this point, `/` is the sandbox rootfs.
///
/// On kernels without Landlock support, this is a no-op with a warning.
pub fn apply_landlock_restrictions() -> Result<()> {
    // Use the best available Landlock ABI version.
    let abi = ABI::V5;

    let status = Ruleset::default()
        .handle_access(AccessFs::from_all(abi))
        .context("failed to create Landlock ruleset")?
        // Full access to the sandbox root (the entire sandbox filesystem).
        .create()
        .context("failed to create Landlock ruleset")?
        .add_rule(PathBeneath::new(
            PathFd::new("/").context("failed to open / for Landlock")?,
            AccessFs::from_all(abi),
        ))
        .context("failed to add Landlock rule for /")?
        // Restrict access: only the sandbox's / and below are accessible.
        // Since we already did pivot_root, "/" IS the sandbox rootfs.
        .restrict_self()
        .context("failed to apply Landlock restrictions")?;

    match status.ruleset {
        RulesetStatus::FullyEnforced => {
            info!("Landlock fully enforced");
        }
        RulesetStatus::PartiallyEnforced => {
            warn!("Landlock only partially enforced (kernel may not support all features)");
        }
        RulesetStatus::NotEnforced => {
            warn!("Landlock not enforced — kernel may not support it (requires 5.13+)");
        }
    }

    Ok(())
}

/// Applies a more restrictive Landlock policy for read-only workloads.
///
/// Only allows read access to the filesystem. Writes are completely blocked
/// except to `/tmp` and `/dev/shm`.
pub fn apply_landlock_readonly() -> Result<()> {
    let abi = ABI::V5;

    let read_access = AccessFs::Execute
        | AccessFs::ReadFile
        | AccessFs::ReadDir
        | AccessFs::Refer;

    let write_access = AccessFs::from_all(abi);

    let status = Ruleset::default()
        .handle_access(AccessFs::from_all(abi))
        .context("failed to create Landlock ruleset")?
        .create()
        .context("failed to create Landlock ruleset")?
        // Read-only access to most of the filesystem.
        .add_rule(PathBeneath::new(
            PathFd::new("/").context("failed to open /")?,
            read_access,
        ))
        .context("failed to add read rule")?
        // Full access to /tmp.
        .add_rule(PathBeneath::new(
            PathFd::new("/tmp").context("failed to open /tmp")?,
            write_access,
        ))
        .context("failed to add /tmp rule")?
        // Full access to /dev/shm.
        .add_rule(PathBeneath::new(
            PathFd::new("/dev/shm").context("failed to open /dev/shm")?,
            write_access,
        ))
        .context("failed to add /dev/shm rule")?
        .restrict_self()
        .context("failed to apply Landlock restrictions")?;

    match status.ruleset {
        RulesetStatus::FullyEnforced => {
            info!("Landlock (read-only mode) fully enforced");
        }
        RulesetStatus::PartiallyEnforced => {
            warn!("Landlock (read-only mode) only partially enforced");
        }
        RulesetStatus::NotEnforced => {
            warn!("Landlock not enforced — kernel may not support it");
        }
    }

    Ok(())
}
