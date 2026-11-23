use super::*;

// Keys made with `ssh-keygen`; the fingerprints are what `ssh-keygen -lf` printed.
const A: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAINxgFR+1+HJZImmHGS7C/3JFs1UUk7So7KwJfdgiFwK4";
const A_FP: &str = "SHA256:SNTYyYQ/dyYFDMjxVe4lzYHHBthrTPrxk7/btmdgrwk";
const B: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAILpyrGi4Hlcfe6TxXZCp7KltPy8jQmt6l40b5P6FcwW0";
const B_FP: &str = "SHA256:GNYCzvgTWmWZ+F5IPHky4My19zkrk8cFjh2/AN16Osw";
const C: &str = "ecdsa-sha2-nistp256 AAAAE2VjZHNhLXNoYTItbmlzdHAyNTYAAAAIbmlzdHAyNTYAAABBBD4D4EJYHK2mMN3M0Cmz6UO0tPEx3tcieZ9dRhm38Z05TYSliqcxueCnCRLlvxYQIcnCVMMwo6h1Q7NFXZrPQ1s=";

fn key(text: &str) -> PublicKey {
    PublicKey::from_openssh(text).unwrap()
}

#[test]
fn a_recorded_key_is_known() {
    let text = format!("example.org {A}\n");
    assert_eq!(check(&text, "example.org", 22, &key(A)), Verdict::Known);
}

#[test]
fn a_host_with_no_line_is_unknown() {
    let text = format!("other.org {A}\n");
    assert_eq!(check(&text, "example.org", 22, &key(A)), Verdict::Unknown);
    assert_eq!(check("", "example.org", 22, &key(A)), Verdict::Unknown);
}

#[test]
fn a_different_key_of_the_same_algorithm_is_changed_with_its_line_counting_comments() {
    let text = format!("# my hosts\n\nother.org {A}\nexample.org {B}\n");
    assert_eq!(
        check(&text, "example.org", 22, &key(A)),
        Verdict::Changed(Recorded {
            line_no: 4,
            line: format!("example.org {B}"),
            fingerprint: B_FP.to_string(),
        })
    );
}

#[test]
fn a_key_of_another_algorithm_leaves_the_host_unknown() {
    let text = format!("example.org {C}\n");
    assert_eq!(check(&text, "example.org", 22, &key(A)), Verdict::Unknown);
}

#[test]
fn a_port_other_than_22_is_recorded_in_brackets() {
    assert_eq!(host_name("Example.org", 22), "example.org");
    assert_eq!(host_name("example.org", 2222), "[example.org]:2222");
    let text = format!("[example.org]:2222 {A}\n");
    assert_eq!(check(&text, "example.org", 2222, &key(A)), Verdict::Known);
    assert_eq!(check(&text, "example.org", 22, &key(A)), Verdict::Unknown);
}

#[test]
fn hashed_names_tab_separators_and_lists_match() {
    // Hashed by `ssh-keygen -H` from "example.org".
    let hashed = format!("|1|jEkSGBvQMbbi4Tt57dIdC0s8+vk=|O1fu64MZ0QFNIiAJakFPeSMM6VU= {A}\n");
    assert_eq!(check(&hashed, "example.org", 22, &key(A)), Verdict::Known);
    assert_eq!(check(&hashed, "example.com", 22, &key(A)), Verdict::Unknown);
    let tabbed = A.replace(' ', "\t");
    let text = format!("one.org,example.org,10.0.0.1\t{tabbed}\n");
    assert_eq!(check(&text, "example.org", 22, &key(A)), Verdict::Known);
}

#[test]
fn wildcards_match_and_negation_excludes() {
    let text = format!("*.example.org,!secret.example.org {A}\n");
    assert_eq!(check(&text, "www.example.org", 22, &key(A)), Verdict::Known);
    assert_eq!(
        check(&text, "secret.example.org", 22, &key(A)),
        Verdict::Unknown
    );
    let text = format!("host?.org {A}\n");
    assert_eq!(check(&text, "host1.org", 22, &key(A)), Verdict::Known);
    assert_eq!(check(&text, "host12.org", 22, &key(A)), Verdict::Unknown);
}

#[test]
fn a_garbled_line_is_skipped() {
    let text = format!("example.org ssh-ed25519 notbase64!!\nexample.org {A}\n");
    assert_eq!(check(&text, "example.org", 22, &key(A)), Verdict::Known);
}

#[test]
fn a_revoked_key_is_refused() {
    let text = format!("@revoked example.org {A}\nexample.org {A}\n");
    assert!(matches!(
        check(&text, "example.org", 22, &key(A)),
        Verdict::Revoked(Recorded { line_no: 1, .. })
    ));
}

#[test]
fn recorded_algorithms_list_the_hosts_key_types_in_file_order() {
    let text = format!("example.org {C}\nexample.org {A}\nother.org {B}\n");
    assert_eq!(
        recorded_algorithms(&text, "example.org", 22),
        vec![key(C).algorithm(), key(A).algorithm()]
    );
}

#[test]
fn fingerprints_are_shown_as_openssh_shows_them() {
    assert_eq!(fingerprint(&key(A)), A_FP);
}

#[test]
fn append_adds_a_line_that_check_then_finds() {
    let dir = std::env::temp_dir().join(format!("sshexplorer-kh-{}", std::process::id()));
    let path = dir.join("ssh").join("known_hosts");
    let _ = std::fs::remove_dir_all(&dir);
    append(&path, "example.org", 2222, &key(A)).unwrap();
    // A file whose last line has no newline gets one before the new line.
    std::fs::write(&path, std::fs::read_to_string(&path).unwrap().trim_end()).unwrap();
    append(&path, "example.org", 22, &key(B)).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(text, format!("[example.org]:2222 {A}\nexample.org {B}\n"));
    assert_eq!(check(&text, "example.org", 2222, &key(A)), Verdict::Known);
}
