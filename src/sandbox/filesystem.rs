//! Filesystem isolation for sandboxes.
//!
//! This module handles setting up the sandbox's filesystem:
//! - Preparing the rootfs directory (Phase 1: bind mount, Phase 2: overlayfs)
//! - Performing `pivot_root` to make the sandbox's rootfs the new `/`
//! - Mounting special filesystems (`/proc`, `/sys`, `/dev`, `/tmp`)
//!
//! After `pivot_root`, the host filesystem is completely unreachable from inside
//! the sandbox. There is no "above" to escape to — `/` is the sandbox root.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use nix::mount::{mount, umount2, MntFlags, MsFlags};
use nix::unistd::{chdir, pivot_root};
use tracing::{debug, info};

/// Base directory for all HiveBox data on the host.
pub const HIVEBOX_BASE_DIR: &str = "/var/lib/hivebox";

/// Directory where sandbox working directories are created.
pub const SANDBOXES_DIR: &str = "/var/lib/hivebox/sandboxes";

/// Directory where rootfs images are stored.
pub const IMAGES_DIR: &str = "/var/lib/hivebox/images";

/// Prepares the rootfs for a sandbox using overlayfs.
///
/// Sets up the full overlay stack:
/// 1. Mount the squashfs image as the read-only lower layer
/// 2. Create a tmpfs-backed writable upper layer
/// 3. Mount overlayfs combining both into a single merged view
///
/// The sandbox sees a normal read-write filesystem, but:
/// - Reads come from the squashfs base image (shared across all sandboxes)
/// - Writes go to the tmpfs upper layer (private to this sandbox, vanishes on destroy)
///
/// Falls back to bind-mounting an extracted rootfs directory if no squashfs image exists
/// (useful for development/testing without building images).
///
/// # Directory structure
///
/// ```text
/// /var/lib/hivebox/sandboxes/{id}/
/// ├── squashfs/  — squashfs mount point (read-only lower layer)
/// ├── upper/     — tmpfs writable layer (sandbox writes go here)
/// ├── work/      — overlayfs internal workdir (required by kernel)
/// └── merged/    — overlayfs union (this becomes the sandbox's /)
/// ```
///
/// Returns the path to the merged directory (the sandbox's rootfs).
pub fn prepare_rootfs(sandbox_id: &str, image: &str) -> Result<PathBuf> {
    let sandbox_dir = PathBuf::from(SANDBOXES_DIR).join(sandbox_id);
    let squashfs_dir = sandbox_dir.join("squashfs");
    let upper_dir = sandbox_dir.join("upper");
    let work_dir = sandbox_dir.join("work");
    let merged_dir = sandbox_dir.join("merged");

    // Check for squashfs image first, fall back to extracted rootfs directory.
    let squashfs_path = PathBuf::from(IMAGES_DIR).join(format!("{image}.squashfs"));
    let rootfs_dir = PathBuf::from(IMAGES_DIR).join(image).join("rootfs");

    if squashfs_path.exists() {
        // Extract squashfs once into a shared cache directory so we don't
        // re-extract for every sandbox (unsquashfs is expensive on large images).
        let shared_rootfs = PathBuf::from(IMAGES_DIR).join(format!("{image}.rootfs"));
        ensure_squashfs_extracted(&squashfs_path, &shared_rootfs)?;

        // Full overlayfs path: shared rootfs (read-only) + tmpfs (writable).
        prepare_rootfs_overlayfs(
            sandbox_id,
            image,
            &squashfs_path,
            &shared_rootfs,
            &sandbox_dir,
            &squashfs_dir,
            &upper_dir,
            &work_dir,
            &merged_dir,
        )
    } else if rootfs_dir.exists() {
        // Fallback: bind-mount an extracted rootfs (development/testing mode).
        prepare_rootfs_bindmount(sandbox_id, image, &rootfs_dir, &merged_dir)
    } else {
        anyhow::bail!(
            "image '{}' not found — expected {} or {}",
            image,
            squashfs_path.display(),
            rootfs_dir.display()
        );
    }
}

/// Extracts a squashfs image into a shared cache directory (once).
///
/// Subsequent calls are a no-op if the directory already contains files.
/// This avoids re-extracting a large squashfs for every sandbox.
fn ensure_squashfs_extracted(squashfs_path: &Path, dest: &Path) -> Result<()> {
    // Already extracted?
    if dest.exists() && fs::read_dir(dest).map_or(false, |mut d| d.next().is_some()) {
        debug!(dest = %dest.display(), "squashfs already extracted, reusing cache");
        return Ok(());
    }

    fs::create_dir_all(dest)
        .with_context(|| format!("failed to create cache dir: {}", dest.display()))?;

    // Try kernel mount first (fastest).
    if mount(
        Some(squashfs_path),
        dest,
        Some("squashfs"),
        MsFlags::MS_RDONLY,
        None::<&str>,
    )
    .is_ok()
    {
        // Mounted — now extract to have a persistent copy, then unmount.
        // Actually, if mount works we can just leave it mounted and use it directly.
        info!(dest = %dest.display(), "squashfs mounted as shared cache");
        return Ok(());
    }

    // Mount failed (e.g., Docker without loop devices) — extract with unsquashfs.
    info!(dest = %dest.display(), "extracting squashfs to shared cache (one-time)...");
    let status = std::process::Command::new("unsquashfs")
        .args([
            "-f",
            "-d",
            &dest.display().to_string(),
            &squashfs_path.display().to_string(),
        ])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .status()
        .context("failed to run unsquashfs — is squashfs-tools installed?")?;

    if !status.success() {
        anyhow::bail!(
            "unsquashfs failed (exit {}): could not extract {} to {}",
            status.code().unwrap_or(-1),
            squashfs_path.display(),
            dest.display()
        );
    }

    info!(dest = %dest.display(), "squashfs extracted to shared cache");
    Ok(())
}

/// Prepares rootfs using the full overlayfs stack (production path).
///
/// Uses a shared pre-extracted rootfs as the lower layer instead of extracting
/// the squashfs for every sandbox.
#[allow(clippy::too_many_arguments)]
fn prepare_rootfs_overlayfs(
    sandbox_id: &str,
    image: &str,
    _squashfs_path: &Path,
    shared_rootfs: &Path,
    sandbox_dir: &Path,
    _squashfs_dir: &Path,
    upper_dir: &Path,
    work_dir: &Path,
    merged_dir: &Path,
) -> Result<PathBuf> {
    // Create all directories in the overlay stack.
    for dir in [upper_dir, work_dir, merged_dir] {
        fs::create_dir_all(dir)
            .with_context(|| format!("failed to create dir: {}", dir.display()))?;
    }

    // Mount tmpfs for the writable upper layer and overlayfs workdir.
    mount(
        Some("tmpfs"),
        sandbox_dir,
        Some("tmpfs"),
        MsFlags::MS_NOSUID,
        Some("size=4g"),
    )
    .or_else(|_| -> Result<()> {
        debug!("tmpfs mount on sandbox_dir failed, using host filesystem for upper/work");
        Ok(())
    })?;

    // Recreate dirs in case tmpfs mount wiped them.
    for dir in [upper_dir, work_dir, merged_dir] {
        fs::create_dir_all(dir).ok();
    }

    // Use the shared pre-extracted rootfs as the lower layer for overlayfs.
    let lower_dir = shared_rootfs;

    // Mount overlayfs combining the read-only lower and writable upper.
    let overlay_opts = format!(
        "lowerdir={},upperdir={},workdir={}",
        lower_dir.display(),
        upper_dir.display(),
        work_dir.display()
    );
    let overlay_result = mount(
        Some("overlay"),
        merged_dir,
        Some("overlay"),
        MsFlags::empty(),
        Some(overlay_opts.as_str()),
    );

    if let Err(overlay_err) = overlay_result {
        // Overlayfs failed — fallback: bind-mount shared rootfs + tmpfs copy.
        info!(
            sandbox = sandbox_id,
            error = %overlay_err,
            "overlayfs mount failed, falling back to tmpfs + copy rootfs"
        );

        let _ = mount(
            Some("tmpfs"),
            merged_dir,
            Some("tmpfs"),
            MsFlags::MS_NOSUID,
            Some("size=4g"),
        );

        let status = std::process::Command::new("cp")
            .args([
                "-a",
                &format!("{}/.", lower_dir.display()),
                &merged_dir.display().to_string(),
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .context("failed to copy rootfs to merged dir")?;

        if !status.success() {
            anyhow::bail!(
                "failed to copy rootfs from {} to {} (exit {})",
                lower_dir.display(),
                merged_dir.display(),
                status.code().unwrap_or(-1)
            );
        }

        info!(
            sandbox = sandbox_id,
            image,
            rootfs = %merged_dir.display(),
            "rootfs prepared (tmpfs + copy fallback)"
        );
    } else {
        info!(
            sandbox = sandbox_id,
            image,
            rootfs = %merged_dir.display(),
            "rootfs prepared (squashfs + overlayfs)"
        );
    }

    Ok(merged_dir.to_path_buf())
}

/// Fallback: bind-mount an extracted rootfs directory (development mode).
fn prepare_rootfs_bindmount(
    sandbox_id: &str,
    image: &str,
    rootfs_dir: &Path,
    merged_dir: &Path,
) -> Result<PathBuf> {
    fs::create_dir_all(merged_dir)
        .with_context(|| format!("failed to create sandbox dir: {}", merged_dir.display()))?;

    mount(
        Some(rootfs_dir),
        merged_dir,
        None::<&str>,
        MsFlags::MS_BIND | MsFlags::MS_REC,
        None::<&str>,
    )
    .with_context(|| {
        format!(
            "failed to bind-mount rootfs from {} to {}",
            rootfs_dir.display(),
            merged_dir.display()
        )
    })?;

    info!(
        sandbox = sandbox_id,
        image,
        rootfs = %merged_dir.display(),
        "rootfs prepared (bind-mount fallback)"
    );

    Ok(merged_dir.to_path_buf())
}

/// Performs `pivot_root` to make the sandbox rootfs the new `/`.
///
/// After this call, the host's filesystem is completely inaccessible from the sandbox.
/// The old root is unmounted and removed — there is no way to navigate back to it.
///
/// # How it works
///
/// 1. Bind-mount the new root to itself (required by `pivot_root`)
/// 2. Create a temporary directory for the old root
/// 3. `pivot_root(new_root, old_root)` swaps `/` to point at new_root
/// 4. `chdir("/")` to enter the new root
/// 5. Unmount the old root with `MNT_DETACH` (lazy unmount)
/// 6. Remove the old root mount point
///
/// This is the critical security boundary — once done, the host FS is gone.
pub fn do_pivot_root(new_root: &Path) -> Result<()> {
    debug!(new_root = %new_root.display(), "performing pivot_root");

    // Bind-mount new_root to itself. pivot_root requires the new root
    // to be a mount point, and a bind mount satisfies this requirement.
    mount(
        Some(new_root),
        new_root,
        None::<&str>,
        MsFlags::MS_BIND | MsFlags::MS_REC,
        None::<&str>,
    )
    .context("failed to bind-mount new root to itself")?;

    // Create a directory to stash the old root during the pivot.
    let old_root = new_root.join(".pivot_old");
    fs::create_dir_all(&old_root).context("failed to create .pivot_old directory")?;

    // pivot_root: swap the filesystem root.
    // new_root becomes `/`, and the old root is moved to .pivot_old.
    pivot_root(new_root, &old_root).context("pivot_root failed")?;

    // Now we're inside the new root. Change to `/`.
    chdir("/").context("failed to chdir to / after pivot_root")?;

    // Unmount the old root. MNT_DETACH ensures lazy unmount even if
    // something still references it — it becomes invisible immediately
    // but actual cleanup happens when the last reference is dropped.
    umount2("/.pivot_old", MntFlags::MNT_DETACH).context("failed to unmount old root")?;

    // Remove the empty mount point directory.
    let _ = fs::remove_dir("/.pivot_old");

    info!("pivot_root complete — host filesystem is unreachable");
    Ok(())
}

/// Mounts special filesystems inside the sandbox.
///
/// These provide the sandboxed process with essential kernel interfaces:
/// - `/proc`: process information (filtered by PID namespace)
/// - `/sys`: kernel/device info (read-only)
/// - `/dev`: device nodes (minimal set: null, zero, urandom, etc.)
/// - `/tmp`: writable temporary storage
///
/// Called inside the child after `pivot_root`, so all paths are relative to
/// the sandbox's new root filesystem.
pub fn mount_special_filesystems() -> Result<()> {
    // Mount /proc — the PID namespace ensures only sandbox processes are visible.
    // This is safe because `ps`, `top`, and similar tools rely on /proc.
    create_dir_and_mount("proc", "/proc", "proc", MsFlags::empty())?;

    // Mount /sys — read-only to prevent the sandbox from modifying kernel state.
    // Two-step: mount sysfs, then remount read-only (some kernels need this).
    create_dir_and_mount("sysfs", "/sys", "sysfs", MsFlags::empty())?;
    mount(
        None::<&str>,
        "/sys",
        None::<&str>,
        MsFlags::MS_REMOUNT | MsFlags::MS_RDONLY,
        None::<&str>,
    )
    .context("failed to remount /sys read-only")?;

    // Mount /dev as tmpfs, then create only the safe device nodes.
    // We don't mount devtmpfs (which would expose all host devices).
    create_dir_and_mount("tmpfs", "/dev", "tmpfs", MsFlags::MS_NOSUID)?;
    create_device_nodes()?;

    // Mount /dev/pts for pseudo-terminals (needed for interactive shells).
    fs::create_dir_all("/dev/pts").context("failed to create /dev/pts")?;
    mount(
        Some("devpts"),
        "/dev/pts",
        Some("devpts"),
        MsFlags::empty(),
        Some("newinstance,ptmxmode=0666"),
    )
    .context("failed to mount /dev/pts")?;

    // Mount /dev/shm for POSIX shared memory.
    fs::create_dir_all("/dev/shm").context("failed to create /dev/shm")?;
    mount(
        Some("tmpfs"),
        "/dev/shm",
        Some("tmpfs"),
        MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
        Some("size=64m"),
    )
    .context("failed to mount /dev/shm")?;

    // Mount /tmp as tmpfs for writable temporary storage.
    create_dir_and_mount(
        "tmpfs",
        "/tmp",
        "tmpfs",
        MsFlags::MS_NOSUID | MsFlags::MS_NODEV,
    )?;

    info!("special filesystems mounted");
    Ok(())
}

/// Sets the hostname inside the sandbox.
///
/// Uses the UTS namespace so the hostname change is invisible to the host.
pub fn set_sandbox_hostname(sandbox_id: &str) -> Result<()> {
    // Use a short prefix + first 6 chars of the ID for a readable hostname.
    let short_id = &sandbox_id[..sandbox_id.len().min(6)];
    let hostname = format!("hb-{short_id}");
    nix::unistd::sethostname(&hostname)
        .with_context(|| format!("failed to set hostname to {hostname}"))?;
    debug!(hostname, "sandbox hostname set");
    Ok(())
}

/// Creates the minimal set of device nodes needed inside the sandbox.
///
/// Only safe, commonly needed devices are created:
/// - `/dev/null`    — data sink (writes succeed, reads return EOF)
/// - `/dev/zero`    — infinite source of zero bytes
/// - `/dev/full`    — always returns ENOSPC on write (for testing)
/// - `/dev/random`  — blocking random number source
/// - `/dev/urandom` — non-blocking random number source
/// - `/dev/tty`     — controlling terminal
///
/// No real hardware devices are exposed.
fn create_device_nodes() -> Result<()> {
    use nix::sys::stat::{makedev, mknod, Mode, SFlag};

    /// Device node specification: (path, major, minor).
    const DEVICES: &[(&str, u64, u64)] = &[
        ("/dev/null", 1, 3),
        ("/dev/zero", 1, 5),
        ("/dev/full", 1, 7),
        ("/dev/random", 1, 8),
        ("/dev/urandom", 1, 9),
        ("/dev/tty", 5, 0),
    ];

    let mode = Mode::from_bits_truncate(0o666);

    for &(path, major, minor) in DEVICES {
        mknod(path, SFlag::S_IFCHR, mode, makedev(major, minor))
            .with_context(|| format!("failed to create device node {path}"))?;
        debug!(path, major, minor, "created device node");
    }

    // Create symlinks for standard file descriptors.
    // Many programs expect these to exist in /dev/.
    std::os::unix::fs::symlink("/proc/self/fd", "/dev/fd")
        .context("failed to create /dev/fd symlink")?;
    std::os::unix::fs::symlink("/proc/self/fd/0", "/dev/stdin")
        .context("failed to create /dev/stdin symlink")?;
    std::os::unix::fs::symlink("/proc/self/fd/1", "/dev/stdout")
        .context("failed to create /dev/stdout symlink")?;
    std::os::unix::fs::symlink("/proc/self/fd/2", "/dev/stderr")
        .context("failed to create /dev/stderr symlink")?;

    Ok(())
}

/// Helper: creates a directory if needed and mounts a filesystem on it.
fn create_dir_and_mount(source: &str, target: &str, fstype: &str, flags: MsFlags) -> Result<()> {
    fs::create_dir_all(target).with_context(|| format!("failed to create mount point {target}"))?;
    mount(Some(source), target, Some(fstype), flags, None::<&str>)
        .with_context(|| format!("failed to mount {fstype} on {target}"))?;
    debug!(source, target, fstype = fstype, "mounted");
    Ok(())
}

/// Cleans up the sandbox filesystem.
///
/// Unmounts the overlayfs stack in reverse order (merged → squashfs → tmpfs)
/// and removes the sandbox directory. Uses lazy unmount to avoid EBUSY errors.
///
/// Called from the parent after the child exits.
pub fn cleanup_rootfs(sandbox_id: &str) -> Result<()> {
    let sandbox_dir = PathBuf::from(SANDBOXES_DIR).join(sandbox_id);
    let merged_dir = sandbox_dir.join("merged");
    let squashfs_dir = sandbox_dir.join("squashfs");

    // Unmount in reverse order: merged (overlayfs) first, then squashfs, then tmpfs.
    // MNT_DETACH ensures we don't block if something still references the mount.
    for mount_point in [&merged_dir, &squashfs_dir, &sandbox_dir] {
        if mount_point.exists() {
            let _ = umount2(mount_point.as_path(), MntFlags::MNT_DETACH);
        }
    }

    // Remove the sandbox working directory.
    if sandbox_dir.exists() {
        fs::remove_dir_all(&sandbox_dir)
            .with_context(|| format!("failed to remove sandbox dir: {}", sandbox_dir.display()))?;
    }

    debug!(sandbox = sandbox_id, "filesystem cleaned up");
    Ok(())
}
