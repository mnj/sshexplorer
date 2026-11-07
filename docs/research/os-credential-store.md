# OS credential store: what a Rust desktop app observes

Question: how a Rust desktop app stores and reads a secret in the operating system's credential store on Linux (Secret Service), macOS (Keychain), and Windows (Credential Manager). For each platform: the crates, what the operator must have running, and what the app observes when the store is missing, locked, or the operator denies the prompt.

Read on 2026-11-07 from the crate sources and the platform docs linked below. This file records those facts. It does not decide what a Session does when the store cannot keep the secret.

Vocabulary: the person at the machine is the Operator. Remember is a Host setting. The OS may still ask for its own keychain or collection password. That prompt belongs to the OS store.

## The stack a desktop app gets

[`keyring` 4.2.0](https://crates.io/crates/keyring/4.2.0) (published 2026-08-29, MIT OR Apache-2.0, rust-version 1.88, edition 2024) is the current stable crate. Its default feature is `v1`. That feature pulls in exactly three stores: `apple-native-keyring-store/keychain`, `windows-native-keyring-store`, and `zbus-secret-service-keyring-store`. It depends on `keyring-core` 1.0. ([`keyring` 4.2.0 `Cargo.toml.orig`](https://docs.rs/crate/keyring/4.2.0/source/Cargo.toml.orig))

`v1` selects one store for the process ([`keyring` 4.2.0 `src/v1.rs`](https://docs.rs/crate/keyring/4.2.0/source/src/v1.rs)):

- macOS: `apple_native_keyring_store::keychain::Store::new`
- Windows: `windows_native_keyring_store::Store::new`
- other Unix, excluding iOS and Android: `zbus_secret_service_keyring_store::Store::new`
- anything else: `Error::Invalid` on the parameter `platform`

The `cli` feature adds `dbus-secret-service-keyring-store`, `linux-keyutils-keyring-store`, and `apple-native-keyring-store/protected`. Those are not on the `v1` path. `linux-keyutils` is the kernel keyring, which is a different store from Secret Service. The `protected` feature is the data-protection keychain and needs a provisioning profile; `v1` uses the `keychain` feature instead.

The call the app makes is `keyring::Entry::new(service, username)`, then `set_password` / `set_secret`, `get_password` / `get_secret`, and `delete_credential`. `Entry::new` and `Entry::store_status` share one process-wide `LazyLock`. The first initialization is kept for the life of the process. ([`keyring` 4.2.0 `src/v1.rs`](https://docs.rs/crate/keyring/4.2.0/source/src/v1.rs))

If that first initialization returns an error, `Entry::new` returns `Error::NoDefaultStore` and drops the cause. `Entry::store_status` keeps the original `Result`. A later call in the same process still sees the cached failure, including after a Linux daemon appears.

`keyring::Error` is a re-export of [`keyring_core::Error`](https://docs.rs/crate/keyring-core/1.0.1/source/src/error.rs) ([`keyring-core` 1.0.1](https://crates.io/crates/keyring-core/1.0.1), published 2026-10-13). The enum is `non_exhaustive`. The variants a desktop app hits here:

| Variant | What the crate says it means |
| --- | --- |
| `NoEntry` | No credential was stored, or it was deleted. |
| `NoStorageAccess(err)` | The store holding saved items could not be accessed. The docs give a locked store as the example. The platform error is attached. |
| `PlatformFailure(err)` | A runtime failure in the platform store. The platform error is attached. |
| `NoDefaultStore` | No default store is set, so entries cannot be created. In `v1` this is also how a failed one-time init is reported. |
| `BadEncoding(bytes)` | The blob is not the text encoding that `get_password` expected. |
| `Invalid(name, reason)` | A parameter is invalid. |
| `Ambiguous(entries)` | More than one credential matched. |
| `TooLong(name, limit)` | A platform length limit was exceeded. |

`source()` on `PlatformFailure` and `NoStorageAccess` is the boxed platform error. ([`keyring-core` 1.0.1 `src/error.rs`](https://docs.rs/crate/keyring-core/1.0.1/source/src/error.rs))

A secret that was never stored is `NoEntry` on all three platforms. That is a different result from a missing store.

## Linux: Secret Service

### Crates

`v1` uses [`zbus-secret-service-keyring-store` 1.0.1](https://crates.io/crates/zbus-secret-service-keyring-store/1.0.1) (published 2026-08-15). It depends on `secret-service` 5 and `zbus` 5. ([`Cargo.toml.orig`](https://docs.rs/crate/zbus-secret-service-keyring-store/1.0.1/source/Cargo.toml.orig))

`secret-service = "5"` stays on the 5.x line. The current stable 5.x is [`secret-service` 5.2.0](https://crates.io/crates/secret-service) (published 2026-08-29, `max_stable_version` 5.2.0). Crates.io also lists `6.0.0-rc.1` (published 2026-11-06). The `v1` dependency does not select that release candidate. The behavior below is the 5.2.0 source.

The store connects with `SecretService::connect(EncryptionType::Dh)` inside a `Mutex`, and maps every connect error with `platform_failure`. ([`service.rs` `Service::new`](https://docs.rs/crate/zbus-secret-service-keyring-store/1.0.1/source/src/service.rs))

`secret-service` opens the session bus, builds a proxy, then opens a cryptographic session. The proxy's well-known name is `org.freedesktop.secrets`, path `/org/freedesktop/secrets`, interface `org.freedesktop.Secret.Service`. The same bus name is `SS_DBUS_NAME`. ([`SecretService::connect`](https://docs.rs/crate/secret-service/5.2.0/source/src/lib.rs), [`proxy/service.rs`](https://docs.rs/crate/secret-service/5.2.0/source/src/proxy/service.rs), [`ss.rs`](https://docs.rs/crate/secret-service/5.2.0/source/src/ss.rs))

### What the operator must have running

The Secret Service API is a service in the user's login session. Prompts are displayed by the service, not by the app. ([Secret Service API Draft, publication 2026-10-19, version 0.2 DRAFT](https://specifications.freedesktop.org/secret-service/latest/), [introduction](https://specifications.freedesktop.org/secret-service/latest/description.html), [prompts](https://specifications.freedesktop.org/secret-service/0.2/prompts.html))

The spec names its authors as the GNOME Keyring developer and the KWallet developer. The `secret-service` crate docs name GNOME Keyring and KWallet as implementations. The spec does not require one particular daemon. On a desktop session the desktop starts the service. The operator does not start a daemon by hand for that case.

The object path of the service is `/org/freedesktop/secrets`. The default collection is also available at `/org/freedesktop/secrets/aliases/default`. ([object paths](https://specifications.freedesktop.org/secret-service-spec/latest/object-paths.html))

Headless is a separate case. The store's own docs say zbus, secret-service, and GNOME Keyring have known trouble there, and they show `echo -n "$pass" | gnome-keyring-daemon --unlock`. ([`lib.rs` "Headless usage"](https://docs.rs/crate/zbus-secret-service-keyring-store/1.0.1/source/src/lib.rs))

The same crate docs say WSL has no `default` collection, so every entry needs a non-`default` `target` modifier. They also say KDE Wallet reads and writes UTF-8 only, so a binary secret has to be encoded (they give base64) before it can be stored there.

### What the app calls

The default entry is a specifier, not a wrapper. Attributes are `service`, `username`, and `target` when a target modifier is set. The default label is `keyring:{user}@{service}`. Search uses `SearchItems` across collections. Zero matches is `NoEntry`. More than one match is `Ambiguous`. A new item is created in the collection named by `target`, or in `"default"` when `target` is unset. Content type is `application/octet-stream`. ([`cred.rs`](https://docs.rs/crate/zbus-secret-service-keyring-store/1.0.1/source/src/cred.rs))

`SearchItems` returns unlocked paths and locked paths separately. Locked matches require authentication. ([lookup attributes](https://specifications.freedesktop.org/secret-service/0.2/lookup-attributes.html), [`org.freedesktop.Secret.Service`](https://specifications.freedesktop.org/secret-service/latest/org.freedesktop.Secret.Service.html))

The specifier path, which `Entry` uses:

1. `find_matching_items` searches, then calls `unlock_all` when any hit is locked. Both steps use `decode_error`.
2. `get_secret` calls `GetSecret` and uses `decode_error`. It does not call `ensure_unlocked` first.
3. `set_secret` updates an existing item the same way. On `NoEntry` it creates an item. `get_collection` unlocks a locked collection through `decode_error`. `Collection::create_item` is mapped with `platform_failure`, not `decode_error`.

([`service.rs`](https://docs.rs/crate/zbus-secret-service-keyring-store/1.0.1/source/src/service.rs))

The wrapper type does call `ensure_unlocked` before reading. `v1`'s `Entry` does not use that type for get and set.

### How secret-service reports the prompt

`Unlock` returns the paths it could unlock immediately and a prompt for the rest. The prompt's result lists what the prompt unlocked. A service may lock items again at any time. Secrets of locked items cannot be accessed, and locked items cannot be modified. ([locking](https://specifications.freedesktop.org/secret-service-spec/latest/unlocking.html))

The crate calls `Prompt("")` with an empty window id, then waits for `Completed`. `dismissed == true` becomes `Error::Prompt` (`"SS error: prompt dismissed"`). The signal stream ending without `Completed` becomes `Error::PromptDisconnected` (`"SS error: connection closed before prompt completed"`). The spec says a dismissed prompt cancels the operation, and `Completed` is emitted with `dismissed` TRUE. ([`util.rs`](https://docs.rs/crate/secret-service/5.2.0/source/src/util.rs), [prompts](https://specifications.freedesktop.org/secret-service/0.2/prompts.html))

`Collection::unlock` and `Item::unlock` call `Unlock` and run the prompt when the unlocked-path list is empty. `ensure_unlocked` does not prompt: if the `Locked` property is true it returns `Error::Locked` (`"SS Error: object locked"`). Comments on `delete` that say `ensure_unlocked` prompts are contradicted by that function body. `GetSecret` itself does not map the D-Bus error `org.freedesktop.Secret.Error.IsLocked` onto `Error::Locked`. That D-Bus name means the object must be unlocked before the action. ([`collection.rs`](https://docs.rs/crate/secret-service/5.2.0/source/src/collection.rs), [`item.rs`](https://docs.rs/crate/secret-service/5.2.0/source/src/item.rs), [errors](https://specifications.freedesktop.org/secret-service/0.2/errors.html))

`CreateItem` returns an item path and a prompt. The crate treats an item path of `/` as "run the prompt". `CreateCollection` in the spec uses `/` when prompting is necessary. ([`collection.rs` `create_item`](https://docs.rs/crate/secret-service/5.2.0/source/src/collection.rs), [`org.freedesktop.Secret.Collection`](https://specifications.freedesktop.org/secret-service-spec/0.2/org.freedesktop.Secret.Collection.html))

`ReadAlias` of a missing alias returns the path `/`, which the crate turns into `Error::NoResult`. `get_default_collection` is `get_collection_by_alias("default")`. ([`lib.rs`](https://docs.rs/crate/secret-service/5.2.0/source/src/lib.rs))

`handle_conn_error` maps `zbus::Error::Address`, `InterfaceNotFound`, and an I/O `NotFound` to `Error::Unavailable`. Every other `zbus::Error` stays `Error::Zbus`. `Unavailable`'s `Display` is `"no secret service provider or dbus session found"`. The rustdoc comment on that variant says the opposite ("was found"). The `Display` impl is the string the app can show. ([`error.rs`](https://docs.rs/crate/secret-service/5.2.0/source/src/error.rs), [`util.rs` `handle_conn_error`](https://docs.rs/crate/secret-service/5.2.0/source/src/util.rs))

### What the app observes

`decode_error` maps `Locked`, `NoResult`, and `Prompt` to `NoStorageAccess`. Every other `secret_service::Error`, including `Unavailable`, `PromptDisconnected`, `Zbus`, and `ZbusFdo`, becomes `PlatformFailure`. ([`errors.rs`](https://docs.rs/crate/zbus-secret-service-keyring-store/1.0.1/source/src/errors.rs))

| Situation | What the app observes |
| --- | --- |
| Session bus missing, or another connect failure, on the first `Entry::new` | `Service::new` wraps the connect error as `PlatformFailure`. `Entry::new` returns only `NoDefaultStore`. `store_status` keeps the `PlatformFailure`. The `LazyLock` keeps that result for the process. |
| Same failure after a successful init (daemon dies later) | An operation error: `PlatformFailure`, or `NoStorageAccess` when the secret-service error is one of the three mapped variants. It is not `NoDefaultStore`. |
| No item with those attributes | `NoEntry`. |
| `default` alias missing (WSL, or no login collection). Read of a never-stored secret | Search misses and returns `NoEntry`. The read does not require the alias. |
| `default` alias missing, and the write has to create an item in `default` | `ReadAlias` yields `/`, `NoResult`, then `NoStorageAccess`. This is not `NoEntry`. |
| Collection or search hit is locked, and the operator dismisses the unlock prompt | `Error::Prompt`, then `NoStorageAccess`. |
| `CreateItem` returns a prompt and the operator dismisses it | `Error::Prompt` travels through `platform_failure`, so the app sees `PlatformFailure`. A dismissed unlock that happened earlier, during search or `get_collection`, is still `NoStorageAccess`. |
| Prompt connection closes before `Completed` | `PromptDisconnected`, then `PlatformFailure`. |
| `GetSecret` returns D-Bus `IsLocked` | A zbus error, then `PlatformFailure`. The specifier path unlocks before `GetSecret`, so this is the leftover case, not the usual locked read. |
| Empty target modifier | `Invalid` on `target`. |

The service may ask for a collection password. That prompt is the service's. The app passes an empty window id and waits.

## macOS: Keychain

### Crates

`v1` uses the `keychain` feature of [`apple-native-keyring-store` 1.0.2](https://crates.io/crates/apple-native-keyring-store/1.0.2) (published 2026-08-06, rust-version 1.85). That feature enables `security-framework` 3.7. The source read here is [`security-framework` 3.7.0](https://docs.rs/crate/security-framework/3.7.0/source/src/os/macos/passwords.rs). ([`Cargo.toml.orig`](https://docs.rs/crate/apple-native-keyring-store/1.0.2/source/Cargo.toml.orig))

`Store::new` records the User (login) keychain and does not touch the OS. A missing or locked keychain shows up on set or get, not as `NoDefaultStore`. The OS creates the User keychain. The operator does not install a credential daemon. The module also names System, Common, and Dynamic, selected with a `keychain` modifier. ([`keychain.rs`](https://docs.rs/crate/apple-native-keyring-store/1.0.2/source/src/keychain.rs))

An entry is a generic password. `service` and `account` identify it. Empty `service` or empty `user` is `Error::Invalid`, because the Keychain Services API treats an empty attribute as a wildcard.

### What the OS does on get and set

`get_secret` calls `find_generic_password`, which calls `SecKeychainFindGenericPassword`. Apple's discussion: the function decrypts the password; if the app is not in the trusted list, the user is prompted; if the access controls disallow decryption, the result is `errSecAuthFailed`; if the keychain is locked, the function calls `SecKeychainUnlock` and shows Unlock Keychain. ([`SecKeychainFindGenericPassword`](https://developer.apple.com/documentation/security/seckeychainfindgenericpassword(_:_:_:_:_:_:_:_:)))

`set_secret` calls `SecKeychain::set_generic_password`. That function first finds the item. On `Ok` it updates the password. On every error, the `_` arm, it calls `add_generic_password`, which calls `SecKeychainAddGenericPassword`. A denied or cancelled find during set is discarded, and an add is attempted. The error the app sees is the add's error. ([`passwords.rs`](https://docs.rs/crate/security-framework/3.7.0/source/src/os/macos/passwords.rs))

`SecKeychainAddGenericPassword` sets the initial access so the creating app is trusted, and it also unlocks a locked keychain by showing Unlock Keychain. ([`SecKeychainAddGenericPassword`](https://developer.apple.com/documentation/security/seckeychainaddgenericpassword(_:_:_:_:_:_:_:_:)))

The apple store does not call `SecKeychainSetUserInteractionAllowed` and does not pass a window id. Apple's default is that interaction is allowed. If interaction is turned off, functions that would show UI return an error; the discussion names `errSecInteractionRequired`. `security-framework`'s own test expects `user_interaction_allowed()` to be true until something disables it. ([`SecKeychainSetUserInteractionAllowed`](https://developer.apple.com/documentation/security/seckeychainsetuserinteractionallowed(_:)?language=objc), [`keychain.rs`](https://docs.rs/crate/security-framework/3.7.0/source/src/os/macos/keychain.rs))

Access-control UI, when it appears, offers Deny, Allow, or Always Allow. Always Allow adds the app to the trusted list. ([Access Control Lists](https://developer.apple.com/documentation/security/access-control-lists))

### OSStatus values and the mapping

Numbers and comments are from Apple's `SecBase.h` in [apple-oss-distributions Security-59754](https://github.com/apple-oss-distributions/Security/blob/rel/Security-59754/base/SecBase.h):

| Code | Constant | Header comment | `decode_error` |
| --- | --- | --- | --- |
| -61 | `errSecWrPerm` | Write permissions error. | `NoStorageAccess` |
| -128 | `errSecUserCanceled` | User canceled the operation. | `PlatformFailure` |
| -25243 | `errSecNoAccessForItem` | The specified item has no access control. | `PlatformFailure` |
| -25244 | `errSecInvalidOwnerEdit` | Invalid attempt to change the owner of this item. | `NoStorageAccess` |
| -25291 | `errSecNotAvailable` | No keychain is available. You may need to restart your computer. | `NoStorageAccess` |
| -25292 | `errSecReadOnly` | This keychain cannot be modified. | `NoStorageAccess` |
| -25293 | `errSecAuthFailed` | The user name or passphrase you entered is not correct. | `PlatformFailure` |
| -25294 | `errSecNoSuchKeychain` | The specified keychain could not be found. | `NoStorageAccess` |
| -25295 | `errSecInvalidKeychain` | The specified keychain is not a valid keychain file. | `NoStorageAccess` |
| -25299 | `errSecDuplicateItem` | The specified item already exists in the keychain. | `PlatformFailure` |
| -25300 | `errSecItemNotFound` | The specified item could not be found in the keychain. | `NoEntry` |
| -25307 | `errSecNoDefaultKeychain` | A default keychain could not be found. | `PlatformFailure` |
| -25308 | `errSecInteractionNotAllowed` | User interaction is not allowed. | `PlatformFailure` |
| -25315 | `errSecInteractionRequired` | User interaction is required, but is currently not allowed. | `PlatformFailure` |

`decode_error` matches the six `NoStorageAccess` codes and `-25300`. Everything else is `PlatformFailure` wrapping `security_framework::base::Error`. `code()` on that error is the `OSStatus`. ([`keychain.rs` `decode_error`](https://docs.rs/crate/apple-native-keyring-store/1.0.2/source/src/keychain.rs))

| Situation | What the app observes |
| --- | --- |
| Item was never stored | `errSecItemNotFound` (-25300), then `NoEntry`. |
| No such keychain, invalid keychain, keychain not available, read-only keychain, invalid owner edit, write permission | `NoStorageAccess`. |
| No default keychain (-25307) | `PlatformFailure`. This code is not in the `NoStorageAccess` list. |
| Keychain locked, interaction left at the default | The OS shows Unlock Keychain from find and from add. The app does not draw that dialog. |
| Operator cancels (-128), or the OS returns auth failed (-25293) | `PlatformFailure`. Neither code is `NoStorageAccess`. Apple's find discussion names `errSecAuthFailed` when access controls disallow decryption. The header defines cancel separately. This file does not assign a particular button to a particular code beyond those texts. |
| Interaction disabled by something else in the process | Apple returns an error instead of UI. `errSecInteractionRequired` (-25315) and `errSecInteractionNotAllowed` (-25308) both become `PlatformFailure`. The `v1` store does not disable interaction. |
| Set, and find fails for any reason including cancel or auth failure | The find error is dropped. Add runs. The observed error is the add's status. If the item already exists, the header's duplicate-item code is -25299, which this store maps to `PlatformFailure`. |

## Windows: Credential Manager

### Crates

`v1` uses [`windows-native-keyring-store` 1.1.0](https://crates.io/crates/windows-native-keyring-store/1.1.0) (published 2026-05-24, rust-version 1.88). It calls `CredWriteW`, `CredReadW`, `CredDeleteW`, and `CredFree` from `windows_sys`, with `CRED_TYPE_GENERIC`. Flags passed to read and write are 0. ([`utils.rs`](https://docs.rs/crate/windows-native-keyring-store/1.1.0/source/src/utils.rs))

`Store::new` only records delimiters. It does not call the OS, so a missing credential set is not `NoDefaultStore`. The default delimiters are prefix `""`, divider `"."`, suffix `""`. The target name is `{prefix}{user}{divider}{service}{suffix}`, which is `{user}.{service}` unless a `target` modifier is set. An explicit target is used as-is. The default persistence modifier is the string `"Enterprise"`, parsed as `CRED_PERSIST_ENTERPRISE`. ([`store.rs`](https://docs.rs/crate/windows-native-keyring-store/1.1.0/source/src/store.rs), [`cred.rs`](https://docs.rs/crate/windows-native-keyring-store/1.1.0/source/src/cred.rs))

`CredentialStoreApi::persistence` returns `CredentialPersistence::UntilDelete`, and its comment says credentials stay in kernel memory and vanish on reboot. The write path above passes `CRED_PERSIST_ENTERPRISE`. Those two statements in the same file disagree. The bytes written use the Enterprise value. Microsoft defines that value separately, below.

`set_password` stores UTF-16LE with no trailing NUL. `set_secret` stores the raw bytes. `get_password` requires even length and valid UTF-16; otherwise `BadEncoding`. `get_secret` returns the raw blob. An empty target is `Invalid`. Length limits that become `TooLong` are `CRED_MAX_USERNAME_LENGTH` (513), `CRED_MAX_GENERIC_TARGET_NAME_LENGTH` (32767), `CRED_MAX_CREDENTIAL_BLOB_SIZE` (5*512), and `CRED_MAX_STRING_LENGTH` (256).

### What the operator must have running

Credential Manager here is the Win32 API in `Advapi32.dll`, for desktop apps, Windows XP and later. There is no extra process to start. `CredRead` reads the credential set of the current token's logon session. The token must not have the user's SID disabled. `CredWrite` creates or replaces a credential in that same set, keyed by `TargetName` plus `Type`. `TargetName` is case-insensitive. For `CRED_TYPE_GENERIC`, `UserName` and `TargetAlias` may be non-NULL, and Credential Manager ignores them. ([`CredReadW`](https://learn.microsoft.com/en-us/windows/win32/api/wincred/nf-wincred-credreadw), [`CredWriteW`](https://learn.microsoft.com/en-us/windows/win32/api/wincred/nf-wincred-credwritew), [`CREDENTIALW`](https://learn.microsoft.com/en-us/windows/win32/api/wincred/ns-wincred-credentialw), page `updated_at` 2025-11-25)

A desktop logon has a credential set. The API's name for a missing set is `ERROR_NO_SUCH_LOGON_SESSION`: the logon session does not exist, or it has no credential set. Network logon sessions have none.

`CRED_PERSIST_SESSION` (1) lasts for this logon. `CRED_PERSIST_LOCAL_MACHINE` (2) persists on this computer across logons and is not shared to other computers. `CRED_PERSIST_ENTERPRISE` (3) persists across logons and is visible to other computers for this user, and it stays local when the account has no roaming profile. Microsoft marks Local Machine and Enterprise unsupported on Windows Vista Home Basic, Vista Home Premium, Vista Starter, and Windows XP Home Edition. The generic blob is application-defined. ([`CREDENTIALW`](https://learn.microsoft.com/en-us/windows/win32/api/wincred/ns-wincred-credentialw))

### What the app observes

`decode_error` maps `ERROR_NOT_FOUND` to `NoEntry` and `ERROR_NO_SUCH_LOGON_SESSION` to `NoStorageAccess`. Every other Win32 code is `PlatformFailure` carrying that code. ([`utils.rs`](https://docs.rs/crate/windows-native-keyring-store/1.1.0/source/src/utils.rs))

| Situation | What the app observes |
| --- | --- |
| No credential with that target name | `CredReadW` returns `ERROR_NOT_FOUND`, then `NoEntry`. |
| No credential set for this logon | `CredReadW` or `CredWriteW` returns `ERROR_NO_SUCH_LOGON_SESSION`, then `NoStorageAccess`. |
| Operator denies a prompt | These generic read and write calls do not show a prompt. There is no deny result. `CRED_FLAGS_PROMPT_NOW` is ignored on input. Microsoft describes it for domain-password and domain-certificate credentials, where an authentication package fails and the application prompts. The crate sets `Flags` to 0 and uses `CRED_TYPE_GENERIC`. |
| Any other Win32 status | `PlatformFailure` with that code. |

`ERROR_NOT_FOUND` on `CredWriteW` is documented for `CRED_PRESERVE_CREDENTIAL_BLOB` when no credential exists. The crate does not pass that flag.

The Win32 API has no locked state for this generic credential. A locked-store result on Linux or macOS has no counterpart here.

## Side by side

| | Linux Secret Service | macOS Keychain | Windows Credential Manager |
| --- | --- | --- | --- |
| `v1` crates | `zbus-secret-service-keyring-store` 1.0.1, `secret-service` 5.2.0, `zbus` 5 | `apple-native-keyring-store` 1.0.2 `keychain`, `security-framework` 3.7 | `windows-native-keyring-store` 1.1.0 |
| Operator's machine | A session bus and a Secret Service implementation on `org.freedesktop.secrets`. A desktop session starts it. | The login keychain the OS creates. No extra daemon. | The logon session's credential set. No extra process. |
| First init can fail | Yes. Connect runs inside `Store::new`. | No. `Store::new` does not call the OS. | No. `Store::new` does not call the OS. |
| Secret never stored | `NoEntry` | `NoEntry` (`-25300`) | `NoEntry` (`ERROR_NOT_FOUND`) |
| Store missing | First `Entry::new`: `NoDefaultStore`. `store_status`: `PlatformFailure` around the connect error. Missing `default` alias on a later write: `NoStorageAccess`. | No such / invalid / unavailable keychain: `NoStorageAccess`. No default keychain (`-25307`): `PlatformFailure`. | No credential set: `NoStorageAccess` (`ERROR_NO_SUCH_LOGON_SESSION`). |
| Locked | Service shows the prompt. Dismissed unlock during search or collection unlock: `NoStorageAccess`. | OS shows Unlock Keychain. The app waits on the Security framework call. | No locked state on this API. |
| Operator denies the prompt | Dismissed unlock: `NoStorageAccess`. Dismissed `CreateItem` prompt: `PlatformFailure`. | Cancel (`-128`) or auth failed (`-25293`): `PlatformFailure`. A set may surface the add's error instead, including duplicate item (`-25299`) as `PlatformFailure`. | No prompt, so no deny result. |

## Sources

Crate index and the versions current on 2026-11-07:

- https://crates.io/crates/keyring/4.2.0
- https://crates.io/crates/keyring-core/1.0.1
- https://crates.io/crates/zbus-secret-service-keyring-store/1.0.1
- https://crates.io/crates/secret-service (`max_stable_version` 5.2.0; `6.0.0-rc.1` is not selected by `secret-service = "5"`)
- https://crates.io/crates/apple-native-keyring-store/1.0.2
- https://crates.io/crates/windows-native-keyring-store/1.1.0

Crate source on docs.rs:

- https://docs.rs/crate/keyring/4.2.0/source/Cargo.toml.orig
- https://docs.rs/crate/keyring/4.2.0/source/src/v1.rs
- https://docs.rs/crate/keyring-core/1.0.1/source/src/error.rs
- https://docs.rs/crate/zbus-secret-service-keyring-store/1.0.1/source/src/lib.rs
- https://docs.rs/crate/zbus-secret-service-keyring-store/1.0.1/source/src/errors.rs
- https://docs.rs/crate/zbus-secret-service-keyring-store/1.0.1/source/src/service.rs
- https://docs.rs/crate/zbus-secret-service-keyring-store/1.0.1/source/src/cred.rs
- https://docs.rs/crate/secret-service/5.2.0/source/src/lib.rs
- https://docs.rs/crate/secret-service/5.2.0/source/src/error.rs
- https://docs.rs/crate/secret-service/5.2.0/source/src/util.rs
- https://docs.rs/crate/secret-service/5.2.0/source/src/collection.rs
- https://docs.rs/crate/secret-service/5.2.0/source/src/item.rs
- https://docs.rs/crate/secret-service/5.2.0/source/src/proxy/service.rs
- https://docs.rs/crate/secret-service/5.2.0/source/src/ss.rs
- https://docs.rs/crate/apple-native-keyring-store/1.0.2/source/Cargo.toml.orig
- https://docs.rs/crate/apple-native-keyring-store/1.0.2/source/src/keychain.rs
- https://docs.rs/crate/security-framework/3.7.0/source/src/os/macos/passwords.rs
- https://docs.rs/crate/security-framework/3.7.0/source/src/os/macos/keychain.rs
- https://docs.rs/crate/windows-native-keyring-store/1.1.0/source/src/store.rs
- https://docs.rs/crate/windows-native-keyring-store/1.1.0/source/src/cred.rs
- https://docs.rs/crate/windows-native-keyring-store/1.1.0/source/src/utils.rs

Secret Service specification (Draft 0.2, publication date 2026-10-19):

- https://specifications.freedesktop.org/secret-service/latest/
- https://specifications.freedesktop.org/secret-service/latest/description.html
- https://specifications.freedesktop.org/secret-service/0.2/prompts.html
- https://specifications.freedesktop.org/secret-service/0.2/errors.html
- https://specifications.freedesktop.org/secret-service/0.2/lookup-attributes.html
- https://specifications.freedesktop.org/secret-service-spec/latest/unlocking.html
- https://specifications.freedesktop.org/secret-service-spec/latest/object-paths.html
- https://specifications.freedesktop.org/secret-service/latest/org.freedesktop.Secret.Service.html
- https://specifications.freedesktop.org/secret-service-spec/0.2/org.freedesktop.Secret.Collection.html

Apple:

- https://developer.apple.com/documentation/security/seckeychainfindgenericpassword(_:_:_:_:_:_:_:_:)
- https://developer.apple.com/documentation/security/seckeychainaddgenericpassword(_:_:_:_:_:_:_:_:)
- https://developer.apple.com/documentation/security/seckeychainsetuserinteractionallowed(_:)?language=objc
- https://developer.apple.com/documentation/security/access-control-lists
- https://github.com/apple-oss-distributions/Security/blob/rel/Security-59754/base/SecBase.h

Microsoft:

- https://learn.microsoft.com/en-us/windows/win32/api/wincred/nf-wincred-credreadw
- https://learn.microsoft.com/en-us/windows/win32/api/wincred/nf-wincred-credwritew
- https://learn.microsoft.com/en-us/windows/win32/api/wincred/ns-wincred-credentialw
