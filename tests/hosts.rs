//! Hosts: the pasted-address rules of the Host dialog's Address field, the Port field's
//! values, tab labels, and saving Hosts in their own file.

use sshexplorer::hosts::{self, Host};

/// What leaving Address with `address` in it, Port holding `port` and Account holding
/// `account`, makes of the three fields, and the messages under Address.
fn split(address: &str, port: &str, account: &str) -> (String, String, String, Vec<String>) {
    let s = hosts::split_pasted(address, port, account);
    let errors = s.errors.iter().map(|e| e.to_string()).collect();
    (s.address, s.port, s.account, errors)
}

fn fields(address: &str, port: &str, account: &str) -> (String, String, String, Vec<String>) {
    (address.into(), port.into(), account.into(), Vec::new())
}

#[test]
fn a_trailing_port_and_a_leading_account_move_into_empty_fields() {
    assert_eq!(
        split("deploy@example.org:2222", "", ""),
        fields("example.org", "2222", "deploy")
    );
    assert_eq!(
        split("example.org:22", "", ""),
        fields("example.org", "22", "")
    );
    assert_eq!(
        split("me@example.org", "", ""),
        fields("example.org", "", "me")
    );
    assert_eq!(
        split(" example.org ", "", ""),
        fields("example.org", "", "")
    );
}

#[test]
fn text_stays_in_address_when_its_field_is_already_set() {
    assert_eq!(
        split("example.org:2222", "2200", ""),
        (
            "example.org:2222".into(),
            "2200".into(),
            "".into(),
            vec!["Port is already set".to_string()]
        )
    );
    assert_eq!(
        split("me@example.org:2222", "", "deploy"),
        (
            "me@example.org".into(),
            "2222".into(),
            "deploy".into(),
            vec!["Account is already set".to_string()]
        )
    );
}

#[test]
fn a_bracketed_ipv6_address_is_split_and_a_bare_one_left_alone() {
    assert_eq!(
        split("[fe80::1]:2222", "", ""),
        fields("fe80::1", "2222", "")
    );
    assert_eq!(
        split("root@[2001:db8::7]:22", "", ""),
        fields("2001:db8::7", "22", "root")
    );
    assert_eq!(split("2001:db8::7", "", ""), fields("2001:db8::7", "", ""));
    assert_eq!(split("::1", "", ""), fields("::1", "", ""));
}

#[test]
fn a_colon_without_digits_is_not_a_port() {
    assert_eq!(
        split("example.org:", "", ""),
        fields("example.org:", "", "")
    );
    assert_eq!(
        split("example.org:ab", "", ""),
        fields("example.org:ab", "", "")
    );
}

#[test]
fn port_accepts_1_to_65535_or_empty() {
    assert_eq!(hosts::port_value(""), Ok(None));
    assert_eq!(hosts::port_value(" 2222 "), Ok(Some(2222)));
    assert_eq!(hosts::port_value("65535"), Ok(Some(65535)));
    for bad in ["0", "65536", "-1", "22a", "2 2"] {
        assert!(hosts::port_value(bad).is_err(), "{bad} is refused");
    }
}

#[test]
fn a_new_host_is_readonly_does_not_remember_and_downloads_into_downloads() {
    let host = Host::default();
    assert!(host.readonly);
    assert!(!host.remember);
    assert_eq!(host.auth, hosts::AuthMethod::Automatic);
    assert_eq!(host.initial_folder, "");
    assert_eq!(host.local_folder, hosts::downloads_dir());
}

fn store() -> (tempfile::TempDir, hosts::HostStore) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let store = hosts::HostStore::at(dir.path().join("config").join("hosts.toml"));
    (dir, store)
}

#[test]
fn no_file_loads_no_hosts() {
    let (_dir, store) = store();
    assert_eq!(store.load(), Vec::<Host>::new());
}

#[test]
fn saved_hosts_load_again_in_order() {
    let (_dir, store) = store();
    let web = Host {
        display_name: "Web".into(),
        address: "web.example.org".into(),
        port: Some(2222),
        account: "deploy".into(),
        auth: hosts::AuthMethod::KeyFile,
        key_file: "~/.ssh/id_web".into(),
        local_folder: "/tmp/web".into(),
        initial_folder: "/var/www".into(),
        readonly: false,
        remember: true,
    };
    let db = Host {
        display_name: "DB".into(),
        address: "db".into(),
        ..Host::default()
    };
    store.save(&[web.clone(), db.clone()]).unwrap();
    assert_eq!(store.load(), vec![web, db]);
}

#[test]
fn a_bad_field_takes_its_default_and_keeps_the_rest() {
    let (_dir, store) = store();
    std::fs::create_dir_all(store.path().parent().unwrap()).unwrap();
    std::fs::write(
        store.path(),
        "[[host]]\ndisplay_name = \"Web\"\naddress = \"web\"\nport = 70000\n\
         auth = \"telepathy\"\nreadonly = \"yes\"\n",
    )
    .unwrap();
    let hosts = store.load();
    assert_eq!(hosts.len(), 1);
    let host = &hosts[0];
    assert_eq!(host.display_name, "Web");
    assert_eq!(host.address, "web");
    assert_eq!(host.port, None);
    assert_eq!(host.auth, hosts::AuthMethod::Automatic);
    assert!(host.readonly);
    assert_eq!(host.local_folder, hosts::downloads_dir());
}

#[test]
fn a_file_that_is_not_toml_is_kept_aside_on_the_next_save() {
    let (_dir, store) = store();
    std::fs::create_dir_all(store.path().parent().unwrap()).unwrap();
    std::fs::write(store.path(), "not = [toml").unwrap();
    assert!(store.load().is_empty());
    store.save(&[Host::default()]).unwrap();
    let aside = store.path().with_file_name("hosts.toml.bad");
    assert_eq!(std::fs::read_to_string(aside).unwrap(), "not = [toml");
    assert_eq!(store.load().len(), 1);
}
