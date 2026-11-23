//! The operator's OpenSSH config, read and applied by sshexplorer itself (ADR-0002).
//!
//! [`Config::read`] parses `~/.ssh/config` and the files it `Include`s; a missing file
//! reads as an empty config. [`Config::lookup`] finds what the config says for an address,
//! matched against the `Host` patterns as `ssh <address>` would match it, and
//! [`Config::resolve`] combines that with a Host's own fields into where and how to
//! connect. Lookup is pure and quick, so the Host dialog can run it on every keystroke.
//!
//! Only `HostName`, `User`, `Port`, `IdentityFile`, `IdentitiesOnly`, `IdentityAgent`,
//! `UserKnownHostsFile`, `ConnectTimeout` and `ServerAliveInterval` apply. `ProxyJump`
//! and `ProxyCommand` are noted so the Session can refuse them. `Match` blocks are
//! skipped up to the next `Host` or `Match` line, and every other keyword is ignored.
//! The first value of a keyword wins, except `IdentityFile`, whose values add up.
//!
//! A config with a line that cannot be read applies nothing: [`Config::error`] names the
//! file and the line, and the Session opens from the Host fields alone.

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::ssh::Target;

/// How deep `Include`s may nest, as in OpenSSH.
const INCLUDE_DEPTH: usize = 16;

/// The config file as the Host dialog names it, on every OS.
pub const CONFIG_NAME: &str = "~/.ssh/config";

/// A parsed config file and the files it includes.
#[derive(Debug, Clone, Default)]
pub struct Config {
    items: Vec<Item>,
    error: Option<ConfigError>,
    home: PathBuf,
}

/// A line of a config file that matters to sshexplorer.
#[derive(Debug, Clone)]
enum Item {
    /// `Host` and its patterns: the lines after it apply when the address matches.
    Host(Vec<String>),
    /// `Match`: the lines after it never apply.
    Match,
    /// The lines of the files an `Include` names, at its place.
    Include(Vec<Item>),
    Set(Setting),
}

/// A keyword sshexplorer applies, with its value.
#[derive(Debug, Clone)]
enum Setting {
    HostName(String),
    User(String),
    Port(u16),
    IdentityFile(String),
    IdentitiesOnly(bool),
    IdentityAgent(String),
    UserKnownHostsFile(Vec<String>),
    ConnectTimeout(u64),
    ServerAliveInterval(u64),
    ProxyJump(String),
    ProxyCommand(String),
}

/// A config line that could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    pub file: PathBuf,
    /// 1-based, counting every line of the file.
    pub line: usize,
    pub reason: String,
}

impl ConfigError {
    /// The one-line note an open Session shows: the file (with `home` written `~`), the
    /// line, and what is wrong with it.
    pub fn note(&self, home: &Path) -> String {
        format!(
            "{} line {}: {}. Opened from the Host fields alone.",
            tilde(&self.file, home),
            self.line,
            self.reason
        )
    }
}

/// `path` with `home` written as `~`, and `/` between the parts.
fn tilde(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if !home.as_os_str().is_empty() => {
            let parts: Vec<String> = rest
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            format!("~/{}", parts.join("/"))
        }
        _ => path.display().to_string(),
    }
}

/// What the config says for one address. A value the config does not set is `None`, or
/// empty for `identity_files`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Matched {
    /// With `%h` expanded to the address.
    pub host_name: Option<String>,
    pub user: Option<String>,
    pub port: Option<u16>,
    /// As written, unexpanded, in order.
    pub identity_files: Vec<String>,
    pub identities_only: Option<bool>,
    pub identity_agent: Option<String>,
    pub user_known_hosts_files: Option<Vec<String>>,
    pub connect_timeout: Option<u64>,
    pub server_alive_interval: Option<u64>,
    pub proxy_jump: Option<String>,
    pub proxy_command: Option<String>,
}

/// The Host's own fields that the config can stand in for. Empty means "not set".
#[derive(Debug, Clone, Default)]
pub struct HostFields {
    pub address: String,
    pub port: Option<u16>,
    pub account: String,
    pub key_file: String,
}

/// Where to reach the SSH agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdentityAgent {
    /// `SSH_AUTH_SOCK`, or the OpenSSH agent's pipe on Windows.
    Default,
    /// `IdentityAgent none`, or a `${VAR}` that is not set: no agent.
    Off,
    /// A socket the config names.
    Socket(PathBuf),
}

/// Where and how to connect, once the Host's fields, the config and the defaults are
/// combined: a non-empty Host field wins, then the config, then the defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// The address is the config's `HostName`, else the address as typed.
    pub target: Target,
    /// The Key file, else every `IdentityFile`; none by default.
    pub key_files: Vec<PathBuf>,
    pub identities_only: bool,
    pub identity_agent: IdentityAgent,
    /// The `UserKnownHostsFile`s, checked as well as `~/.ssh/known_hosts`.
    pub known_hosts: Vec<PathBuf>,
    pub connect_timeout: Option<Duration>,
    /// `None` when the config does not set it; zero turns keepalives off.
    pub server_alive_interval: Option<Duration>,
}

/// The config routes the host through a proxy, which sshexplorer cannot follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NeedsProxy {
    /// `ProxyJump` or `ProxyCommand`.
    pub keyword: &'static str,
}

impl fmt::Display for NeedsProxy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "This host needs {}, which sshexplorer does not support yet.",
            self.keyword
        )
    }
}

impl Config {
    /// Reads the config at `path`, with `home` the operator's home directory: `~`
    /// stands for it, and a relative `Include` is under `path`'s folder (`~/.ssh`).
    pub fn read(path: &Path, home: &Path) -> Config {
        let base = path.parent().unwrap_or(Path::new("")).to_path_buf();
        let reader = Reader { home, base: &base };
        let (items, error) = match std::fs::read_to_string(path) {
            Ok(text) => match reader.parse(path, &text, 0) {
                Ok(items) => (items, None),
                Err(error) => (Vec::new(), Some(error)),
            },
            Err(_) => (Vec::new(), None),
        };
        Config {
            items,
            error,
            home: home.to_path_buf(),
        }
    }

    /// The line that kept the config from being read, if any.
    pub fn error(&self) -> Option<&ConfigError> {
        self.error.as_ref()
    }

    /// The note an open Session shows when the config could not be read.
    pub fn note(&self) -> Option<String> {
        self.error.as_ref().map(|e| e.note(&self.home))
    }

    /// What the config says for `address`, the address as typed.
    pub fn lookup(&self, address: &str) -> Matched {
        let mut matched = Matched::default();
        let address = address.trim();
        walk(&self.items, address, &mut matched);
        if let Some(name) = &mut matched.host_name {
            *name = percent(name, &[('h', address)]);
        }
        matched
    }

    /// Combines `fields` with what the config says for their address. `local_user` is
    /// the account when neither names one.
    pub fn resolve(&self, fields: &HostFields, local_user: &str) -> Result<Resolved, NeedsProxy> {
        let address = fields.address.trim();
        let m = self.lookup(address);
        let routed = |v: &Option<String>| {
            v.as_deref()
                .is_some_and(|v| !v.eq_ignore_ascii_case("none"))
        };
        if routed(&m.proxy_jump) {
            return Err(NeedsProxy {
                keyword: "ProxyJump",
            });
        }
        if routed(&m.proxy_command) {
            return Err(NeedsProxy {
                keyword: "ProxyCommand",
            });
        }
        let account = fields.account.trim();
        let target = Target {
            address: m.host_name.clone().unwrap_or_else(|| address.to_owned()),
            port: fields.port.or(m.port).unwrap_or(22),
            account: if account.is_empty() {
                m.user.clone().unwrap_or_else(|| local_user.to_owned())
            } else {
                account.to_owned()
            },
        };
        let port = target.port.to_string();
        let home = self.home.to_string_lossy().into_owned();
        let tokens = [
            ('d', home.as_str()),
            ('h', target.address.as_str()),
            ('n', address),
            ('p', port.as_str()),
            ('r', target.account.as_str()),
            ('u', local_user),
        ];
        let path = |p: &str| PathBuf::from(expand_tilde(&percent(p, &tokens), &self.home));
        let key_file = fields.key_file.trim();
        let key_files = if key_file.is_empty() {
            m.identity_files.iter().map(|f| path(f)).collect()
        } else {
            vec![path(key_file)]
        };
        let identity_agent = match m.identity_agent.as_deref() {
            None | Some("SSH_AUTH_SOCK") => IdentityAgent::Default,
            Some(a) if a.eq_ignore_ascii_case("none") => IdentityAgent::Off,
            Some(a) => match a.strip_prefix("${").and_then(|a| a.strip_suffix('}')) {
                Some(var) => match std::env::var_os(var) {
                    Some(socket) => IdentityAgent::Socket(socket.into()),
                    None => IdentityAgent::Off,
                },
                None => IdentityAgent::Socket(path(a)),
            },
        };
        let known_hosts = m
            .user_known_hosts_files
            .iter()
            .flatten()
            .filter(|f| !f.eq_ignore_ascii_case("none"))
            .map(|f| path(f))
            .collect();
        Ok(Resolved {
            target,
            key_files,
            identities_only: m.identities_only.unwrap_or(false),
            identity_agent,
            known_hosts,
            connect_timeout: m
                .connect_timeout
                .filter(|&s| s > 0)
                .map(Duration::from_secs),
            server_alive_interval: m.server_alive_interval.map(Duration::from_secs),
        })
    }
}

/// Applies `items`, one file's lines, to `matched`. A file starts out applying; a file
/// included from a block that does not apply is never walked, and an included file's
/// `Host` lines end at its end.
fn walk(items: &[Item], address: &str, matched: &mut Matched) {
    let mut active = true;
    for item in items {
        match item {
            Item::Host(patterns) => active = host_matches(patterns, address),
            Item::Match => active = false,
            Item::Include(included) if active => walk(included, address, matched),
            Item::Set(setting) if active => apply(setting, matched),
            Item::Include(_) | Item::Set(_) => {}
        }
    }
}

fn apply(setting: &Setting, m: &mut Matched) {
    match setting.clone() {
        Setting::HostName(v) => first(&mut m.host_name, v),
        Setting::User(v) => first(&mut m.user, v),
        Setting::Port(v) => first(&mut m.port, v),
        Setting::IdentityFile(v) => m.identity_files.push(v),
        Setting::IdentitiesOnly(v) => first(&mut m.identities_only, v),
        Setting::IdentityAgent(v) => first(&mut m.identity_agent, v),
        Setting::UserKnownHostsFile(v) => first(&mut m.user_known_hosts_files, v),
        Setting::ConnectTimeout(v) => first(&mut m.connect_timeout, v),
        Setting::ServerAliveInterval(v) => first(&mut m.server_alive_interval, v),
        Setting::ProxyJump(v) => first(&mut m.proxy_jump, v),
        Setting::ProxyCommand(v) => first(&mut m.proxy_command, v),
    }
}

/// Sets `slot` unless an earlier line already did: the first value wins.
fn first<T>(slot: &mut Option<T>, value: T) {
    if slot.is_none() {
        *slot = Some(value);
    }
}

/// Reads config files: `~` is `home`, and a relative `Include` is under `base`.
struct Reader<'a> {
    home: &'a Path,
    base: &'a Path,
}

/// Why a line could not be read.
enum Parse {
    /// This line is wrong, for this reason.
    Line(String),
    /// A line of a file it includes is wrong.
    Included(ConfigError),
}

impl Reader<'_> {
    fn parse(&self, file: &Path, text: &str, depth: usize) -> Result<Vec<Item>, ConfigError> {
        let mut items = Vec::new();
        for (i, line) in text.lines().enumerate() {
            let error = |reason: String| ConfigError {
                file: file.to_path_buf(),
                line: i + 1,
                reason,
            };
            let Some((keyword, args)) = split_line(line).map_err(error)? else {
                continue;
            };
            let item = self.item(&keyword, args, depth).map_err(|e| match e {
                Parse::Line(reason) => error(reason),
                Parse::Included(inner) => inner,
            })?;
            items.extend(item);
        }
        Ok(items)
    }

    fn item(&self, keyword: &str, args: Vec<String>, depth: usize) -> Result<Option<Item>, Parse> {
        let one = || {
            args.first()
                .cloned()
                .ok_or_else(|| Parse::Line(format!("{keyword} needs a value")))
        };
        let bad = |value: &str| Parse::Line(format!("bad {keyword} \"{value}\""));
        let seconds = || {
            let value = one()?;
            value.parse::<u64>().map_err(|_| bad(&value))
        };
        let setting = match keyword.to_ascii_lowercase().as_str() {
            "host" if args.is_empty() => return Err(Parse::Line("Host needs a pattern".into())),
            "host" => return Ok(Some(Item::Host(args))),
            "match" => return Ok(Some(Item::Match)),
            "include" => return self.include(&args, depth).map(Some),
            "hostname" => Setting::HostName(one()?),
            "user" => Setting::User(one()?),
            "port" => {
                let value = one()?;
                match value.parse::<u16>() {
                    Ok(port) if port > 0 => Setting::Port(port),
                    _ => return Err(bad(&value)),
                }
            }
            "identityfile" => Setting::IdentityFile(one()?),
            "identitiesonly" => {
                let value = one()?;
                match value.to_ascii_lowercase().as_str() {
                    "yes" => Setting::IdentitiesOnly(true),
                    "no" => Setting::IdentitiesOnly(false),
                    _ => return Err(bad(&value)),
                }
            }
            "identityagent" => Setting::IdentityAgent(one()?),
            "userknownhostsfile" => {
                one()?;
                Setting::UserKnownHostsFile(args)
            }
            "connecttimeout" => Setting::ConnectTimeout(seconds()?),
            "serveraliveinterval" => Setting::ServerAliveInterval(seconds()?),
            "proxyjump" => Setting::ProxyJump(one()?),
            "proxycommand" => {
                one()?;
                Setting::ProxyCommand(args.join(" "))
            }
            _ => return Ok(None),
        };
        Ok(Some(Item::Set(setting)))
    }

    /// Reads the files `Include args` names; a file that is not there is skipped.
    fn include(&self, args: &[String], depth: usize) -> Result<Item, Parse> {
        if args.is_empty() {
            return Err(Parse::Line("Include needs a file".into()));
        }
        if depth >= INCLUDE_DEPTH {
            return Err(Parse::Line("Include nests too deep".into()));
        }
        let mut items = Vec::new();
        for arg in args {
            for path in self.include_paths(arg) {
                if let Ok(text) = std::fs::read_to_string(&path) {
                    let included = self
                        .parse(&path, &text, depth + 1)
                        .map_err(Parse::Included)?;
                    items.extend(included);
                }
            }
        }
        Ok(Item::Include(items))
    }

    /// The files an `Include` argument names, globs expanded in lexical order.
    fn include_paths(&self, arg: &str) -> Vec<PathBuf> {
        let expanded = PathBuf::from(expand_tilde(arg, self.home));
        let path = if expanded.is_absolute() {
            expanded
        } else {
            self.base.join(expanded)
        };
        let mut found = vec![PathBuf::new()];
        for component in path.components() {
            let part = component.as_os_str().to_string_lossy();
            if !part.contains(['*', '?']) {
                for p in &mut found {
                    p.push(component);
                }
                continue;
            }
            let mut next = Vec::new();
            for dir in &found {
                let Ok(entries) = std::fs::read_dir(dir) else {
                    continue;
                };
                let mut names: Vec<String> = entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| wildcard(part.as_bytes(), n.as_bytes()))
                    .collect();
                names.sort();
                next.extend(names.into_iter().map(|n| dir.join(n)));
            }
            found = next;
        }
        found.retain(|p| p.is_file());
        found
    }
}

/// A line's keyword and arguments, or `None` for a blank or comment line. The keyword
/// ends at whitespace or `=`. Arguments are split at whitespace, `"` quotes one with
/// spaces in it, and a `#` that starts an argument comments out the rest.
fn split_line(line: &str) -> Result<Option<(String, Vec<String>)>, String> {
    let line = line.trim_start();
    if line.is_empty() || line.starts_with('#') {
        return Ok(None);
    }
    let end = line
        .find(|c: char| c.is_whitespace() || c == '=')
        .unwrap_or(line.len());
    let keyword = line[..end].to_owned();
    let mut rest = line[end..].trim_start();
    if let Some(after) = rest.strip_prefix('=') {
        rest = after.trim_start();
    }
    let mut args = Vec::new();
    let mut chars = rest.chars().peekable();
    loop {
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        let Some(&c) = chars.peek() else { break };
        if c == '#' {
            break;
        }
        let mut arg = String::new();
        if c == '"' {
            chars.next();
            loop {
                match chars.next() {
                    Some('"') => break,
                    Some(c) => arg.push(c),
                    None => return Err("unclosed quote".into()),
                }
            }
        } else {
            while let Some(c) = chars.next_if(|c| !c.is_whitespace()) {
                arg.push(c);
            }
        }
        args.push(arg);
    }
    Ok(Some((keyword, args)))
}

/// `text` with a leading `~` or `~/` standing for `home`.
fn expand_tilde(text: &str, home: &Path) -> String {
    if text == "~" {
        return home.to_string_lossy().into_owned();
    }
    match text.strip_prefix("~/") {
        Some(rest) => home.join(rest).to_string_lossy().into_owned(),
        None => text.to_owned(),
    }
}

/// `text` with each `%x` of `tokens` replaced and `%%` made `%`. Other tokens stay.
fn percent(text: &str, tokens: &[(char, &str)]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('%') => out.push('%'),
            Some(t) => match tokens.iter().find(|(k, _)| *k == t) {
                Some((_, value)) => out.push_str(value),
                None => {
                    out.push('%');
                    out.push(t);
                }
            },
            None => out.push('%'),
        }
    }
    out
}

/// Whether a `Host` line's patterns take in `address`: some pattern matches it and no
/// negated one (`!pattern`) does.
fn host_matches(patterns: &[String], address: &str) -> bool {
    let address = address.to_ascii_lowercase();
    let mut any = false;
    for pattern in patterns {
        let pattern = pattern.to_ascii_lowercase();
        match pattern.strip_prefix('!') {
            Some(negated) if wildcard(negated.as_bytes(), address.as_bytes()) => return false,
            Some(_) => {}
            None => any |= wildcard(pattern.as_bytes(), address.as_bytes()),
        }
    }
    any
}

/// Whether `text` matches `pattern`, where `*` stands for any run of characters and `?`
/// for any one.
fn wildcard(pattern: &[u8], text: &[u8]) -> bool {
    let (mut p, mut t) = (0, 0);
    let mut star = None;
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == b'?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == b'*' {
            star = Some((p, t));
            p += 1;
        } else if let Some((sp, st)) = star {
            p = sp + 1;
            t = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|&c| c == b'*')
}
