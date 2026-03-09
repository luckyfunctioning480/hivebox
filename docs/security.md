# Security Model

HiveBox provides defense-in-depth through six independent security layers. Even if one layer is bypassed, the remaining layers continue to protect the host.

## Layer 1: Linux Namespaces

Every sandbox runs in six isolated namespaces:

| Namespace | Isolation provided |
|-----------|--------------------|
| **PID** | Sandbox can only see its own processes |
| **Mount** | Sandbox has its own filesystem view |
| **Network** | Sandbox gets an empty network stack |
| **User** | UID 0 inside maps to unprivileged user outside |
| **UTS** | Sandbox has its own hostname |
| **IPC** | Isolated shared memory and message queues |

**Key property**: User namespaces mean that even though the sandbox appears to run as root, it has zero privilege on the host. The mapping `0 → unprivileged_uid` means any escape attempt runs as a normal user.

## Layer 2: cgroup v2 Resource Limits

Every sandbox is placed in a dedicated cgroup with:

- **Memory**: hard limit with swap disabled (`memory.swap.max = 0`)
- **CPU**: fractional allocation via `cpu.max`
- **PIDs**: hard limit prevents fork bombs

The `cgroup.kill` feature (kernel 5.14+) provides atomic process group termination.

## Layer 3: pivot_root Filesystem Isolation

After mounting the sandbox's overlayfs, `pivot_root` replaces the filesystem root:

1. New root is bind-mounted to itself
2. `pivot_root(new_root, old_root)` swaps `/`
3. Old root is unmounted with `MNT_DETACH`
4. The old root mount point is removed

After this, there is no path from the sandbox to the host filesystem. Even `..` at `/` stays in the sandbox.

## Layer 4: Landlock LSM

Landlock restricts filesystem operations by path, providing a second layer of filesystem protection:

- Applied after `pivot_root` — restricts access within the sandbox root
- Even allowed syscalls (like `open()`) are constrained to permitted paths
- Gracefully degrades on older kernels (5.13+)

## Layer 5: Capability Dropping

All Linux capabilities are dropped except a minimal set:

**Kept**: `CAP_SETUID`, `CAP_SETGID`, `CAP_FOWNER`, `CAP_DAC_OVERRIDE`

**Dropped** (among others):
- `CAP_SYS_ADMIN` — no mount, no namespace operations
- `CAP_NET_RAW` — no raw sockets
- `CAP_SYS_PTRACE` — no process inspection
- `CAP_SYS_MODULE` — no kernel module loading
- `CAP_SYS_BOOT` — no reboot

`PR_SET_NO_NEW_PRIVS` is also set, preventing privilege escalation through `execve`.

## Layer 6: seccomp-BPF

A BPF filter restricts which system calls the sandbox can make:

### Default Profile (~80 syscalls)
Allows common operations: file I/O, networking, process management, memory, signals, time.

**Blocked** (examples):
- `mount`, `umount2` — no filesystem manipulation
- `reboot`, `kexec_load` — no system control
- `init_module`, `finit_module` — no kernel modules
- `ptrace` — no process debugging
- `bpf` — no BPF program loading
- `userfaultfd` — no userfaultfd (common exploit primitive)
- `keyctl` — no kernel keyring access
- `personality` — no execution domain changes

### Strict Profile (~40 syscalls)
For pure computation workloads. No networking, no process creation beyond initial exec.

## Security Application Order

The order matters — each step requires capabilities that the next step removes:

```
1. clone() with CLONE_NEW* flags
2. Parent writes UID/GID maps
3. Child: mount filesystems          ← needs CAP_SYS_ADMIN
4. Child: pivot_root                 ← needs mount capability
5. Child: mount /proc, /dev, etc.    ← needs mount capability
6. Child: apply Landlock             ← restricts filesystem paths
7. Child: drop capabilities          ← removes mount capability
8. Child: set NO_NEW_PRIVS           ← prevents privilege escalation
9. Child: install seccomp filter     ← requires NO_NEW_PRIVS
10. Child: execve(user_command)      ← runs with minimal privileges
```

## Threat Model

### What HiveBox protects against

- **Code execution escape**: untrusted code running inside a sandbox cannot access host resources
- **Resource exhaustion**: cgroup limits prevent CPU/memory/PID exhaustion
- **Network attacks**: default "none" mode provides complete network isolation
- **Fork bombs**: PID limit in cgroup stops runaway process creation
- **Filesystem escape**: `pivot_root` + Landlock prevent accessing host files
- **Privilege escalation**: user namespaces + capability dropping + NO_NEW_PRIVS
- **Dangerous syscalls**: seccomp blocks mount, reboot, module loading, etc.

### Limitations

- **Kernel vulnerabilities**: sandbox relies on kernel primitives — a kernel bug could bypass isolation
- **Privileged Docker**: when running HiveBox inside Docker with `--privileged`, the outer container provides weaker isolation than native
- **Time-of-check-to-time-of-use**: file operations between the host and sandbox could theoretically race, though `pivot_root` mitigates this
- **Side channels**: CPU cache timing and similar microarchitectural attacks are not mitigated

### Recommendations

1. Keep the kernel updated (5.15+ recommended)
2. Use the strict seccomp profile for untrusted code
3. Use "none" network mode unless internet access is needed
4. Set conservative memory and PID limits
5. Set short timeouts (don't leave sandboxes running indefinitely)
6. Monitor host resources for anomalies
