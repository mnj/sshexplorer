//! Logging in, by the Host's Authentication method: key files, the SSH agent, and the
//! password, which keyboard-interactive can ask for too.
//!
//! Automatic tries, in order: (1) each key file, through the agent when it holds that
//! key, else loaded from disk with its passphrase asked for when locked; (2) the agent's
//! other identities, unless `IdentitiesOnly`; (3) the password. Key file, Agent and
//! Password each keep to their own step. A method the host does not offer is skipped.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use russh::client::{AuthResult, Handle, KeyboardInteractiveAuthResponse};
use russh::keys::agent::AgentIdentity;
use russh::keys::agent::client::{AgentClient, AgentStream};
use russh::keys::ssh_key::{HashAlg, PrivateKey, PublicKey};
use russh::keys::{self, PrivateKeyWithHashAlg};
use russh::{AgentAuthError, Disconnect, MethodKind, MethodSet};
use tokio::sync::oneshot;
use zeroize::Zeroizing;

use super::{Event, Events, Failure, HostCheck, Options, PasswordPrompt, Saved, Target};
use crate::credentials::{self, Kind, Lookup};
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
    /// What to say of the credential store.
    pub saved: Saved,
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

/// A secret the operator entered that logged in, to keep when Remember is on.
pub(super) type Kept = (Kind, Zeroizing<String>);

/// The secrets a Session logged in with, one per kind, offered first when it reconnects.
pub(super) type Held = HashMap<Kind, Zeroizing<String>>;

/// What logging in gave: the entered secret to keep under Remember, and every secret
/// that worked, for reconnecting.
pub(super) struct LoggedIn {
    pub kept: Option<Kept>,
    pub used: Held,
}

/// A secret, and whether it came from the credential store rather than the operator.
struct Secret {
    text: Zeroizing<String>,
    saved: bool,
}

/// How far a Session has got with the store's secret of one kind.
enum Stage {
    /// Read and sent; asking for this kind again after a rejection means the store's
    /// secret was rejected.
    Tried(Zeroizing<String>),
    /// No more from the store; the next prompt says `Saved`.
    Done(Saved),
}

/// Logs `target.account` in by `options.auth`, asking the operator through `out`. A
/// secret in `held` is sent before the store's or the operator's. A host that closes
/// the connection after its own attempt limit is reconnected to quietly. Gives the
/// secret the operator entered that logged in, if any, and the secrets that worked.
pub(super) async fn authenticate(
    handle: &mut Handle<HostCheck>,
    target: &Target,
    options: &Options,
    held: &Held,
    out: &Arc<Events>,
) -> Result<LoggedIn, Failure> {
    let mut login = Login {
        handle,
        target,
        options,
        out,
        held,
        methods: MethodSet::empty(),
        stages: HashMap::new(),
        kept: None,
        used: Held::new(),
    };
    let result = login.run(options).await;
    let kept = login.kept.take();
    let used = std::mem::take(&mut login.used);
    if matches!(result, Err(Failure::Cancelled)) {
        let _ = handle
            .disconnect(Disconnect::ByApplication, "Cancelled", "en")
            .await;
    }
    result.map(|()| LoggedIn { kept, used })
}

struct Login<'a> {
    handle: &'a mut Handle<HostCheck>,
    target: &'a Target,
    options: &'a Options,
    out: &'a Arc<Events>,
    /// The secrets the Session logged in with before, tried first.
    held: &'a Held,
    /// What the host last said it would accept.
    methods: MethodSet,
    /// The store's secrets, once looked up.
    stages: HashMap<Kind, Stage>,
    /// The entered secret that logged in.
    kept: Option<Kept>,
    /// Every secret that logged in, entered or not.
    used: Held,
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
            let (key, passphrase) = match self.unlock(&file).await? {
                Ok(unlocked) => unlocked,
                Err(reason) => {
                    unreadable.get_or_insert(reason);
                    continue;
                }
            };
            tried.push(key.public_key().clone());
            if self.offer_key(&user, key).await? {
                self.keep(Kind::Passphrase, passphrase);
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

    /// Notes `secret` as the one that logged in, when the operator entered it.
    fn keep(&mut self, kind: Kind, secret: Option<Secret>) {
        let Some(secret) = secret else {
            return;
        };
        self.used.insert(kind, secret.text.clone());
        if !secret.saved {
            self.kept = Some((kind, secret.text));
        }
    }

    /// The secret of `kind`: the one the Session held or else the store's, the first
    /// time it is asked for, else the operator's through the prompt `ask` makes.
    /// `rejected` says the last one sent was rejected.
    async fn secret(
        &mut self,
        kind: Kind,
        rejected: bool,
        ask: impl FnOnce(Saved, oneshot::Sender<Option<String>>) -> Event,
    ) -> Result<Secret, Failure> {
        let mut saved = Saved::Nothing;
        let stage = match self.stages.remove(&kind) {
            Some(stage) => Some(stage),
            None => match (self.held.get(&kind), &self.options.remember) {
                (Some(text), _) => Some(Stage::Tried(text.clone())),
                (None, Some(remember)) => {
                    let store = remember.store.clone();
                    let key = credentials::Key::new(remember.host.clone(), kind);
                    Some(
                        match tokio::task::spawn_blocking(move || store.get(&key)).await {
                            Ok(Lookup::Found(text)) => Stage::Tried(text),
                            Ok(Lookup::NoEntry) => Stage::Done(Saved::Nothing),
                            _ => Stage::Done(Saved::Unavailable),
                        },
                    )
                }
                (None, None) => None,
            },
        };
        if let Some(stage) = stage {
            match stage {
                // The first request, or another key file asking: the held or store's
                // secret.
                Stage::Tried(text) if !rejected => {
                    self.stages.insert(kind, Stage::Tried(text.clone()));
                    return Ok(Secret { text, saved: true });
                }
                Stage::Tried(_) if self.options.remember.is_some() => saved = Saved::Rejected,
                Stage::Tried(_) => {}
                Stage::Done(first) => saved = first,
            }
            self.stages.insert(kind, Stage::Done(Saved::Nothing));
        }
        let (answer, reply) = oneshot::channel();
        self.out.send(ask(saved, answer));
        match reply.await {
            Ok(Some(text)) => Ok(Secret {
                text: Zeroizing::new(text),
                saved: false,
            }),
            _ => Err(Failure::Cancelled),
        }
    }

    /// The key in `file`, asking for its passphrase until one unlocks it, and the
    /// passphrase that did. The inner error says why the file cannot be used at all.
    async fn unlock(
        &mut self,
        file: &KeyFile,
    ) -> Result<Result<(PrivateKey, Option<Secret>), String>, Failure> {
        if !is_locked(&file.text) {
            return Ok(decode_key(&file.text, None)
                .map(|key| (key, None))
                .map_err(|e| unusable(&file.path, e)));
        }
        let mut rejected = false;
        loop {
            let path = file.path.clone();
            let passphrase = self
                .secret(Kind::Passphrase, rejected, |saved, answer| {
                    Event::Passphrase(PassphrasePrompt {
                        key: path,
                        rejected,
                        saved,
                        answer,
                    })
                })
                .await?;
            match decode_key(&file.text, Some(&passphrase.text)) {
                Ok(key) => return Ok(Ok((key, Some(passphrase)))),
                Err(_) => rejected = true,
            }
        }
    }

    /// Connects again after the host closed the connection mid-login, as it does past
    /// its own attempt limit. Whether the host let the account in without asking.
    async fn redial(&mut self) -> Result<bool, Failure> {
        let mut handle = super::dial(self.target, self.options, self.out).await?;
        let user = self.target.account.clone();
        let logged_in = match handle.authenticate_none(&user).await.map_err(lost)? {
            AuthResult::Success => true,
            AuthResult::Failure {
                remaining_methods, ..
            } => {
                self.methods = remaining_methods;
                false
            }
        };
        *self.handle = handle;
        Ok(logged_in)
    }

    /// Sends `password` by the password method, reconnecting once if the host closed
    /// the connection.
    async fn send_password(&mut self, password: &str) -> Result<bool, Failure> {
        let user = self.target.account.clone();
        let result = match self.handle.authenticate_password(&user, password).await {
            Ok(result) => result,
            Err(_) => {
                if self.redial().await? {
                    return Ok(true);
                }
                self.handle
                    .authenticate_password(&user, password)
                    .await
                    .map_err(lost)?
            }
        };
        Ok(self.accepted(result))
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
        let mut rejected = false;
        loop {
            let password = self.ask_password(rejected).await?;
            if self.send_password(&password.text).await? {
                self.keep(Kind::Password, Some(password));
                return Ok(());
            }
            rejected = true;
        }
    }

    async fn ask_password(&mut self, rejected: bool) -> Result<Secret, Failure> {
        let account = format!("{}@{}", self.target.account, self.target.address);
        self.secret(Kind::Password, rejected, |saved, answer| {
            Event::Password(PasswordPrompt {
                account,
                rejected,
                saved,
                answer,
            })
        })
        .await
    }

    /// Starts a keyboard-interactive round, reconnecting once if the host closed the
    /// connection. `None` when the reconnection let the account in.
    async fn start_keyboard(&mut self) -> Result<Option<KeyboardInteractiveAuthResponse>, Failure> {
        let user = self.target.account.clone();
        match self
            .handle
            .authenticate_keyboard_interactive_start(&user, None)
            .await
        {
            Ok(response) => Ok(Some(response)),
            Err(_) => {
                if self.redial().await? {
                    return Ok(None);
                }
                self.handle
                    .authenticate_keyboard_interactive_start(&user, None)
                    .await
                    .map(Some)
                    .map_err(lost)
            }
        }
    }

    /// Runs keyboard-interactive rounds until the host accepts (true), or refuses before
    /// anything was asked (false: it is not really on offer).
    async fn keyboard(&mut self) -> Result<bool, Failure> {
        let mut rejected = false;
        loop {
            let Some(mut response) = self.start_keyboard().await? else {
                return Ok(true);
            };
            let mut asked = false;
            // The password this round was answered with, when it asked for one.
            let mut password = None;
            loop {
                match response {
                    KeyboardInteractiveAuthResponse::Success => {
                        self.keep(Kind::Password, password);
                        return Ok(true);
                    }
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
                            [only] if !only.echo && asks_password(&only.prompt) => {
                                let secret = self.ask_password(rejected).await?;
                                let answer = secret.text.to_string();
                                password = Some(secret);
                                vec![answer]
                            }
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

/// Whether a keyboard-interactive prompt asks for the password (/password/i): the only
/// prompt whose answer is remembered.
fn asks_password(prompt: &str) -> bool {
    prompt.to_lowercase().contains("password")
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
