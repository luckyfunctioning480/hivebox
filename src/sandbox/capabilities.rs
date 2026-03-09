//! Linux capability management for sandboxes.
//!
//! Linux capabilities split the monolithic "root" privilege into fine-grained
//! permissions. Even though the sandbox runs as UID 0 inside the user namespace,
//! we drop all capabilities except the minimal set needed.
//!
//! # Why drop capabilities?
//!
//! Even with user namespaces (where "root" is actually unprivileged on the host),
//! some capabilities inside the namespace can be used for mischief:
//! - `CAP_NET_RAW` → raw sockets (could be used for network attacks)
//! - `CAP_SYS_ADMIN` → mount, namespace operations
//! - `CAP_SYS_PTRACE` → inspect other processes
//!
//! By dropping everything, we ensure the sandbox has the minimum privileges needed.
//!
//! # PR_SET_NO_NEW_PRIVS
//!
//! We also set the `PR_SET_NO_NEW_PRIVS` flag, which:
//! - Prevents `execve` from granting new privileges (no setuid/setgid)
//! - Is required for unprivileged seccomp filter installation
//! - Is inherited across `fork`/`clone`/`execve` and cannot be unset

use anyhow::{Context, Result};
use caps::{CapSet, Capability};
use tracing::{debug, info};

/// The minimal set of capabilities to keep inside the sandbox.
///
/// - `CAP_SETUID`/`CAP_SETGID`: needed for some programs that switch users
/// - `CAP_FOWNER`: needed to manipulate files regardless of ownership
/// - `CAP_DAC_OVERRIDE`: needed for some package managers to access files
///
/// These are safe to keep because:
/// - The user namespace maps UID 0 inside to an unprivileged user outside
/// - `PR_SET_NO_NEW_PRIVS` prevents privilege escalation through exec
/// - seccomp blocks dangerous syscalls that these caps would normally enable
const KEEP_CAPS: &[Capability] = &[
    Capability::CAP_SETUID,
    Capability::CAP_SETGID,
    Capability::CAP_FOWNER,
    Capability::CAP_DAC_OVERRIDE,
];

/// Drops all capabilities except the minimal set, and sets NO_NEW_PRIVS.
///
/// Must be called **after** all filesystem and mount operations (which may
/// need capabilities) but **before** seccomp filter installation (which
/// requires NO_NEW_PRIVS).
///
/// # Security order
///
/// 1. Landlock (filesystem restrictions)
/// 2. **Drop capabilities** ← this function
/// 3. Set NO_NEW_PRIVS ← this function
/// 4. seccomp (syscall filtering, requires NO_NEW_PRIVS)
pub fn drop_capabilities() -> Result<()> {
    // Set PR_SET_NO_NEW_PRIVS first.
    // This flag ensures that execve never grants new privileges (no setuid escalation).
    // It's also a prerequisite for unprivileged seccomp filter installation.
    set_no_new_privs()
        .context("failed to set PR_SET_NO_NEW_PRIVS")?;

    // Get all possible capabilities.
    let all_caps = caps::all();

    // Drop capabilities not in our keep-list from all sets:
    // - Effective: currently active capabilities
    // - Permitted: maximum capabilities that can be made effective
    // - Inheritable: capabilities passed across execve
    // - Ambient: capabilities automatically granted after execve
    //            (even without file caps)

    for &cap in all_caps.iter() {
        if KEEP_CAPS.contains(&cap) {
            continue;
        }

        // Drop from all capability sets.
        for &set in &[CapSet::Effective, CapSet::Permitted, CapSet::Inheritable] {
            if caps::has_cap(None, set, cap).unwrap_or(false) {
                caps::drop(None, set, cap)
                    .with_context(|| format!("failed to drop {:?} from {:?}", cap, set))?;
            }
        }

        // Ambient capabilities need a different call.
        if caps::has_cap(None, CapSet::Ambient, cap).unwrap_or(false) {
            let _ = caps::drop(None, CapSet::Ambient, cap);
        }
    }

    debug!(
        kept = ?KEEP_CAPS,
        "capabilities dropped to minimum set"
    );

    info!("capabilities dropped, NO_NEW_PRIVS set");
    Ok(())
}

/// Sets the `PR_SET_NO_NEW_PRIVS` flag on the current process.
///
/// After this call:
/// - `execve` of setuid/setgid binaries will not grant elevated privileges
/// - Unprivileged seccomp filter installation becomes possible
/// - The flag is inherited by all children and cannot be unset
fn set_no_new_privs() -> Result<()> {
    // prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0)
    let ret = unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
    if ret != 0 {
        anyhow::bail!(
            "prctl(PR_SET_NO_NEW_PRIVS) failed: {}",
            std::io::Error::last_os_error()
        );
    }
    debug!("PR_SET_NO_NEW_PRIVS set");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_keep_caps_not_empty() {
        assert!(!KEEP_CAPS.is_empty());
    }
}
