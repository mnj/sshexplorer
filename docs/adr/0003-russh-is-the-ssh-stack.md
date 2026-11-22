# russh is the SSH stack

ADR-0002 leaves the in-process SSH stack to the implementer. sshexplorer uses russh 0.65 with russh-sftp 4 for listing (issue #4, ticket #22). Both are pure Rust apart from the crypto backend (aws-lc-rs by default), work on Linux, macOS and Windows, can ask for a password, passphrase or keyboard-interactive answer through ordinary async calls, talk to an SSH agent, and let one connection carry SFTP and `exec` (for Search) side by side. russh is tokio-only, so sshexplorer runs its own tokio runtime on background threads and the egui frame thread only polls channels.

## Considered Options

- **ssh2 (libssh2):** blocking, needs libssh2 and OpenSSL on every OS, and its agent and keyboard-interactive support is thinner.
- **libssh-rs (libssh):** blocking, needs libssh and OpenSSL, and has fewer users from Rust.

## Consequences

- russh's own `known_hosts` helpers are not used: they miscount lines after comments and lack markers, wildcards and tabs. sshexplorer parses `known_hosts` itself (`src/known_hosts.rs`), as ADR-0002 already requires.
- russh pins an exact `ssh-key` release candidate, so sshexplorer reaches it through `russh::keys::ssh_key` and does not depend on `ssh-key` directly.
- The test server is russh's own server with a russh-sftp server handler, in process (`tests/common/fixture.rs`), so SFTP details such as `longname` are the fixture's, not OpenSSH's.
