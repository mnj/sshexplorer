# sshexplorer resolves OpenSSH config itself over an in-process SSH stack

A Session must be able to ask for a password or a key passphrase and keep it with Remember, and it must also honour the operator's OpenSSH config and SSH agent. No maintained Rust stack does both: the process-backed `openssh` crate applies the real config but refuses any authentication that reads stdin. So sshexplorer connects through an in-process stack (russh is the lean; ssh2 or libssh-rs also qualify) and owns config resolution: it reads `~/.ssh/config` and its `Include`s, matches the Host's address against `Host` patterns, and applies a fixed set of keywords (`HostName`, `User`, `Port`, `IdentityFile`, `IdentitiesOnly`, `IdentityAgent`, `UserKnownHostsFile`, `ConnectTimeout`, `ServerAliveInterval`).

## Considered Options

- **The `openssh` crate (spawn the system `ssh`):** full config and agent behaviour for free, but no password or passphrase prompt, so Remember cannot work, and it is Unix-only.
- **A stack's own config parser (russh-config, ssh2-config, libssh's):** each parses a different subset and none applies it, so behaviour would depend on the stack.

## Consequences

- Keywords outside the set, `Match` blocks, `ProxyJump` and `ProxyCommand` are not honoured; a Session whose config needs a proxy fails and names the keyword. Adding routing later means implementing it here, not switching stacks.
- Host-key checking against `known_hosts` is sshexplorer's code too.
