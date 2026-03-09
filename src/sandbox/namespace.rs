//! Linux namespace management for sandbox isolation.
//!
//! This module handles the creation of isolated namespaces using the Linux `clone` syscall.
//! Each sandbox gets its own PID, mount, network, user, UTS, and IPC namespaces, providing
//! six-layer isolation from the host system.
//!
//! # Namespace types used
//!
//! - **PID** (`CLONE_NEWPID`): sandbox sees only its own processes, with PID 1 as init
//! - **Mount** (`CLONE_NEWNS`): sandbox has its own mount table, host mounts are invisible
//! - **Network** (`CLONE_NEWNET`): sandbox gets an empty network stack
//! - **User** (`CLONE_NEWUSER`): UID 0 inside maps to an unprivileged user outside
//! - **UTS** (`CLONE_NEWUTS`): sandbox has its own hostname
//! - **IPC** (`CLONE_NEWIPC`): sandbox has its own System V IPC / POSIX message queues

use std::io::{Read, Write};
use std::os::unix::io::{AsRawFd, FromRawFd, IntoRawFd};

use anyhow::{Context, Result};
use nix::sched::{clone, CloneFlags};
use nix::sys::signal::Signal;
use nix::unistd::{getgid, getuid, Pid};
use tracing::{debug, info};

/// Size of the stack allocated for the cloned child process (8 MiB).
const CHILD_STACK_SIZE: usize = 8 * 1024 * 1024;

/// All namespace flags combined. This provides full isolation for the sandbox process.
pub const CLONE_ALL_NS: CloneFlags = CloneFlags::from_bits_truncate(
    CloneFlags::CLONE_NEWPID.bits()
        | CloneFlags::CLONE_NEWNS.bits()
        | CloneFlags::CLONE_NEWNET.bits()
        | CloneFlags::CLONE_NEWUSER.bits()
        | CloneFlags::CLONE_NEWUTS.bits()
        | CloneFlags::CLONE_NEWIPC.bits(),
);

/// Spawns a child process inside fresh namespaces.
///
/// The child runs `child_fn` after waiting for the parent to configure UID/GID mappings.
/// Returns the child's PID as seen from the host.
///
/// # Synchronization
///
/// A pipe is used to synchronize parent and child:
/// 1. Parent calls `clone()`, child is created and blocks reading the pipe
/// 2. Parent writes UID/GID maps to `/proc/{child}/uid_map` and `/proc/{child}/gid_map`
/// 3. Parent writes a byte to the pipe, unblocking the child
/// 4. Child proceeds with filesystem setup and exec
pub fn spawn_in_namespace<F>(child_fn: F) -> Result<Pid>
where
    F: FnMut() -> isize,
{
    // Create a pipe for parent -> child synchronization.
    // The child waits on the read end until the parent has set up UID/GID mappings.
    // nix 0.29+ returns OwnedFd, so we convert to raw FDs for use across clone().
    let (pipe_read_fd, pipe_write_fd) = nix::unistd::pipe()
        .context("failed to create synchronization pipe")?;
    let pipe_read_raw = pipe_read_fd.as_raw_fd();
    let pipe_write_raw = pipe_write_fd.as_raw_fd();

    // Leak the OwnedFds so they don't get closed when dropped — we'll manage
    // their lifetimes manually across the parent/child boundary.
    let _ = pipe_read_fd.into_raw_fd();
    let _ = pipe_write_fd.into_raw_fd();

    // Allocate stack for the cloned child process.
    // clone() requires a manually allocated stack because the child starts in a new PID namespace
    // and cannot share the parent's stack.
    let mut stack = vec![0u8; CHILD_STACK_SIZE];

    // Wrap the user's closure to include the pipe synchronization.
    // The child will block until the parent signals that UID/GID maps are ready.
    let mut child_fn = child_fn;
    let cb = Box::new(move || -> isize {
        // Close the write end in the child — we only read from it.
        unsafe { libc::close(pipe_write_raw) };

        // Block until parent signals that UID/GID maps are configured.
        let mut buf = [0u8; 1];
        let mut pipe_reader = unsafe { std::fs::File::from_raw_fd(pipe_read_raw) };
        if pipe_reader.read_exact(&mut buf).is_err() {
            eprintln!("hivebox: failed to read sync pipe from parent");
            return -1;
        }
        drop(pipe_reader);

        // Now UID/GID maps are set. Proceed with the child's work.
        child_fn()
    });

    // Clone the child process into fresh namespaces.
    // SIGCHLD ensures the parent gets notified when the child exits.
    // SAFETY: clone() is called with a heap-allocated stack and a boxed closure.
    // The child process gets its own address space (due to CLONE_NEWPID + CLONE_NEWNS),
    // so sharing the parent's memory is safe. The stack is large enough (8 MiB).
    let child_pid = unsafe {
        clone(
            cb,
            &mut stack,
            CLONE_ALL_NS,
            Some(Signal::SIGCHLD as i32),
        )
    }
    .context("clone() failed — are user namespaces enabled in the kernel?")?;

    info!(pid = child_pid.as_raw(), "spawned child in new namespaces");

    // Close the read end in the parent — we only write to it.
    unsafe { libc::close(pipe_read_raw) };

    // Set up UID/GID mappings so the child appears as root inside the namespace
    // but runs as our unprivileged user on the host.
    setup_uid_gid_maps(child_pid)
        .context("failed to configure UID/GID mappings")?;

    // Signal the child to proceed.
    let mut pipe_writer = unsafe { std::fs::File::from_raw_fd(pipe_write_raw) };
    pipe_writer
        .write_all(&[1u8])
        .context("failed to signal child via sync pipe")?;
    drop(pipe_writer);

    debug!(pid = child_pid.as_raw(), "child signaled to proceed");

    Ok(child_pid)
}

/// Configures UID/GID mappings for a process in a new user namespace.
///
/// Maps UID 0 inside the namespace to the calling user's UID outside.
/// This means the sandboxed process thinks it's running as root, but on the host
/// it's running as an unprivileged user — a key safety property.
///
/// # Kernel requirement
///
/// `/proc/{pid}/setgroups` must be written to "deny" before writing `gid_map`.
/// This is a kernel security requirement to prevent privilege escalation.
fn setup_uid_gid_maps(child_pid: Pid) -> Result<()> {
    let pid = child_pid.as_raw();
    let uid = getuid();
    let gid = getgid();

    debug!(
        pid,
        host_uid = uid.as_raw(),
        host_gid = gid.as_raw(),
        "setting up UID/GID maps"
    );

    // Deny setgroups — required before writing gid_map.
    // Without this, the kernel rejects gid_map writes as a security precaution.
    std::fs::write(format!("/proc/{pid}/setgroups"), "deny")
        .context("failed to write /proc/{pid}/setgroups")?;

    // Map UID 0 inside the namespace to our real UID outside.
    // Format: "inside_uid outside_uid count"
    std::fs::write(
        format!("/proc/{pid}/uid_map"),
        format!("0 {} 1\n", uid.as_raw()),
    )
    .context("failed to write uid_map")?;

    // Map GID 0 inside the namespace to our real GID outside.
    std::fs::write(
        format!("/proc/{pid}/gid_map"),
        format!("0 {} 1\n", gid.as_raw()),
    )
    .context("failed to write gid_map")?;

    info!(pid, "UID/GID maps configured: root inside -> unprivileged outside");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_clone_flags_combined() {
        // Verify all expected namespace flags are included.
        assert!(CLONE_ALL_NS.contains(CloneFlags::CLONE_NEWPID));
        assert!(CLONE_ALL_NS.contains(CloneFlags::CLONE_NEWNS));
        assert!(CLONE_ALL_NS.contains(CloneFlags::CLONE_NEWNET));
        assert!(CLONE_ALL_NS.contains(CloneFlags::CLONE_NEWUSER));
        assert!(CLONE_ALL_NS.contains(CloneFlags::CLONE_NEWUTS));
        assert!(CLONE_ALL_NS.contains(CloneFlags::CLONE_NEWIPC));
    }
}
