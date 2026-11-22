//! The credential store seam and the rules for when a saved Host's kept secret goes.
//! Nothing here touches the OS credential store.

use sshexplorer::credentials::{self, Kind, Lookup, MemoryStore, Store};
use sshexplorer::hosts::Host;

fn remembering() -> Host {
    Host {
        id: "h1".into(),
        display_name: "Web".into(),
        address: "web.example.org".into(),
        account: "deploy".into(),
        key_file: "~/.ssh/id_web".into(),
        remember: true,
        ..Host::default()
    }
}

#[test]
fn saving_with_remember_off_forgets_both_kinds() {
    let old = remembering();
    let new = Host {
        remember: false,
        ..old.clone()
    };
    assert_eq!(
        credentials::forgotten(&old, Some(&new)),
        vec![Kind::Password, Kind::Passphrase]
    );
}

#[test]
fn deleting_the_host_forgets_both_kinds() {
    assert_eq!(
        credentials::forgotten(&remembering(), None),
        vec![Kind::Password, Kind::Passphrase]
    );
}

#[test]
fn changing_address_or_account_forgets_both_kinds() {
    let old = remembering();
    for new in [
        Host {
            address: "other".into(),
            ..old.clone()
        },
        Host {
            account: "root".into(),
            ..old.clone()
        },
    ] {
        assert_eq!(
            credentials::forgotten(&old, Some(&new)),
            vec![Kind::Password, Kind::Passphrase]
        );
    }
}

#[test]
fn changing_the_key_file_forgets_only_the_passphrase() {
    let old = remembering();
    let new = Host {
        key_file: "~/.ssh/id_other".into(),
        ..old.clone()
    };
    assert_eq!(
        credentials::forgotten(&old, Some(&new)),
        vec![Kind::Passphrase]
    );
}

#[test]
fn other_field_changes_keep_the_secret() {
    let old = remembering();
    let new = Host {
        display_name: "Renamed".into(),
        port: Some(2222),
        readonly: false,
        initial_folder: "/var/www".into(),
        ..old.clone()
    };
    assert_eq!(credentials::forgotten(&old, Some(&new)), vec![]);
}

#[test]
fn a_host_that_did_not_remember_has_nothing_to_forget() {
    let old = Host {
        remember: false,
        ..remembering()
    };
    assert_eq!(credentials::forgotten(&old, None), vec![]);
}

#[test]
fn the_memory_store_keeps_each_kind_apart_and_reports_its_failures() {
    let store = MemoryStore::default();
    let password = credentials::Key::new("h1", Kind::Password);
    let passphrase = credentials::Key::new("h1", Kind::Passphrase);
    assert!(matches!(store.get(&password), Lookup::NoEntry));
    store.set(&password, "open sesame").unwrap();
    assert!(matches!(store.get(&password), Lookup::Found(s) if *s == "open sesame"));
    assert!(matches!(store.get(&passphrase), Lookup::NoEntry));
    assert_eq!(store.secret(&password).as_deref(), Some("open sesame"));

    store.fail_get(true);
    assert!(matches!(store.get(&password), Lookup::Unavailable));
    store.fail_set(true);
    assert!(store.set(&passphrase, "x").is_err());
    store.fail_delete(true);
    assert!(store.delete(&password).is_err());
    assert_eq!(store.secret(&password).as_deref(), Some("open sesame"));
    store.fail_delete(false);
    store.delete(&password).unwrap();
    store.delete(&password).unwrap();
    assert_eq!(store.secret(&password), None);
}

#[test]
fn entries_are_named_by_the_host_id_and_the_kind() {
    assert_eq!(
        credentials::Key::new("h1", Kind::Password).username(),
        "h1:password"
    );
    assert_eq!(
        credentials::Key::new("h1", Kind::Passphrase).username(),
        "h1:passphrase"
    );
}

#[test]
fn every_new_host_id_is_different() {
    let a = sshexplorer::hosts::new_id();
    let b = sshexplorer::hosts::new_id();
    assert_ne!(a, b);
    assert!(a.len() >= 16);
}
