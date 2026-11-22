//! Hosts: what the operator saves to open Sessions from, kept in `hosts.toml` beside the
//! app settings; and the rules of the Host dialog's Address and Port fields.

use std::io;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// How a Host's Sessions log in.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthMethod {
    #[default]
    Automatic,
    Password,
    KeyFile,
    Agent,
}

/// A saved Host, or the fields of one being entered.
#[derive(Clone, PartialEq, Eq, Debug, Serialize)]
pub struct Host {
    /// A stable name for the Host that never changes with its fields: the credential
    /// store keeps its secret under it. Empty until the Host is first saved.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub id: String,
    pub display_name: String,
    pub address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    pub account: String,
    pub auth: AuthMethod,
    pub key_file: String,
    pub local_folder: PathBuf,
    /// Empty means the account's home directory.
    pub initial_folder: String,
    pub readonly: bool,
    /// Whether its password or key passphrase is kept in the OS credential store.
    pub remember: bool,
}

impl Default for Host {
    /// A new Host: readonly, not remembering, downloading into Downloads.
    fn default() -> Self {
        Host {
            id: String::new(),
            display_name: String::new(),
            address: String::new(),
            port: None,
            account: String::new(),
            auth: AuthMethod::Automatic,
            key_file: String::new(),
            local_folder: downloads_dir(),
            initial_folder: String::new(),
            readonly: true,
            remember: false,
        }
    }
}

/// The local OS user name (`USER`, or `USERNAME` on Windows): the account when neither
/// the Host nor the config names one.
pub fn local_user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default()
}

/// The operator's Downloads directory; `~/Downloads` when the OS names none.
pub fn downloads_dir() -> PathBuf {
    directories::UserDirs::new()
        .and_then(|d| d.download_dir().map(PathBuf::from))
        .or_else(|| std::env::home_dir().map(|h| h.join("Downloads")))
        .unwrap_or_else(|| PathBuf::from("Downloads"))
}

/// A new Host id: 128 random bits in hex.
pub fn new_id() -> String {
    use std::hash::{BuildHasher as _, Hasher as _};
    let half = || {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u128(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos()),
        );
        hasher.finish()
    };
    format!("{:016x}{:016x}", half(), half())
}

impl Host {
    /// The Host read from one `[[host]]` table: a field that is missing or cannot be read
    /// takes a new Host's value, and the others stay as written.
    fn from_table(table: &toml::Table) -> Host {
        fn get<T: DeserializeOwned>(table: &toml::Table, key: &str) -> Option<T> {
            table.get(key)?.clone().try_into().ok()
        }
        let mut host = Host::default();
        macro_rules! read {
            ($($field:ident),*) => {$(
                if let Some(value) = get(table, stringify!($field)) {
                    host.$field = value;
                }
            )*};
        }
        read!(
            id,
            display_name,
            address,
            account,
            auth,
            key_file,
            local_folder,
            initial_folder,
            readonly,
            remember
        );
        host.port = get::<u16>(table, "port").filter(|p| *p != 0);
        host
    }
}

/// The file Hosts are kept in: `hosts.toml`, a `[[host]]` table each, in the left
/// column's order.
#[derive(Clone, Debug)]
pub struct HostStore {
    path: PathBuf,
}

#[derive(Serialize)]
struct HostsFile<'a> {
    host: &'a [Host],
}

impl HostStore {
    /// The Hosts file at `path`.
    pub fn at(path: impl Into<PathBuf>) -> HostStore {
        HostStore { path: path.into() }
    }

    /// `hosts.toml` beside the app settings file at `settings`.
    pub fn beside(settings: &Path) -> HostStore {
        HostStore::at(settings.with_file_name("hosts.toml"))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The saved Hosts; none when the file is missing or is not TOML.
    pub fn load(&self) -> Vec<Host> {
        let Ok(text) = std::fs::read_to_string(&self.path) else {
            return Vec::new();
        };
        let Ok(file) = toml::from_str::<toml::Table>(&text) else {
            return Vec::new();
        };
        let Some(toml::Value::Array(hosts)) = file.get("host") else {
            return Vec::new();
        };
        hosts
            .iter()
            .filter_map(toml::Value::as_table)
            .map(Host::from_table)
            .collect()
    }

    /// Writes `hosts` to the file, creating its directory. A file there that is not TOML
    /// is first moved aside to `<name>.bad`.
    pub fn save(&self, hosts: &[Host]) -> io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        if let Ok(old) = std::fs::read_to_string(&self.path)
            && toml::from_str::<toml::Table>(&old).is_err()
        {
            let mut name = self.path.file_name().unwrap_or_default().to_owned();
            name.push(".bad");
            std::fs::rename(&self.path, self.path.with_file_name(name))?;
        }
        let text = toml::to_string(&HostsFile { host: hosts }).map_err(io::Error::other)?;
        let partial = self.path.with_extension("toml.partial");
        std::fs::write(&partial, text)?;
        std::fs::rename(&partial, &self.path)
    }
}

/// The Port field's value: `Ok(None)` when empty, `Err` unless 1 to 65535.
#[allow(clippy::result_unit_err)]
pub fn port_value(text: &str) -> Result<Option<u16>, ()> {
    let text = text.trim();
    if text.is_empty() {
        return Ok(None);
    }
    match text.parse::<u16>() {
        Ok(0) | Err(_) => Err(()),
        Ok(port) => Ok(Some(port)),
    }
}

/// Why pasted text stayed in Address.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PasteError {
    PortSet,
    AccountSet,
}

impl std::fmt::Display for PasteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            PasteError::PortSet => "Port is already set",
            PasteError::AccountSet => "Account is already set",
        })
    }
}

/// Address, Port and Account after focus left Address.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Split {
    pub address: String,
    pub port: String,
    pub account: String,
    pub errors: Vec<PasteError>,
}

/// Moves a trailing `:port` of `address` into an empty Port and a leading `user@` into
/// an empty Account. Text whose field is already filled stays in Address, with an error.
/// `[v6]:port` is split; a bare IPv6 address, with more than one colon, is left alone.
pub fn split_pasted(address: &str, port: &str, account: &str) -> Split {
    let address = address.trim();
    let mut split = Split {
        address: String::new(),
        port: port.to_owned(),
        account: account.to_owned(),
        errors: Vec::new(),
    };
    let (user, rest) = match address.rfind('@') {
        Some(at) => (Some(&address[..at]), &address[at + 1..]),
        None => (None, address),
    };
    if let Some(user) = user {
        if split.account.trim().is_empty() {
            split.account = user.to_owned();
        } else {
            split.errors.push(PasteError::AccountSet);
            split.address.push_str(user);
            split.address.push('@');
        }
    }
    match split_port(rest) {
        Some((host, pasted)) if split.port.trim().is_empty() => {
            split.port = pasted.to_owned();
            split.address.push_str(host);
        }
        Some(_) => {
            split.errors.push(PasteError::PortSet);
            split.address.push_str(rest);
        }
        None => split.address.push_str(rest),
    }
    split
}

/// `host:port` or `[v6]:port` into the host, without brackets, and the port's digits.
fn split_port(text: &str) -> Option<(&str, &str)> {
    let digits = |p: &str| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit());
    let (host, port) = match text.strip_prefix('[') {
        Some(bracketed) => bracketed.split_once("]:")?,
        None => {
            let (host, port) = text.split_once(':')?;
            if port.contains(':') {
                return None;
            }
            (host, port)
        }
    };
    digits(port).then_some((host, port))
}
