# Cross-platform SSH transport

Status: implemented; recursive paths are enabled by default in stable builds<br>
Implementation baseline: `2082841bb`, `1e6fdfaa4`, `363914f1d`, `beeaeb85b`<br>
Last updated: 2026-08-21<br>
Chinese version: [cross-platform-ssh-transport.zh-CN.md](cross-platform-ssh-transport.zh-CN.md)

## Goal and invariants

The goal is to extend InfiniShell shell integration and remote-server support
to Windows without regressing any SSH mode that previously worked on macOS,
Linux or Windows.

The implementation therefore follows four invariants:

1. Existing macOS/Linux OpenSSH and ControlMaster paths stay available.
2. Windows clients can enhance compatible SSH sessions to either POSIX or
   Windows remotes without depending on ControlMaster.
3. `ProxyJump` and process-style `ProxyCommand` configurations remain usable.
4. If InfiniShell cannot faithfully preserve an SSH option's behavior or
   security semantics, it hands the original arguments to native OpenSSH
   before authentication instead of silently ignoring the option.

## Capability matrix

| Client | POSIX remote (`bash` / `zsh`) | Windows remote (PowerShell) |
|---|---|---|
| macOS / Linux | Existing OpenSSH ControlMaster extension and POSIX bootstrap | Versioned PowerShell capability probe, PowerShell bootstrap and remote-server |
| Windows | Rust SSH worker, POSIX bootstrap and remote-server | Rust SSH worker, PowerShell bootstrap and Windows remote-server |

This matrix describes compatible interactive, single-destination sessions.
Native SSH remains available for every row when the enhanced path is not
applicable. A fallback can mean that the terminal connection works normally
while InfiniShell shell integration or remote-server features are unavailable
for that session.

## Recursive and multi-hop SSH

An enhanced remote shell can intercept another compatible interactive `ssh`
command and extend the new target through its parent remote-server. Each hop is
still created by OpenSSH on the machine where the user typed the command, so
that machine's DNS, `~/.ssh/config`, credentials and network reachability remain
authoritative. InfiniShell carries only a scoped, capability-protected control
reference through the parent daemon; it does not copy private keys or silently
enable agent forwarding.

The tunnel protocol uses per-stream byte credit, bounded frames, half-close and
reset semantics, parent-owned cancellation, hop-depth protection and safe
fallback. Repeating the same protocol supports `local -> A -> B -> C` rather
than treating the second hop as a special case. A failed install or extension
must leave the ordinary interactive shell usable.

This capability is enabled by default in stable builds; ordinary users do not
need to set an environment variable. Focused POSIX multi-hop, protocol,
flow-control and fallback checks are in place. Native Windows automation builds
the real SSH worker and verifies that PowerShell preserves bootstrap argument
boundaries, but it does not replace manual end-to-end coverage of
Windows-origin, Windows-remote and mixed-OS multi-hop topologies. Those Windows
paths remain the highest-risk release area and must pass the matrix below before
the stable tag is pushed.

## Selection and connection flow

### macOS and Linux clients

The existing shell wrappers continue to use OpenSSH ControlMaster. A first
probe reads `$SHELL`; if that result does not identify `bash` or `zsh`, a
fixed, versioned PowerShell capability probe runs over the already-authenticated
master connection. A valid Windows result selects the PowerShell bootstrap;
an unknown result continues as ordinary SSH.

### Windows clients

The PowerShell wrapper intercepts only an interactive command with one
destination and no configured `RemoteCommand`. `Warp-Test-IsWindows` detects
the local client platform; it does not attempt to classify the remote host.
On Windows it invokes the bundled `infinishell-ssh rust-ssh-session` worker.

The worker then:

1. asks the user's OpenSSH executable for the effective configuration with
   `ssh -G`;
2. audits every returned option and selects native fallback for unsupported
   non-neutral values;
3. connects through direct TCP, `ProxyCommand`, or a `ProxyJump` byte bridge;
4. verifies the host key and authenticates;
5. probes the remote shell on that target session;
6. starts the matching POSIX or PowerShell bootstrap and interactive PTY; and
7. exposes a loopback broker whose capability token authorizes additional exec
   channels on the same target session.

This avoids a second target connection and a second authentication prompt for
remote-server commands. A `ProxyJump` necessarily has its own connection to
the jump host, but the target session is still reused.

## Transport and remote-server details

The Windows package includes two Rust backends behind the same worker
protocol:

- the compatibility backend based on `ssh2`;
- the newer asynchronous `russh` backend, enabled by the compile-time
  `russh_transport` feature and the runtime `RusshTransport` feature flag.

Disabling `RusshTransport` retains the `ssh2` backend. The runtime flag is not
in a default rollout list as of this document, so it can be enabled in stages
without removing the established path.

Both enhanced backends cover the session behaviors required by InfiniShell:

- ordinary `known_hosts` verification and strict host-key prompts;
- OpenSSH agent authentication, including Unix sockets and Windows OpenSSH
  named pipes/Pageant where supported by the selected backend;
- ordered identity files, encrypted private keys, RSA signature selection,
  keyboard-interactive and password authentication;
- negotiated algorithms, with ML-KEM used when both configuration and backend
  support it;
- PTY allocation, environment propagation, terminal resize, escape handling,
  keepalives and compression; and
- the capability-protected loopback broker used by the remote-server command
  transport.

Windows remote-server artifacts are packaged separately from the client,
installed with PowerShell-aware paths and archive handling, and launched via a
Windows daemon/proxy implementation. POSIX remote-server behavior remains on
its existing path.

## Safe fallback boundary

The wrapper leaves non-interactive operations, forwarding/tunneling modes,
multiple destinations and explicit remote commands with native OpenSSH.
The worker also falls back before authentication when it encounters a setting
whose semantics it cannot preserve. Examples include:

- GSSAPI, host-based authentication, SSH certificates and security-key
  provider policies;
- agent/X11/port forwarding and shell-dependent `ProxyCommand` expressions;
- `@cert-authority` or `@revoked` entries in `known_hosts`;
- `UpdateHostKeys=yes`, `ObscureKeystrokeTiming=yes`,
  `StrictHostKeyChecking=no`, and other security-sensitive non-neutral values;
  or
- an algorithm configuration whose intersection with the selected backend is
  empty, such as SNTRUP-only key exchange with the current `russh` backend.

Fallback is allowed only before a prompt or successful authentication commits
the attempt. After that point, failure returns from the current attempt instead
of unexpectedly starting a second SSH connection. If remote-shell probing or
bootstrap is unavailable after authentication, the same target session can
continue as a plain interactive shell.

`UpdateHostKeys` proof requests and OpenSSH keystroke-obfuscation chaff require
protocol hooks that the current public `russh` API does not expose. These are
tracked as compatibility gaps, not approximated with weaker behavior.

## Regression contract

The following behaviors are release blockers if they regress:

- a connection that previously worked through native SSH must still connect;
- original SSH arguments and the user's resolved OpenSSH configuration must be
  preserved on fallback;
- `ProxyJump` and process-style `ProxyCommand` must keep working;
- enhanced target sessions must not cause a surprise second authentication
  prompt or second target connection;
- host-key mismatch or rejection must never be bypassed; and
- failure to install or start remote-server must not destroy an otherwise
  usable interactive SSH shell.

## Verification

Run the local gate before cross-platform testing:

```bash
cargo check -p warp --lib --locked
cargo check -p warp --bin infinishell-ssh --locked \
  --features rust_ssh_worker,russh_transport
cargo nextest run -p warp --locked \
  --features rust_ssh_worker,russh_transport \
  -E 'test(remote_server::rust_ssh)'
cargo fmt --all -- --check
```

On a Windows build host also verify remote-server packaging:

```powershell
pwsh -File script/windows/test_package_remote_server.ps1
```

Manual release coverage should include at least:

| Case | Required observation |
|---|---|
| Direct key authentication | One prompt at most; enhanced shell and broker work |
| Password / keyboard-interactive | Prompt is usable and is not repeated by a fallback |
| Changed host key | Connection is rejected |
| New host with strict `ask` | Explicit approval is required before learning the key |
| `ProxyJump` and process-style `ProxyCommand` | Target connects and enhanced commands reuse the session |
| POSIX and Windows remotes | Correct bootstrap and remote-server artifact are selected |
| POSIX `local -> A -> B` and `local -> A -> B -> C` | Every hop is enhanced and `exit` restores the parent context |
| Windows-origin, Windows-remote and mixed-OS multi-hop | Argument boundaries, bootstrap, install, nested shell and parent restore all work |
| Unsupported SSH option | Native SSH receives the original command and remains usable |
| Remote-server install failure | Interactive shell remains usable without enhancement |

Cross-platform cloud verification should use Linux and Windows runners after
the exact merge commit is available remotely. A local-only commit must not be
pushed solely to trigger verification without explicit authorization.

### Baseline verification record (2026-08-21)

Commit `beeaeb85b` passed the repository's focused local checks and the
[Linux x64 and Windows x64 preflight](https://github.com/Infinimesh-ai/InfiniShell-Desktop/actions/runs/32457514532).
The Windows job built the real SSH worker and passed the PowerShell worker
argument test in addition to the shared checks. Recursive tunnel tests cover
the byte window, upload compatibility and failure paths that previously caused
an extension-install attempt to report a false failure.

This is automated build and protocol evidence, not a claim that the Windows
runtime matrix is complete. Before the stable release, manually verify Windows
as the client, an intermediate hop and the final remote, including mixed
POSIX/Windows chains, cold installation, nested `exit`, parent disconnect and
native fallback. Do not publish a stable candidate with recursive SSH enabled
by default until that matrix passes.

## Post-merge observation log

Use the table below in a release issue or follow-up document. Keep counts by
client/remote pair so a healthy POSIX path cannot hide a Windows regression.

| Signal | Expected result | Follow-up threshold |
|---|---|---|
| Native SSH connection success | No regression from the previous release | Any confirmed regression blocks rollout |
| Time to usable prompt | Record median and p95 by matrix cell against the previous release | A repeatable material regression pauses rollout |
| Enhanced bootstrap activation | Succeeds for supported shell/config pairs | Repeated failures for one matrix cell require a fixture and test |
| Remote-server install/start | Succeeds with the matching OS/architecture artifact | Any systematic platform/architecture failure blocks rollout |
| Broker command execution | Uses the existing authenticated target session | Any second target login/prompt is a blocker |
| First broker command latency | Record cold-install and warm-session median/p95 separately | A repeatable material regression requires profiling before rollout |
| Safe native fallback | Plain SSH remains usable | Any dropped/rewritten argument is a blocker |
| Fallback rate and reason | Each fallback maps to an intentional compatibility boundary | A new or rising unexplained reason requires a fixture and test |
| ProxyJump/ProxyCommand | Direct and jump-host fixtures pass | Any previously working fixture failure is a blocker |
| Host-key handling | New/mismatch/revoked cases preserve policy | Any weakening is a security blocker |

InfiniShell does not add cloud telemetry for this transport. Collect timing and
fallback observations in controlled test fixtures, and track field behavior
through the verification matrix and sanitized issue reports. A useful report
contains:

- client OS and InfiniShell version;
- `ssh -V` output and remote OS/default shell;
- direct, `ProxyJump`, or `ProxyCommand` connection type;
- a sanitized `ssh -G <host>` result and the visible fallback message;
- whether the same command still works in a normal terminal; and
- whether the failure occurred before authentication, after authentication,
  during bootstrap, installation, or broker command execution.

Never attach private keys, passwords, capability tokens, a complete
`known_hosts` file, or unsanitized host/user/path/proxy endpoints.

## Code map

- PowerShell client wrapper:
  `app/assets/bundled/bootstrap/pwsh_ssh_wrapper.ps1`
- POSIX-to-Windows capability probe:
  `app/assets/bundled/bootstrap/ssh_remote_shell_probe.sh`
- Rust worker and configuration gate:
  `app/src/remote_server/rust_ssh.rs`
- Staged `russh` backend:
  `app/src/remote_server/rust_ssh/russh_backend.rs`
- Client remote-server transport:
  `app/src/remote_server/ssh_transport.rs`
- Windows daemon/proxy:
  `app/src/remote_server/windows/`
- Remote installation/platform setup:
  `crates/remote_server/src/setup.rs` and
  `crates/remote_server/src/setup/windows.rs`
- Windows packaging:
  `script/windows/package_remote_server.ps1` and
  `script/windows/bundle.ps1`
