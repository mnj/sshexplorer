//! The throwaway keys in tests/keys, and a test SSH agent holding some of them.
//!
//! The keys were made for these tests with `ssh-keygen` (OpenSSH, `-m PEM` and
//! `-m PKCS8`) and `openssl` (the PKCS#8 Ed25519 pair). Those named `*_locked` have the
//! passphrase [`PASSPHRASE`]. Their public halves are in tests/keys/pub, apart from the
//! keys, so that the app has to derive them from the key file itself.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use russh::keys::{PrivateKey, PublicKey};

pub const PASSPHRASE: &str = "hunter2";

pub fn key_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/keys")
        .join(name)
}

/// The unlocked key `name`, for the test agent to hold.
pub fn private_key(name: &str) -> PrivateKey {
    let text = std::fs::read_to_string(key_path(name)).unwrap();
    let passphrase = name.ends_with("_locked").then_some(PASSPHRASE);
    russh::keys::decode_secret_key(&text, passphrase).unwrap()
}

/// The public half of the key `name`, from tests/keys/pub (kept apart from the keys, so
/// that the app derives it from the key file itself).
pub fn public_key(name: &str) -> PublicKey {
    russh::keys::load_public_key(key_path(&format!("pub/{name}.pub"))).unwrap()
}

/// A test SSH agent: russh's agent server on a socket in a temp directory, holding
/// keys of tests/keys, on its own runtime. Dropping it stops the agent.
#[cfg(unix)]
pub struct TestAgent {
    dir: tempfile::TempDir,
    _runtime: tokio::runtime::Runtime,
}

#[cfg(unix)]
impl TestAgent {
    /// An agent holding the keys of tests/keys named `names`.
    pub fn holding_keys(names: &[&str]) -> TestAgent {
        use russh::keys::agent::client::AgentClient;
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("agent.sock");
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .unwrap();
        let listener = runtime.block_on(async { tokio::net::UnixListener::bind(&socket) });
        let listener = listener.unwrap();
        let incoming = Box::pin(futures::stream::unfold(listener, |l| async move {
            let stream = l.accept().await.map(|(s, _)| s);
            Some((stream, l))
        }));
        runtime.spawn(russh::keys::agent::server::serve(incoming, ()));
        let keys: Vec<PrivateKey> = names.iter().map(|n| private_key(n)).collect();
        runtime.block_on(async {
            let mut client = AgentClient::connect_uds(&socket).await.unwrap();
            for key in &keys {
                client.add_identity(key, &[]).await.unwrap();
            }
        });
        TestAgent {
            dir,
            _runtime: runtime,
        }
    }

    pub fn socket(&self) -> PathBuf {
        self.dir.path().join("agent.sock")
    }
}
