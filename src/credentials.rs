//! Remember: a saved Host's password or key passphrase, kept in the OS credential store
//! (issues #2, #6, #12, #13).
//!
//! The store is behind [`Store`], so tests use a [`MemoryStore`] and never the real one.
//! [`OsStore`] is `keyring` 4 with `v1`: Keychain on macOS, Credential Manager on Windows,
//! the Secret Service elsewhere. Its calls can block (an unlock dialog, D-Bus), so they
//! never run on the frame thread. On Linux a Secret Service that is missing at the first
//! call stays missing for the rest of the run, since `keyring` sets its store up once.
//!
//! Each Host keeps at most one secret of each [`Kind`], under the service `sshexplorer`
//! and the user name `<host id>:password` or `<host id>:passphrase`.

use std::collections::HashMap;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use zeroize::Zeroizing;

use crate::hosts::Host;

/// The service name every entry is kept under.
pub const SERVICE: &str = "sshexplorer";

/// The dialog line when the store could not say whether it keeps a secret.
pub const UNAVAILABLE: &str =
    "The credential store did not return a saved secret. This Session will use what you enter.";
/// The Session note when the store did not keep a secret that logged in.
pub const NOT_KEPT: &str =
    "The credential store did not keep the secret. This Session will use what you entered.";
/// The Host dialog's line when a kept secret could not be removed.
pub const NOT_REMOVED: &str =
    "The credential store did not remove the saved secret. It may still be kept there.";

/// Which secret: a password (the password method, or a keyboard-interactive prompt that
/// asks for one) or a key file's passphrase.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Kind {
    Password,
    Passphrase,
}

impl Kind {
    /// How the dialog names it: "password" or "key passphrase".
    pub fn name(self) -> &'static str {
        match self {
            Kind::Password => "password",
            Kind::Passphrase => "key passphrase",
        }
    }
}

/// One entry of the store: a Host's secret of one kind.
#[derive(Clone, PartialEq, Eq, Debug, Hash)]
pub struct Key {
    pub host: String,
    pub kind: Kind,
}

impl Key {
    pub fn new(host: impl Into<String>, kind: Kind) -> Key {
        Key {
            host: host.into(),
            kind,
        }
    }

    /// The entry's user name in the store: `<host id>:password` or `<host id>:passphrase`.
    pub fn username(&self) -> String {
        let kind = match self.kind {
            Kind::Password => "password",
            Kind::Passphrase => "passphrase",
        };
        format!("{}:{kind}", self.host)
    }
}

/// What the store answered when asked for a secret.
pub enum Lookup {
    Found(Zeroizing<String>),
    /// It keeps none for that entry.
    NoEntry,
    /// It could not say: no store, no access, or a failure.
    Unavailable,
}

/// The store failed to do what it was asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Refused;

/// A credential store. Each call may block, so none runs on the frame thread.
pub trait Store: Send + Sync {
    fn get(&self, key: &Key) -> Lookup;
    fn set(&self, key: &Key, secret: &str) -> Result<(), Refused>;
    /// Removes the entry; nothing to remove is not a failure.
    fn delete(&self, key: &Key) -> Result<(), Refused>;
}

/// The OS credential store, through `keyring`.
pub struct OsStore;

impl Store for OsStore {
    fn get(&self, key: &Key) -> Lookup {
        let entry = match keyring::Entry::new(SERVICE, &key.username()) {
            Ok(entry) => entry,
            Err(_) => return Lookup::Unavailable,
        };
        match entry.get_password() {
            Ok(secret) => Lookup::Found(Zeroizing::new(secret)),
            Err(keyring::Error::NoEntry) => Lookup::NoEntry,
            Err(_) => Lookup::Unavailable,
        }
    }

    fn set(&self, key: &Key, secret: &str) -> Result<(), Refused> {
        keyring::Entry::new(SERVICE, &key.username())
            .and_then(|entry| entry.set_password(secret))
            .map_err(|_| Refused)
    }

    fn delete(&self, key: &Key) -> Result<(), Refused> {
        match keyring::Entry::new(SERVICE, &key.username())
            .and_then(|entry| entry.delete_credential())
        {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(Refused),
        }
    }
}

/// A store in memory, for tests: each call can be made to fail.
#[derive(Default)]
pub struct MemoryStore {
    entries: Mutex<HashMap<Key, String>>,
    fail_get: AtomicBool,
    fail_set: AtomicBool,
    fail_delete: AtomicBool,
}

impl MemoryStore {
    /// What it keeps for `key`, without going through [`Store::get`].
    pub fn secret(&self, key: &Key) -> Option<String> {
        self.entries.lock().ok()?.get(key).cloned()
    }

    /// From now on, `get` answers [`Lookup::Unavailable`] (or not).
    pub fn fail_get(&self, fail: bool) {
        self.fail_get.store(fail, Ordering::SeqCst);
    }

    /// From now on, `set` is refused (or not).
    pub fn fail_set(&self, fail: bool) {
        self.fail_set.store(fail, Ordering::SeqCst);
    }

    /// From now on, `delete` is refused (or not).
    pub fn fail_delete(&self, fail: bool) {
        self.fail_delete.store(fail, Ordering::SeqCst);
    }
}

impl Store for MemoryStore {
    fn get(&self, key: &Key) -> Lookup {
        if self.fail_get.load(Ordering::SeqCst) {
            return Lookup::Unavailable;
        }
        match self.secret(key) {
            Some(secret) => Lookup::Found(Zeroizing::new(secret)),
            None => Lookup::NoEntry,
        }
    }

    fn set(&self, key: &Key, secret: &str) -> Result<(), Refused> {
        if self.fail_set.load(Ordering::SeqCst) {
            return Err(Refused);
        }
        let mut entries = self.entries.lock().map_err(|_| Refused)?;
        entries.insert(key.clone(), secret.to_owned());
        Ok(())
    }

    fn delete(&self, key: &Key) -> Result<(), Refused> {
        if self.fail_delete.load(Ordering::SeqCst) {
            return Err(Refused);
        }
        let mut entries = self.entries.lock().map_err(|_| Refused)?;
        entries.remove(key);
        Ok(())
    }
}

/// The kinds of secret that saving the Host `old` as `new` (or deleting it, `None`)
/// removes from the store: both when Remember goes off, the Host goes, or its address
/// or account changes; the passphrase when its key-file path changes. A Host that did
/// not remember keeps nothing to remove.
pub fn forgotten(old: &Host, new: Option<&Host>) -> Vec<Kind> {
    if !old.remember {
        return Vec::new();
    }
    match new {
        None => vec![Kind::Password, Kind::Passphrase],
        Some(new) if !new.remember || new.address != old.address || new.account != old.account => {
            vec![Kind::Password, Kind::Passphrase]
        }
        Some(new) if new.key_file != old.key_file => vec![Kind::Passphrase],
        Some(_) => Vec::new(),
    }
}
