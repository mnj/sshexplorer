//! Logging in, by the Host's Authentication method: key files, the SSH agent, and the
//! password, which keyboard-interactive can ask for too.
//!
//! Automatic tries, in order: (1) each key file, through the agent when it holds that
//! key, else loaded from disk with its passphrase asked for when locked; (2) the agent's
//! other identities, unless `IdentitiesOnly`; (3) the password. Key file, Agent and
//! Password each keep to their own step. A method the host does not offer is skipped.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use russh::client::{AuthResult, Handle, KeyboardInteractiveAuthResponse};
use russh::keys::agent::AgentIdentity;
use russh::keys::agent::client::{AgentClient, AgentStream};
use russh::keys::ssh_key::{HashAlg, PrivateKey, PublicKey};
use russh::keys::{self, PrivateKeyWithHashAlg};
use russh::{AgentAuthError, Disconnect, MethodKind, MethodSet};
use tokio::sync::oneshot;

use super::{Event, Events, Failure, HostCheck, Options, PasswordPrompt, Target};
use crate::hosts::AuthMethod;

/// The message when the Agent method finds no agent.
pub const NO_AGENT: &str = "No SSH agent is running.";

/// Where the SSH agent is when the config does not say: `SSH_AUTH_SOCK` on Unix, the
/// OpenSSH agent's pipe on Windows. `None` when there is none to try.
pub fn default_agent() -> Option<PathBuf> {
    if cfg!(windows) {
        Some(PathBuf::from(r"\\.\pipe\openssh-ssh-agent"))
    } else {
        std::env::var_os("SSH_AUTH_SOCK")
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
    }
}

/// The request for a locked key file's passphrase. Dropping it cancels.
pub struct PassphrasePrompt {
    /// The key file, as the Host or the config names it.
    pub key: PathBuf,
    /// Whether the passphrase entered before did not unlock the key.
    pub rejected: bool,
    answer: oneshot::Sender<Option<String>>,
}

impl PassphrasePrompt {
    pub fn enter(self, passphrase: String) {
        let _ = self.answer.send(Some(passphrase));
    }

    /// Closes the connection; the Session fails with [`Failure::Cancelled`].
    pub fn cancel(self) {
        let _ = self.answer.send(None);
    }
}

/// One prompt of a keyboard-interactive round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    /// The host's text, such as "Verification code: ".
    pub prompt: String,
    /// Whether the answer may be shown as it is typed.
    pub echo: bool,
}

/// A keyboard-interactive round with several prompts, or one shown answer: each prompt
/// is answered in the secret dialog. Dropping it cancels.
pub struct QuestionsPrompt {
    /// Who is logging in: `account@address`.
    pub account: String,
    /// The round's name and instructions from the host; either may be empty.
    pub name: String,
    pub instructions: String,
    pub questions: Vec<Question>,
    /// Whether the host just rejected the answers given before.
    pub rejected: bool,
    answer: oneshot::Sender<Option<Vec<String>>>,
}

impl QuestionsPrompt {
    /// Sends one answer per question, in order.
    pub fn answer(self, answers: Vec<String>) {
        let _ = self.answer.send(Some(answers));
    }

    /// Closes the connection; the Session fails with [`Failure::Cancelled`].
    pub fn cancel(self) {
        let _ = self.answer.send(None);
    }
}

type Agent = AgentClient<Box<dyn AgentStream + Send + Unpin + 'static>>;

/// Logs `target.account` in by `options.auth`, asking the operator through `out`.
pub(super) async fn authenticate(
    handle: &mut Handle<HostCheck>,
    target: &Target,
    options: &Options,
    out: &Events,
) -> Result<(), Failure> {
    let result = Login {
        handle,
        target,
        out,
        methods: MethodSet::empty(),
    }
    .run(options)
    .await;
    if matches!(result, Err(Failure::Cancelled)) {
        let _ = handle
            .disconnect(Disconnect::ByApplication, "Cancelled", "en")
            .await;
    }
    result
}

struct Login<'a> {
    handle: &'a mut Handle<HostCheck>,
    target: &'a Target,
    out: &'a Events,
    /// What the host last said it would accept.
    methods: MethodSet,
}

/// A key file read from disk, and its public key when that is known without unlocking.
struct KeyFile {
    path: PathBuf,
    text: String,
    public: Option<PublicKey>,
}

impl Login<'_> {
    async fn run(&mut self, options: &Options) -> Result<(), Failure> {
        let method = options.auth;
        let user = self.target.account.clone();
        match self.handle.authenticate_none(&user).await.map_err(lost)? {
            AuthResult::Success => return Ok(()),
            AuthResult::Failure {
                remaining_methods, ..
            } => self.methods = remaining_methods,
        }
        let files = matches!(method, AuthMethod::Automatic | AuthMethod::KeyFile);
        let agents = matches!(method, AuthMethod::Automatic | AuthMethod::Agent);
        let passwords = matches!(method, AuthMethod::Automatic | AuthMethod::Password);

        let mut agent = None;
        if agents {
            if let Some(path) = &options.agent {
                agent = connect_agent(path).await;
            }
            if agent.is_none() && method == AuthMethod::Agent {
                return Err(Failure::Auth(NO_AGENT.into()));
            }
        }
        if method == AuthMethod::KeyFile && options.key_files.is_empty() {
            return Err(Failure::Auth("No key file is set for this Host.".into()));
        }

        // 1. The key files, through the agent when it holds them.
        let mut tried: Vec<PublicKey> = Vec::new();
        let mut unreadable = None;
        for path in &options.key_files {
            if !self.offers(MethodKind::PublicKey) {
                break;
            }
            let file = match read_key_file(path) {
                Ok(file) => file,
                Err(reason) => {
                    unreadable.get_or_insert(reason);
                    continue;
                }
            };
            if let Some((client, identities)) = &mut agent
                && let Some(public) = &file.public
                && let Some(identity) = identities
                    .iter()
                    .find(|i| i.public_key().key_data() == public.key_data())
                    .cloned()
            {
                tried.push(public.clone());
                if self.offer_agent(&user, client, &identity).await? {
                    return Ok(());
                }
                continue;
            }
            if !files {
                continue;
            }
            let key = match self.unlock(&file).await? {
                Ok(key) => key,
                Err(reason) => {
                    unreadable.get_or_insert(reason);
                    continue;
                }
            };
            tried.push(key.public_key().clone());
            if self.offer_key(&user, key).await? {
                return Ok(());
            }
        }

        // 2. The agent's other identities.
        if let Some((client, identities)) = &mut agent
            && !options.identities_only
        {
            for identity in identities.clone() {
                if !self.offers(MethodKind::PublicKey) {
                    break;
                }
                let public = identity.public_key();
                if tried.iter().any(|t| t.key_data() == public.key_data()) {
                    continue;
                }
                if self.offer_agent(&user, client, &identity).await? {
                    return Ok(());
                }
            }
        }

        // 3. The password.
        if passwords {
            return self.password().await;
        }
        Err(Failure::Auth(match method {
            AuthMethod::Agent => "The host accepted none of the agent's keys.".into(),
            _ => match unreadable {
                Some(reason) if tried.is_empty() => reason,
                _ => "The host did not accept the key file.".into(),
            },
        }))
    }

    fn offers(&self, kind: MethodKind) -> bool {
        self.methods.contains(&kind)
    }

    /// Whether the host accepted; a refusal updates what it still offers.
    fn accepted(&mut self, result: AuthResult) -> bool {
        match result {
            AuthResult::Success => true,
            AuthResult::Failure {
                remaining_methods, ..
            } => {
                self.methods = remaining_methods;
                false
            }
        }
    }

    async fn rsa_hash(&self, key: &PublicKey) -> Option<HashAlg> {
        if !key.algorithm().is_rsa() {
            return None;
        }
        self.handle
            .best_supported_rsa_hash()
            .await
            .ok()
            .flatten()
            .flatten()
    }

    async fn offer_key(&mut self, user: &str, key: PrivateKey) -> Result<bool, Failure> {
        let hash = self.rsa_hash(key.public_key()).await;
        let key = PrivateKeyWithHashAlg::new(Arc::new(key), hash);
        let result = self
            .handle
            .authenticate_publickey(user, key)
            .await
            .map_err(lost)?;
        Ok(self.accepted(result))
    }

    async fn offer_agent(
        &mut self,
        user: &str,
        agent: &mut Agent,
        identity: &AgentIdentity,
    ) -> Result<bool, Failure> {
        let public = identity.public_key().into_owned();
        let hash = self.rsa_hash(&public).await;
        match self
            .handle
            .authenticate_publickey_with(user, public, hash, agent)
            .await
        {
            Ok(result) => Ok(self.accepted(result)),
            // The agent could not sign: go on with the next key.
            Err(AgentAuthError::Key(_)) => Ok(false),
            Err(AgentAuthError::Send(e)) => Err(Failure::Connection(e.to_string())),
        }
    }

    /// The key in `file`, asking for its passphrase until one unlocks it. The inner
    /// error says why the file cannot be used at all.
    async fn unlock(&self, file: &KeyFile) -> Result<Result<PrivateKey, String>, Failure> {
        if !is_locked(&file.text) {
            return Ok(decode_key(&file.text, None).map_err(|e| unusable(&file.path, e)));
        }
        let mut rejected = false;
        loop {
            let (answer, reply) = oneshot::channel();
            self.out.send(Event::Passphrase(PassphrasePrompt {
                key: file.path.clone(),
                rejected,
                answer,
            }));
            let Ok(Some(passphrase)) = reply.await else {
                return Err(Failure::Cancelled);
            };
            match decode_key(&file.text, Some(&passphrase)) {
                Ok(key) => return Ok(Ok(key)),
                Err(_) => rejected = true,
            }
        }
    }

    /// Asks for the password, by keyboard-interactive when the host offers it, else by the
    /// password method, until the host accepts or the operator cancels.
    async fn password(&mut self) -> Result<(), Failure> {
        if self.offers(MethodKind::KeyboardInteractive) && self.keyboard().await? {
            return Ok(());
        }
        if !self.offers(MethodKind::Password) {
            return Err(Failure::Auth(
                "The host accepted no key and does not take a password.".into(),
            ));
        }
        let user = self.target.account.clone();
        let mut rejected = false;
        loop {
            let password = self.ask_password(rejected).await?;
            let result = self
                .handle
                .authenticate_password(&user, password)
                .await
                .map_err(lost)?;
            if self.accepted(result) {
                return Ok(());
            }
            rejected = true;
        }
    }

    async fn ask_password(&self, rejected: bool) -> Result<String, Failure> {
        let (answer, reply) = oneshot::channel();
        self.out.send(Event::Password(PasswordPrompt {
            account: format!("{}@{}", self.target.account, self.target.address),
            rejected,
            answer,
        }));
        match reply.await {
            Ok(Some(password)) => Ok(password),
            _ => Err(Failure::Cancelled),
        }
    }

    /// Runs keyboard-interactive rounds until the host accepts (true), or refuses before
    /// anything was asked (false: it is not really on offer).
    async fn keyboard(&mut self) -> Result<bool, Failure> {
        let user = self.target.account.clone();
        let mut rejected = false;
        loop {
            let mut response = self
                .handle
                .authenticate_keyboard_interactive_start(&user, None)
                .await
                .map_err(lost)?;
            let mut asked = false;
            loop {
                match response {
                    KeyboardInteractiveAuthResponse::Success => return Ok(true),
                    KeyboardInteractiveAuthResponse::Failure {
                        remaining_methods, ..
                    } => {
                        self.methods = remaining_methods;
                        if !asked {
                            self.methods.remove(MethodKind::KeyboardInteractive);
                            return Ok(false);
                        }
                        rejected = true;
                        break;
                    }
                    KeyboardInteractiveAuthResponse::InfoRequest {
                        name,
                        instructions,
                        prompts,
                    } => {
                        let answers = match prompts.as_slice() {
                            [] => Vec::new(),
                            [only] if !only.echo => vec![self.ask_password(rejected).await?],
                            _ => {
                                let questions = prompts
                                    .into_iter()
                                    .map(|p| Question {
                                        prompt: p.prompt,
                                        echo: p.echo,
                                    })
                                    .collect();
                                self.ask_questions(name, instructions, questions, rejected)
                                    .await?
                            }
                        };
                        if !answers.is_empty() {
                            asked = true;
                            rejected = false;
                        }
                        response = self
                            .handle
                            .authenticate_keyboard_interactive_respond(answers)
                            .await
                            .map_err(lost)?;
                    }
                }
            }
        }
    }

    async fn ask_questions(
        &self,
        name: String,
        instructions: String,
        questions: Vec<Question>,
        rejected: bool,
    ) -> Result<Vec<String>, Failure> {
        let (answer, reply) = oneshot::channel();
        self.out.send(Event::Questions(QuestionsPrompt {
            account: format!("{}@{}", self.target.account, self.target.address),
            name,
            instructions,
            questions,
            rejected,
            answer,
        }));
        match reply.await {
            Ok(Some(answers)) => Ok(answers),
            _ => Err(Failure::Cancelled),
        }
    }
}

fn lost(e: russh::Error) -> Failure {
    Failure::Connection(e.to_string())
}

fn unusable(path: &Path, reason: impl std::fmt::Display) -> String {
    format!("Could not use the key file {}: {reason}", path.display())
}

/// The agent at `path` and the identities it holds; `None` when it cannot be reached.
async fn connect_agent(path: &Path) -> Option<(Agent, Vec<AgentIdentity>)> {
    #[cfg(unix)]
    let client = AgentClient::connect_uds(path).await.ok()?.dynamic();
    #[cfg(windows)]
    let client = AgentClient::connect_named_pipe(path).await.ok()?.dynamic();
    let mut client: Agent = client;
    let identities = client.request_identities().await.ok()?;
    Some((client, identities))
}

fn read_key_file(path: &Path) -> Result<KeyFile, String> {
    let text = std::fs::read_to_string(path).map_err(|e| unusable(path, e))?;
    let mut pub_path = path.as_os_str().to_owned();
    pub_path.push(".pub");
    let public = keys::load_public_key(PathBuf::from(pub_path))
        .ok()
        .or_else(|| {
            // An OpenSSH key carries its public half in the clear, even when locked.
            PrivateKey::from_openssh(&text)
                .ok()
                .map(|k| k.public_key().clone())
        })
        .or_else(|| {
            (!is_locked(&text))
                .then(|| decode_key(&text, None).ok())
                .flatten()
                .map(|k| k.public_key().clone())
        });
    Ok(KeyFile {
        path: path.to_path_buf(),
        text,
        public,
    })
}

/// Whether the key file needs a passphrase.
fn is_locked(text: &str) -> bool {
    text.contains("-----BEGIN ENCRYPTED PRIVATE KEY-----")
        || text.contains("Proc-Type: 4,ENCRYPTED")
        || PrivateKey::from_openssh(text).is_ok_and(|k| k.is_encrypted())
}

/// Decodes an OpenSSH, PKCS#1, SEC1 or PKCS#8 private key, unlocking it with
/// `passphrase`. A traditional encrypted PEM key (`Proc-Type: 4,ENCRYPTED`) is decrypted
/// here, since russh reads only AES-128-CBC RSA ones.
fn decode_key(text: &str, passphrase: Option<&str>) -> Result<PrivateKey, String> {
    if text.contains("Proc-Type: 4,ENCRYPTED") {
        let plain = pem::decrypt(text, passphrase.unwrap_or_default())?;
        return keys::decode_secret_key(&plain, None).map_err(|e| e.to_string());
    }
    keys::decode_secret_key(text, passphrase).map_err(|e| e.to_string())
}

/// Traditional (OpenSSL) encrypted PEM: `Proc-Type: 4,ENCRYPTED` and a `DEK-Info`
/// header, the key derived from the passphrase with MD5 as `EVP_BytesToKey` does.
mod pem {
    use aes::cipher::block_padding::Pkcs7;
    use aes::cipher::{BlockModeDecrypt, KeyIvInit};
    use base64::Engine as _;
    use base64::engine::general_purpose::STANDARD;

    /// The same key, unencrypted, in PEM with the same label.
    pub fn decrypt(text: &str, passphrase: &str) -> Result<String, String> {
        let mut label = None;
        let mut dek = None;
        let mut body = String::new();
        for line in text.lines().map(str::trim) {
            if let Some(l) = line
                .strip_prefix("-----BEGIN ")
                .and_then(|l| l.strip_suffix("-----"))
            {
                label = Some(l.to_owned());
            } else if line.starts_with("-----END ") {
                break;
            } else if let Some(value) = line.strip_prefix("DEK-Info:") {
                dek = Some(value.trim().to_owned());
            } else if label.is_some() && !line.contains(':') {
                body.push_str(line);
            }
        }
        let label = label.ok_or("not a PEM key")?;
        let dek = dek.ok_or("no DEK-Info header")?;
        let (cipher, iv) = dek.split_once(',').ok_or("bad DEK-Info header")?;
        let iv = hex(iv.trim()).ok_or("bad DEK-Info IV")?;
        let mut data = STANDARD.decode(body).map_err(|e| e.to_string())?;
        let bad = |_| "wrong passphrase".to_string();
        let plain = match cipher {
            "AES-128-CBC" => {
                let key = derive(passphrase, &iv, 16);
                cbc::Decryptor::<aes::Aes128>::new_from_slices(&key, &iv)
                    .map_err(|e| e.to_string())?
                    .decrypt_padded::<Pkcs7>(&mut data)
                    .map_err(bad)?
                    .to_vec()
            }
            "AES-256-CBC" => {
                let key = derive(passphrase, &iv, 32);
                cbc::Decryptor::<aes::Aes256>::new_from_slices(&key, &iv)
                    .map_err(|e| e.to_string())?
                    .decrypt_padded::<Pkcs7>(&mut data)
                    .map_err(bad)?
                    .to_vec()
            }
            "DES-EDE3-CBC" => {
                let key = derive(passphrase, &iv, 24);
                cbc::Decryptor::<des::TdesEde3>::new_from_slices(&key, &iv)
                    .map_err(|e| e.to_string())?
                    .decrypt_padded::<Pkcs7>(&mut data)
                    .map_err(bad)?
                    .to_vec()
            }
            other => return Err(format!("the cipher {other} is not supported")),
        };
        let encoded = STANDARD.encode(plain);
        let mut out = format!("-----BEGIN {label}-----\n");
        for chunk in encoded.as_bytes().chunks(64) {
            out.push_str(std::str::from_utf8(chunk).unwrap_or_default());
            out.push('\n');
        }
        out.push_str(&format!("-----END {label}-----\n"));
        Ok(out)
    }

    /// `EVP_BytesToKey` with MD5, one round, the IV's first 8 bytes as the salt.
    fn derive(passphrase: &str, iv: &[u8], len: usize) -> Vec<u8> {
        let salt = &iv[..8.min(iv.len())];
        let mut key = Vec::new();
        let mut last: Vec<u8> = Vec::new();
        while key.len() < len {
            let mut c = md5::Context::new();
            c.consume(&last);
            c.consume(passphrase.as_bytes());
            c.consume(salt);
            last = c.finalize().0.to_vec();
            key.extend_from_slice(&last);
        }
        key.truncate(len);
        key
    }

    fn hex(s: &str) -> Option<Vec<u8>> {
        if !s.len().is_multiple_of(2) {
            return None;
        }
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(s.get(i..i + 2)?, 16).ok())
            .collect()
    }
}
