# Which Rust SSH stacks can list a remote filesystem?

Research date: 2026-11-07. Versions are the newest crates.io release on that date. This note does not choose a stack for sshexplorer.

Question, from [issue 4](https://github.com/mnj/sshexplorer/issues/4): which Rust SSH library, or Rust binding to a maintained C library, can do all of the following on one connection?

- Authenticate with a password.
- Authenticate with a key file, including a key that needs a passphrase.
- Authenticate with an SSH agent.
- Apply the operator's OpenSSH client config.
- List a remote directory.
- Run a remote name search equivalent to `find` under a directory.

"One connection" here means one authenticated session can both list a directory and run that search without opening a second SSH session. A directory listing is one directory. None of the libraries below documents an SFTP call that searches names recursively. The search is a remote command on a second channel (or a multiplexed `ssh` command) of the same session.

Checked against their own docs: russh, ssh2 (libssh2), libssh-rs (libssh), and the process-backed `openssh` crate. sunset and thrussh were opened and dropped because their own docs show they cannot cover the checklist. This is not a survey of every SSH crate on crates.io.

## Answer

No maintained stack in this set does every item with full OpenSSH client-config fidelity.

| Stack | Password | Key file with passphrase | Agent | OpenSSH client config | List one directory | Remote command on that same connection |
| --- | --- | --- | --- | --- | --- | --- |
| russh 0.65.1 + russh-sftp 3.0.1 | Yes | Yes, `load_secret_key` | Yes | Partial, separate crate, not applied for you | Yes, SFTP | Yes, `exec` |
| ssh2 0.9.7 (libssh2) | Yes | Yes; Rust docs say PEM | Yes, first agent key, or the agent API | Partial, separate crate, not applied for you | Yes, SFTP | Yes, `Channel::exec` |
| libssh-rs 0.3.8 (libssh) | Yes; docs prefer keyboard-interactive | Yes | Yes | Broadest in-process parser; not full `ssh_config(5)` | Yes, SFTP | Yes, `request_exec` |
| openssh 0.11.6 (system `ssh`) | No, if `ssh` must read stdin | No, if the key makes `ssh` read stdin | Yes, because `ssh` does | Yes, because `ssh` does | Yes, SFTP subsystem | Yes, multiplexed command |

russh, ssh2, and libssh-rs can authenticate all three ways and can list plus run a command on one session. Their config support is a subset, and for russh and ssh2 the application copies parsed fields onto the session. The `openssh` crate applies the operator's real OpenSSH config and agent, and can list and run a command on one multiplexed connection, and its own docs exclude password authentication and any authentication that reads stdin, including a locked key passphrase.

All four can run that work off the egui frame thread. None of them is an egui integration. ssh2 and libssh-rs block the calling thread and serialize one session. See [Beside an egui frame loop](#beside-an-egui-frame-loop).

## russh 0.65.1

Pure Rust client and server. crates.io: 0.65.1 published 2026-11-03, Apache-2.0, repository `https://github.com/warp-tech/russh`, `rust-version` field `1.89`. Default features include `aws-lc-rs`; `ring` is a separate feature. [crates.io russh](https://crates.io/api/v1/crates/russh)

### What it can do

- Password: `Handle::authenticate_password`. [client/mod.rs, russh 0.65.1](https://docs.rs/russh/0.65.1/src/russh/client/mod.rs.html)
- Key file, including a passphrase: `keys::load_secret_key(path, Option<&str>)` is documented as "Load a secret key, deciphering it with the supplied password if necessary." [load_secret_key](https://docs.rs/russh/0.65.1/russh/keys/fn.load_secret_key.html)
- Agent: `authenticate_publickey_with` takes a `Signer`. The same source says the crate's signer implementation is the SSH agent client. If the signer fails, the error is returned, the attempt is abandoned, and "the handle stays usable" for another method. [client/mod.rs](https://docs.rs/russh/0.65.1/src/russh/client/mod.rs.html)
- Unix agent: `AgentClient::connect_env` reads `SSH_AUTH_SOCK` and returns `Error::EnvVar("SSH_AUTH_SOCK")` when it is missing. A missing socket becomes `Error::BadAuthSock`. `connect_uds` takes a path. [keys/agent/client.rs](https://docs.rs/russh/0.65.1/src/russh/keys/agent/client.rs.html)
- Windows agent: `connect_pageant` and `connect_named_pipe` are `#[cfg(windows)]` on that same file.

### What it cannot do

russh does not parse or apply OpenSSH client config. The companion crate russh-config 0.58.0 (published 2026-03-18, Apache-2.0, same repository) parses a small subset and does not apply it. [crates.io russh-config](https://crates.io/api/v1/crates/russh-config)

`parse_home` reads only `~/.ssh/config`. It does not read `/etc/ssh/ssh_config`. The keyword match arms in 0.58.0 are `user`, `hostname`, `port` (a parse failure is ignored), `identityfile`, `proxycommand`, `proxyjump`, `addkeystoagent`, `userknownhostsfile`, and `stricthostkeychecking`. Any other key is debug-logged and ignored, so `Match`, `Include`, `IdentityAgent`, `IdentitiesOnly`, `PreferredAuthentications`, and cipher or kex lines are not applied. `StrictHostKeyChecking` is false only for the literal `no`; every other value, including `accept-new`, becomes true. `expand_tokens` replaces `%u`, `%h`, `%H`, `%p`, and `%%` only. The `%p` arm's comment says "original typed hostname" while the code inserts the port number. `Config::stream` uses `ProxyCommand` (split on spaces) or a direct TCP connect. It stores `proxy_jump` and does not use it to connect. [russh-config 0.58.0 lib.rs](https://docs.rs/russh-config/0.58.0/src/russh_config/lib.rs.html)

Parsed `IdentityFile` values are not loaded for you. The application loads them and calls the auth methods.

### Directory listing and remote commands

`Handle::channel_open_session` returns when the connection is authenticated; the channel is usable once the server confirms it. [client/mod.rs](https://docs.rs/russh/0.65.1/src/russh/client/mod.rs.html)

`Channel::exec` "execute[s] a remote program (will be passed to a shell)." `Channel::request_subsystem` starts a named subsystem. `Channel::into_stream` turns the channel into `AsyncRead` + `AsyncWrite` of `ChannelMsg::Data`. [Channel, russh 0.65.1](https://docs.rs/russh/0.65.1/russh/struct.Channel.html)

Directory listing is the separate crate russh-sftp 3.0.1 (published 2026-09-28, Apache-2.0, repository `https://github.com/AspectUnk/russh-sftp`). [crates.io russh-sftp](https://crates.io/api/v1/crates/russh-sftp). Its `ReadDir` iterator skips entries named `.` and `..`. `DirEntry::path` joins with `/` and does not canonicalize. [dir.rs, russh-sftp 3.0.1](https://docs.rs/russh-sftp/3.0.1/src/russh_sftp/client/fs/dir.rs.html)

A name search is a second channel: `channel_open_session` then `exec` of a remote command. That is the same connection, not a second login.

### Host keys

`Handler::check_server_key` must be implemented. "The default implementation rejects all keys." The library verifies the key-exchange signature before that call; the application decides trust. For the first key exchange, `kex_done` runs only after `check_server_key` has accepted the server. [Handler](https://docs.rs/russh/0.65.1/russh/client/trait.Handler.html)

`keys::known_hosts::check_known_hosts(host, port, pubkey) -> Result<bool, Error>` "check[s] whether the host is known, from its standard location." It is not called for you. [check_known_hosts](https://docs.rs/russh/0.65.1/russh/keys/known_hosts/fn.check_known_hosts.html)

### License and the frame loop

Apache-2.0 for russh, russh-config, and russh-sftp, per the crates.io records above. The client API is async and the agent client uses `tokio::net`, so the work can run on a Tokio runtime that is not the egui thread. Do not block inside `App::ui` or `App::logic`. See [Beside an egui frame loop](#beside-an-egui-frame-loop).

## ssh2 0.9.7 (libssh2)

Rust bindings to libssh2. crates.io: 0.9.7 published 2026-11-05, MIT OR Apache-2.0, repository `https://github.com/rust-lang/ssh2-rs`. [crates.io ssh2](https://crates.io/api/v1/crates/ssh2). The C library's license is the text at [libssh2.org/license.html](https://libssh2.org/license.html), separate from the crate's SPDX line. This note does not assign that page an SPDX identifier.

The API is synchronous.

### What it can do

- Password: `Session::userauth_password`. [Session, ssh2 0.9.7](https://docs.rs/ssh2/0.9.7/ssh2/struct.Session.html)
- Key file with passphrase: `userauth_pubkey_file(username, pubkey: Option<&Path>, privatekey: &Path, passphrase: Option<&str>)` "attempt[s] public key authentication using a PEM encoded private key file stored on disk." `userauth_pubkey_memory` is the same PEM wording and "is available with openssl enabled (Required for Unix, or with vendored-openssl or openssl-on-win32 features)." Whether libssh2 also accepts an OpenSSH new-format private key was not re-checked in libssh2's C sources for this note. The Rust docs say PEM.
- Agent: `userauth_agent` "attempt[s] to authenticate the current connection with the first public key found in an SSH agent." The same paragraph says more control requires the `agent` API.

### What it cannot do

ssh2 and libssh2 do not parse `ssh_config`. The companion crate ssh2-config 0.9.0 (published 2026-10-20, MIT, repository `https://github.com/veeso/ssh2-config`) describes configuration with the OpenSSH `ssh_config(5)` man page and then says "Only arguments supported by libssh2 are implemented." [HostParams](https://docs.rs/ssh2-config/latest/ssh2_config/struct.HostParams.html). [crates.io ssh2-config](https://crates.io/api/v1/crates/ssh2-config). The parser does not open the connection. The application copies the fields it wants onto the libssh2 session. `ProxyJump` in that parser is not a jump client inside libssh2.

### Directory listing and remote commands

`Sftp::readdir` "read[s] the files in a directory." "The returned paths are all joined with `dirname` when returned, and the paths `.` and `..` are filtered out." `opendir` is the lower-level open. [Sftp](https://docs.rs/ssh2/latest/ssh2/struct.Sftp.html). The session reaches SFTP through `Session::sftp`.

A remote command is `Session::channel_session` and then `Channel::exec`. The same session can open more than one channel. A blocking read on one channel blocks every other call on objects from that session. See the next subsection and the frame-loop section.

A name search is `Channel::exec` of a remote command on that session, not an SFTP search.

### Host keys

Nothing is checked automatically. `Session::host_key` returns `Option<(&[u8], HostKeyType)>`. `KnownHosts` is "a set of known hosts which can be used to verify the identity of a remote server." The documented example reads `~/.ssh/known_hosts` with `KnownHostFileKind::OpenSSH`, then `check`, which returns `CheckResult::Match`, `NotFound`, `Mismatch`, or `Failure`. `check_port` takes a port. `add` and `write_file` update the file. The application calls these. [KnownHosts, ssh2 0.9.7](https://docs.rs/ssh2/0.9.7/ssh2/struct.KnownHosts.html)

### License and the frame loop

Crate license MIT OR Apache-2.0. libssh2's own license is the page linked above.

`Session` and the objects it returns are `Send` and `Sync` and "internally synchronized via a `Mutex`." "A blocking read from a `Channel` or `Stream` will block all other calls on objects created from the same underlying `Session`. If you need the ability to perform concurrent operations then you will need to create separate `Session` instances, or employ non-blocking mode." Separate sessions are separate connections. `set_blocking` "will instantly affect any channels associated with this session." A non-blocking session returns immediately when no data or no write room is available. [Session](https://docs.rs/ssh2/0.9.7/ssh2/struct.Session.html). That is a non-blocking socket mode, not an egui integration. Calling it on the UI thread still occupies `App::ui` until it returns.

## libssh-rs 0.3.8 (libssh)

Rust bindings to libssh, a different C library from libssh2. crates.io: 0.3.8 published 2026-07-05, MIT, repository `https://github.com/wez/libssh-rs`. [crates.io libssh-rs](https://crates.io/api/v1/crates/libssh-rs)

The C library's `COPYING` on the libssh mirror is the GNU Lesser General Public License, version 2.1, February 1999, plus section 17, a special exception "to link the code of its release of libssh with the OpenSSL project's OpenSSL library" and distribute the linked executables. [libssh COPYING](https://raw.githubusercontent.com/libssh/libssh-mirror/master/COPYING). This note states that license text. It is not legal advice about linking.

### What it can do

- Password: `userauth_password`. The docs say it "appears similar to, but is not the same as keyboard-interactive authentication" and "you should prefer using keyboard-interactive authentication over password auth." `userauth_keyboard_interactive` carries the same preference. Usernames "should almost always be `None`" so the configured or config-file username is kept. [lib.rs, libssh-rs 0.3.8](https://docs.rs/libssh-rs/0.3.8/src/libssh_rs/lib.rs.html)
- Key file with passphrase: `SshKey::from_privkey_file(filename, passphrase: Option<&str>)` and `from_privkey_base64`. [SshKey](https://docs.rs/libssh-rs/0.3.8/libssh_rs/struct.SshKey.html). `userauth_publickey(username, privkey)` "authenticate[s] with public/private key or certificate."
- Agent, then `~/.ssh` keys: `userauth_agent` "tr[ies] to authenticate using an ssh agent." `userauth_public_key_auto` "will attempt to use an ssh agent if available, and will then attempt to use your keys/identities from your `~/.ssh` dir." Its `password` argument "can be used to pre-fill a password to unlock the private key(s). Leaving it set to `None` will cause libssh to prompt for the passphrase if you have previously used `set_auth_callback` to configure a callback. If you haven't set the callback and a key is password protected, this authentication method will fail." [lib.rs](https://docs.rs/libssh-rs/0.3.8/src/libssh_rs/lib.rs.html)

### OpenSSH client config

`Session::options_parse_config(file_name: Option<&str>)` "should be the last call of all options, it may overwrite options which are already set. It requires that the `SshOption::Hostname` is already set. if `file_name` is `None` the default `~/.ssh/config` will be used." The implementation passes that pointer to `ssh_options_parse_config`, and `None` becomes a null pointer. [lib.rs lines around `options_parse_config`](https://docs.rs/libssh-rs/0.3.8/src/libssh_rs/lib.rs.html)

The C function those bindings call, documented for libssh 0.12.0, says a null filename uses `~/.ssh/config` and `/etc/ssh/ssh_config`, and "if complied with support for hermetic-usr" (the typo is on the page) also `/usr/etc/ssh/ssh_config` last. [ssh_options_parse_config](https://api.libssh.org/master/group__libssh__session.html). The Rust docstring names only `~/.ssh/config`. The C function it calls also names the system file when the pointer is null.

The same C option list includes `SSH_OPTIONS_PROXYCOMMAND`, `SSH_OPTIONS_PROXYJUMP`, `SSH_OPTIONS_PROCESS_CONFIG`, `SSH_OPTIONS_IDENTITY_AGENT`, `SSH_OPTIONS_IDENTITIES_ONLY`, and `SSH_OPTIONS_STRICTHOSTKEYCHECK`. `SSH_OPTIONS_PROCESS_CONFIG` says libssh "automatically uses" the per-user and system-wide OpenSSH configuration files unless that option is set false or a different file is provided. `SSH_OPTIONS_PREFERRED_AUTHENTICATIONS` "is parsed from the configuration file and stored for the calling application to read; libssh does not automatically reorder authentication methods based on this setting." [ssh_options_set, libssh 0.12.0](https://api.libssh.org/master/group__libssh__session.html). So this is the broadest in-process config application in the set, and it is not a claim that every `ssh_config(5)` keyword is honored. This note did not extract libssh's supported-keyword table from `config.c`, and it did not trace whether `Session::connect` in libssh-rs 0.3.8 leaves `SSH_OPTIONS_PROCESS_CONFIG` at the C default.

### Directory listing and remote commands

"A given session can open multiple `Channel`s." [Session](https://docs.rs/libssh-rs/0.3.8/libssh_rs/struct.Session.html). `Session::sftp` calls `sftp_new`. [lib.rs](https://docs.rs/libssh-rs/0.3.8/src/libssh_rs/lib.rs.html)

`Sftp::open_dir` opens a directory. `Sftp::read_dir` "reads all of the directory entries into a `Vec`" by looping `SftpDir::read_dir`, which returns `sftp_readdir` results and `None` at EOF. The source does not filter `.` or `..`. [sftp.rs](https://docs.rs/libssh-rs/0.3.8/src/libssh_rs/sftp.rs.html)

`Channel::request_exec` "run[s] a shell command without an interactive shell. This is similar to `sh -c command`." The channel must already be a session channel (`open_session`). [Channel](https://docs.rs/libssh-rs/0.3.8/libssh_rs/struct.Channel.html). A name search is that call. It shares the session mutex with the SFTP channel, so the two do not run truly concurrently.

### Host keys

`Session::is_known_server` "check[s] if the servers public key for the connected session is known." The `whoami` example matches `KnownHosts::Ok`, `NotFound`, and `Unknown`, and calls `update_known_hosts_file` only after the application accepts the key. [Session::is_known_server](https://docs.rs/libssh-rs/0.3.8/libssh_rs/struct.Session.html)

### License and the frame loop

Bindings: MIT, crates.io record above. C library: LGPL 2.1 plus the OpenSSL linking exception in `COPYING`.

"libssh doesn't allow using anything associated with a given `Session` from multiple threads concurrently." The bindings put the session in a mutex. Methods lock that session "and this can lead to blocking in surprising situations." [Session, thread safety](https://docs.rs/libssh-rs/0.3.8/libssh_rs/struct.Session.html). No `set_blocking` method was found on the 0.3.8 `Session` page. The safe place for this work relative to egui is a worker thread, with results sent back to the UI.

## openssh 0.11.6 (process-backed)

"Scriptable SSH through OpenSSH (only works on unix)." "This crate wraps the OpenSSH remote login client (`ssh` on most machines)." "Since all commands are executed through the `ssh` command, all your existing configuration (e.g., in `.ssh/config`) should continue to work as expected." [openssh 0.11.6](https://docs.rs/openssh/0.11.6/openssh/). crates.io: 0.11.6 published 2025-12-03, MIT OR Apache-2.0, repository `https://github.com/openssh-rust/openssh.git`. [crates.io openssh](https://crates.io/api/v1/crates/openssh). The system `ssh` binary has OpenSSH's own license, which is not the crate license.

It is a maintained process-backed client: one ControlMaster connection, then later commands reuse it. Two modes are `process-mux` (a process per command against the master; the default feature) and `native-mux` (a socket per command, optional dependency). "The maximum number of multiplexed remote commands is 10 by default" and can be raised with sshd `MaxSessions`. Closing the session severs the connection. [crate docs](https://docs.rs/openssh/0.11.6/openssh/)

### What it can do

- Agent and OpenSSH client config: yes, because the `ssh` binary reads them, unless the builder overrides a setting. `SessionBuilder::config_file` is "equivalent to `ssh -F <p>`" and otherwise "ssh uses `~/.ssh/config`." `ssh_auth_sock` "specif[ies] the path to the ssh-agent." `keyfile` is `ssh -i` and defaults to `None`, so the default is whatever `ssh` would use. `user` is `ssh -l`. `port` is `ssh -p`. [SessionBuilder](https://docs.rs/openssh/0.11.6/openssh/struct.SessionBuilder.html)
- Directory listing and a remote command on that same multiplexed session. See below.

### What it cannot do

"This library supports only password-less authentication schemes. If running `ssh` to a target host requires you to provide input on standard input (`STDIN`), then this crate will not work for you. You should set up keypair-based authentication instead." `Session::connect` and `Session::connect_mux` repeat that: "If connecting requires interactive authentication based on `STDIN` (such as reading a password), the connection will fail." [crate docs](https://docs.rs/openssh/0.11.6/openssh/), [Session](https://docs.rs/openssh/0.11.6/openssh/struct.Session.html)

That excludes password authentication. It also excludes a key file whose passphrase makes `ssh` prompt on stdin. `keyfile` only passes `-i`. No askpass API appears in those docs.

Unix only, as the crate's first line says. `Session::check` is "available on non-Windows only."

### Directory listing and remote commands

`Session::command(program)` escapes `program` "so that special characters aren't evaluated by the remote shell." `raw_command` does not. `shell` passes one escaped argument to `sh -c`. The crate docs warn that escaping targets the shell `shell-escape` targets and can fail for a remote shell with different syntax, and they point at `raw_command` / `raw_arg` for that case. [Session::command](https://docs.rs/openssh/0.11.6/openssh/struct.Session.html), [crate docs, remote shells](https://docs.rs/openssh/0.11.6/openssh/)

A name search is `command`, `raw_command`, or `shell` of a remote command. It reuses the ControlMaster session.

SFTP is `Session::subsystem("sftp")` with piped stdin and stdout, then `openssh_sftp_client::Sftp::new`. The session docs include that example. [Session](https://docs.rs/openssh/0.11.6/openssh/struct.Session.html). openssh-sftp-client 0.15.12 (published 2026-11-04, MIT) exposes `Fs::open_dir` and `Dir::read_dir`, which returns a `ReadDir` that implements `Stream<Item = Result<DirEntry, Error>>`. [Fs::open_dir](https://docs.rs/openssh-sftp-client/0.15.12/openssh_sftp_client/fs/struct.Fs.html), [Dir::read_dir](https://docs.rs/openssh-sftp-client/0.15.12/openssh_sftp_client/fs/struct.Dir.html), [ReadDir](https://docs.rs/openssh-sftp-client/0.15.12/openssh_sftp_client/fs/struct.ReadDir.html). Those pages do not say the stream skips `.` and `..`. [crates.io openssh-sftp-client](https://crates.io/api/v1/crates/openssh-sftp-client)

The SFTP subsystem and the search command are both children of one `Session`. They count toward the server's `MaxSessions` (10 by default).

### Host keys

`KnownHosts::Strict` is `StrictHostKeyChecking=yes` (unknown host rejected). `Add` is `StrictHostKeyChecking=accept-new` ("strict, but if the host is not already in the known hosts file, it will be added"). `Accept` is `StrictHostKeyChecking=no` ("accept whatever key the server provides and add it"). `SessionBuilder::known_hosts_check` defaults to `KnownHosts::Add`. [KnownHosts](https://docs.rs/openssh/0.11.6/openssh/enum.KnownHosts.html), [SessionBuilder](https://docs.rs/openssh/0.11.6/openssh/struct.SessionBuilder.html). The crate cannot show OpenSSH's interactive ask prompt, because it refuses stdin. The default is accept-new, not ask.

`user_known_hosts_file` defaults to `~/.ssh/known_hosts` and `~/.ssh/known_hosts2`, and the path may use `~`.

### License and the frame loop

Crate license MIT OR Apache-2.0. `connect`, `spawn`, and the SFTP client are async, so they can be driven from a runtime that is not the egui thread. The `ssh` processes still block inside the runtime's worker, which is the point of keeping them off `App::ui`.

## Beside an egui frame loop

eframe's `App` trait, current docs linked against egui 0.37.0, requires `ui(&mut self, ui: &mut Ui, frame: &mut Frame)`, "called each time the UI needs repainting, which may be many times per second." `logic` "is called once before each call to `Self::ui`" and also when the UI is hidden if `request_repaint` was called. "You may NOT show any ui or do any painting during the call to `Self::logic`." "To force a repaint, call `egui::Context::request_repaint` at any time (e.g. from another thread)." [eframe::App](https://docs.rs/eframe/latest/eframe/trait.App.html)

Inference from that contract, not from any SSH crate: a call that does not return from `App::ui` or `App::logic` delays the next repaint. None of the four stacks documents an egui integration.

- russh is async and its agent types use Tokio. Run the connection on a runtime outside the frame loop, and call `request_repaint` when a listing or search result arrives.
- `openssh` 0.11.6's session and command APIs are async. Same shape: the runtime is not the UI thread.
- ssh2 blocks the calling thread by default, and one session's mutex makes a blocking read stall every other call on that session. `set_blocking(false)` changes that session's channels. It still must not be polled inside `App::ui`.
- libssh-rs blocks on the session mutex. The 0.3.8 `Session` docs do not show a non-blocking switch. Use a worker thread.

## Dropped after their own docs

sunset 0.6.0, published 2026-08-02, license 0BSD, repository `https://github.com/mkj/sunset`. [crates.io sunset](https://crates.io/api/v1/crates/sunset). Its README lists "SFTP server" under Working and "SFTP client?" under Desirable. Password and public key authentication are listed as working. A client that does not implement SFTP cannot list a remote directory. [sunset 0.6.0 README](https://static.crates.io/readmes/sunset/sunset-0.6.0.html). Dropped because it cannot do the job. The README still describes a maintained project ("at the time of writing Sunset will build with Rust 1.95").

thrussh 0.49.0, published 2026-08-28, Apache-2.0. Its own README says "interactions with agents, PGP, and password-protected/encrypted keys are not yet implemented." [thrussh 0.49.0 README](https://static.crates.io/readmes/thrussh/thrussh-0.49.0.html). Dropped because that text says it cannot do two checklist items. russh 0.65.1, published later from `warp-tech/russh`, is the library whose current docs were used above. "Unmaintained" is the wrong reason: 0.49.0 was published 2026-08-28.

## What was not claimed

- No stack was selected.
- libssh2's ability to read OpenSSH new-format private keys was not re-checked past the ssh2 0.9.7 PEM wording.
- libssh's full keyword-support table in `config.c` was not extracted. The C docs show both real ssh_config options and at least one option (`PreferredAuthentications`) that is stored and not applied by the library.
- Whether libssh-rs 0.3.8 connects with libssh's automatic config processing left on was not traced past `options_parse_config` and the C `SSH_OPTIONS_PROCESS_CONFIG` text.
- `openssh-sftp-client` `ReadDir` was not shown to skip `.` and `..`. russh-sftp's `ReadDir` and ssh2's `Sftp::readdir` were.
- Other crates (ssh-rs, and other process wrappers) were not reviewed, so they were not dropped.
