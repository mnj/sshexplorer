//! Known hosts: the operator's `known_hosts` file, shared with `ssh` (ADR-0002 makes the
//! check sshexplorer's own code, and russh's own helpers miscount lines after comments).
//!
//! [`check`] reads a file's text the way OpenSSH does: a host's name is `address` for
//! port 22 and `[address]:port` otherwise, lower-cased; the host field is a comma list of
//! patterns with `*`, `?` and `!` negation, or one hashed `|1|salt|hash` name. A recorded
//! key equal to the host's is Known. Else a recorded key of the same algorithm is a
//! Changed key, and else the host is Unknown. A line that does not parse is skipped.
//! `@revoked` keys are refused; `@cert-authority` lines are skipped, as sshexplorer does
//! not take host certificates.

use std::io::Write as _;
use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use hmac::{KeyInit as _, Mac as _};
use russh::keys::ssh_key::{Algorithm, HashAlg, PublicKey};

/// What a `known_hosts` file says about a host's key.
#[derive(Debug, PartialEq, Eq)]
pub enum Verdict {
    /// The key is recorded for the host.
    Known,
    /// No key of this algorithm is recorded for the host.
    Unknown,
    /// The host is recorded with a different key of the same algorithm.
    Changed(Recorded),
    /// The key is marked `@revoked`.
    Revoked(Recorded),
}

/// The `known_hosts` line that a Changed or Revoked verdict comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Recorded {
    /// 1-based, counting comments and blank lines.
    pub line_no: usize,
    /// The line as written.
    pub line: String,
    /// The recorded key's `SHA256:` fingerprint.
    pub fingerprint: String,
}

/// The name a host is recorded under: `address` on port 22, else `[address]:port`.
pub fn host_name(address: &str, port: u16) -> String {
    let address = address.to_lowercase();
    if port == 22 {
        address
    } else {
        format!("[{address}]:{port}")
    }
}

/// A key's fingerprint as OpenSSH shows it: `SHA256:` and unpadded base64.
pub fn fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

/// Checks `key` for the host at `address` and `port` against a `known_hosts` file's text.
pub fn check(text: &str, address: &str, port: u16, key: &PublicKey) -> Verdict {
    let name = host_name(address, port);
    let mut changed = None;
    for line in lines(text).filter(|l| l.matches(&name)) {
        let same_key = line.key.key_data() == key.key_data();
        match line.marker {
            Marker::Revoked if same_key => return Verdict::Revoked(line.recorded()),
            Marker::Revoked | Marker::CertAuthority => {}
            Marker::None if same_key => return Verdict::Known,
            Marker::None => {
                if changed.is_none() && line.key.algorithm() == key.algorithm() {
                    changed = Some(line.recorded());
                }
            }
        }
    }
    changed.map_or(Verdict::Unknown, Verdict::Changed)
}

/// The algorithms of the keys recorded for the host, in file order: what to prefer in
/// the key exchange, so that a host known by one key type is not asked for another.
pub fn recorded_algorithms(text: &str, address: &str, port: u16) -> Vec<Algorithm> {
    let name = host_name(address, port);
    let mut algorithms = Vec::new();
    for line in lines(text).filter(|l| l.marker == Marker::None && l.matches(&name)) {
        let algorithm = line.key.algorithm();
        if !algorithms.contains(&algorithm) {
            algorithms.push(algorithm);
        }
    }
    algorithms
}

/// The line that records `key` for the host: `name algorithm base64`.
pub fn entry(address: &str, port: u16, key: &PublicKey) -> String {
    let key = key.to_openssh().unwrap_or_default();
    // `to_openssh` adds the comment, which a host key has none of; drop a trailing space.
    format!("{} {}", host_name(address, port), key.trim_end())
}

/// Appends the line recording `key` for the host to the file at `path`, creating the file
/// and its directory when missing.
pub fn append(path: &Path, address: &str, port: u16, key: &PublicKey) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let existing = std::fs::read(path).unwrap_or_default();
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let mut text = String::new();
    if !existing.is_empty() && !existing.ends_with(b"\n") {
        text.push('\n');
    }
    text.push_str(&entry(address, port, key));
    text.push('\n');
    file.write_all(text.as_bytes())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Marker {
    None,
    Revoked,
    CertAuthority,
}

struct Line<'a> {
    no: usize,
    text: &'a str,
    marker: Marker,
    hosts: &'a str,
    key: PublicKey,
}

impl Line<'_> {
    fn recorded(&self) -> Recorded {
        Recorded {
            line_no: self.no,
            line: self.text.to_string(),
            fingerprint: fingerprint(&self.key),
        }
    }

    /// Whether the host field names `name`.
    fn matches(&self, name: &str) -> bool {
        if let Some(hashed) = self.hosts.strip_prefix("|1|") {
            return hashed_matches(hashed, name);
        }
        let mut hit = false;
        for pattern in self.hosts.split(',') {
            if let Some(negated) = pattern.strip_prefix('!') {
                if wildcard(&negated.to_lowercase(), name) {
                    return false;
                }
            } else if wildcard(&pattern.to_lowercase(), name) {
                hit = true;
            }
        }
        hit
    }
}

/// The lines of a file that parse, numbered from 1.
fn lines(text: &str) -> impl Iterator<Item = Line<'_>> {
    text.lines().enumerate().filter_map(|(i, raw)| {
        let text = raw.trim_end_matches('\r');
        let mut fields = text.split_ascii_whitespace();
        let mut first = fields.next()?;
        if first.starts_with('#') {
            return None;
        }
        let marker = match first {
            "@revoked" => Marker::Revoked,
            "@cert-authority" => Marker::CertAuthority,
            m if m.starts_with('@') => return None,
            _ => Marker::None,
        };
        if marker != Marker::None {
            first = fields.next()?;
        }
        let algorithm = fields.next()?;
        let data = fields.next()?;
        let key = PublicKey::from_openssh(&format!("{algorithm} {data}")).ok()?;
        Some(Line {
            no: i + 1,
            text,
            marker,
            hosts: first,
            key,
        })
    })
}

/// Whether a hashed name `salt|hash` (both base64) is HMAC-SHA1(salt, name).
fn hashed_matches(hashed: &str, name: &str) -> bool {
    let Some((salt, hash)) = hashed.split_once('|') else {
        return false;
    };
    let (Ok(salt), Ok(hash)) = (STANDARD.decode(salt), STANDARD.decode(hash)) else {
        return false;
    };
    let Ok(mut mac) = hmac::Hmac::<sha1::Sha1>::new_from_slice(&salt) else {
        return false;
    };
    mac.update(name.as_bytes());
    mac.verify_slice(&hash).is_ok()
}

/// OpenSSH's pattern match: `*` is any run of characters, `?` any one.
fn wildcard(pattern: &str, name: &str) -> bool {
    let (p, n): (Vec<char>, Vec<char>) = (pattern.chars().collect(), name.chars().collect());
    let (mut pi, mut ni) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ni));
            pi += 1;
        } else if let Some((sp, sn)) = star {
            pi = sp + 1;
            ni = sn + 1;
            star = Some((sp, sn + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests;
