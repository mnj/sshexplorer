//! Download against the test server (tests/common/fixture.rs): what is copied where,
//! conflicts, failures, cancelling, symlinks, metadata and the per-Session queue.

mod common;

use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use common::fixture::{self, Fixture};
use sshexplorer::download::{
    self, Choice, Conflict, Download, End, Event, Job, Note, Options, Queue, Summary, numbered,
};
use sshexplorer::ssh::{Entry, Event as Opening, Ssh, Target};

const WAIT: Duration = Duration::from_secs(10);

/// A fixture with a Session open on it, and an empty Local folder.
struct Setup {
    fixture: Fixture,
    _ssh: Ssh,
    session: sshexplorer::ssh::Session,
    local: tempfile::TempDir,
    _known: tempfile::TempDir,
}

impl Setup {
    fn new() -> Setup {
        let fixture = Fixture::start();
        let ssh = Ssh::new(|| {});
        let known = tempfile::tempdir().unwrap();
        let path = known.path().join("known_hosts");
        std::fs::write(&path, format!("{}\n", fixture.known_hosts_line())).unwrap();
        let target = Target {
            address: fixture.address().to_string(),
            port: fixture.port(),
            account: fixture::ACCOUNT.to_string(),
        };
        let opening = ssh.open(target, path);
        let Some(Opening::Password(prompt)) = opening.recv_timeout(WAIT) else {
            panic!("expected the password prompt");
        };
        prompt.enter(fixture::PASSWORD.to_string());
        let Some(Opening::Opened(opened)) = opening.recv_timeout(WAIT) else {
            panic!("expected the Session to open");
        };
        Setup {
            fixture,
            _ssh: ssh,
            session: opened.session,
            local: tempfile::tempdir().unwrap(),
            _known: known,
        }
    }

    /// The remote path `path` on the fixture's disk.
    fn remote(&self, path: &str) -> PathBuf {
        self.fixture.root().join(path.trim_start_matches('/'))
    }

    fn write(&self, path: &str, text: &str) {
        let at = self.remote(path);
        std::fs::create_dir_all(at.parent().unwrap()).unwrap();
        std::fs::write(at, text).unwrap();
    }

    fn local(&self, rel: &str) -> PathBuf {
        self.local.path().join(rel)
    }

    /// The entries `names` of the remote directory `dir`, as its listing gives them.
    fn entries(&self, dir: &str, names: &[&str]) -> Vec<Entry> {
        let listing = self
            .session
            .list(dir.to_string())
            .recv_timeout(WAIT)
            .unwrap()
            .unwrap();
        names
            .iter()
            .map(|name| {
                listing
                    .entries
                    .iter()
                    .find(|e| e.name == *name)
                    .unwrap_or_else(|| panic!("{name} is listed"))
                    .clone()
            })
            .collect()
    }

    fn job(&self, dir: &str, names: &[&str], options: Options) -> Job {
        Job {
            dir: dir.to_string(),
            entries: self.entries(dir, names),
            local_folder: self.local.path().to_path_buf(),
            options,
        }
    }

    fn start(&self, dir: &str, names: &[&str]) -> Download {
        download::start(&self.session, self.job(dir, names, Options::default()))
    }
}

fn next(download: &Download) -> Event {
    download
        .recv_timeout(WAIT)
        .expect("an event from the Download")
}

fn conflict(download: &Download) -> Conflict {
    match next(download) {
        Event::Conflict(c) => c,
        Event::Finished(s) => panic!("expected a conflict, finished with {s:?}"),
    }
}

fn finished(download: &Download) -> Summary {
    match next(download) {
        Event::Finished(s) => s,
        Event::Conflict(c) => panic!("unexpected conflict on {}", c.name),
    }
}

/// Every file and directory under `dir`, relative and sorted, directories with a `/`.
fn tree(dir: &Path) -> Vec<String> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let rel = entry
                .path()
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            if entry.file_type().unwrap().is_dir() {
                out.push(format!("{rel}/"));
                walk(base, &entry.path(), out);
            } else {
                out.push(rel);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

fn read(path: PathBuf) -> String {
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn numbered_names_put_the_number_before_the_extension() {
    assert_eq!(numbered("notes.txt", 1), "notes (1).txt");
    assert_eq!(numbered("archive.tar.gz", 2), "archive (2).tar.gz");
    assert_eq!(numbered("Makefile", 1), "Makefile (1)");
    assert_eq!(numbered(".profile", 3), ".profile (3)");
    assert_eq!(numbered(".config.toml", 1), ".config (1).toml");
}

#[test]
fn a_file_is_copied_flat_and_a_directory_with_its_structure() {
    let s = Setup::new();
    s.write("/srv/a.txt", "alpha");
    s.write("/srv/site/index.html", "<html>");
    s.write("/srv/site/css/main.css", "body{}");
    std::fs::create_dir_all(s.remote("/srv/site/empty")).unwrap();

    let download = s.start("/srv", &["a.txt", "site"]);
    let summary = finished(&download);

    assert_eq!(summary.end, End::Finished);
    assert_eq!(summary.copied, 3);
    assert!(summary.is_clean());
    assert_eq!(
        tree(s.local.path()),
        [
            "a.txt",
            "site/",
            "site/css/",
            "site/css/main.css",
            "site/empty/",
            "site/index.html"
        ]
    );
    assert_eq!(read(s.local("site/css/main.css")), "body{}");
    let p = download.progress();
    assert_eq!(p.files_total, Some(3));
    assert_eq!(p.files_done, 3);
    assert_eq!(p.bytes_total, Some(5 + 6 + 6));
    assert_eq!(p.bytes_done, 17);
}

#[test]
fn an_existing_local_directory_is_merged_into() {
    let s = Setup::new();
    s.write("/srv/site/new.txt", "new");
    std::fs::create_dir_all(s.local("site")).unwrap();
    std::fs::write(s.local("site/mine.txt"), "mine").unwrap();

    let summary = finished(&s.start("/srv", &["site"]));

    assert!(summary.is_clean(), "{summary:?}");
    assert_eq!(
        tree(s.local.path()),
        ["site/", "site/mine.txt", "site/new.txt"]
    );
}

#[test]
fn a_conflict_waits_for_skip_overwrite_or_rename() {
    let s = Setup::new();
    for name in ["a.txt", "b.txt", "c.txt"] {
        s.write(&format!("/srv/{name}"), &format!("remote {name}"));
        std::fs::write(s.local(name), "local").unwrap();
    }
    std::fs::write(s.local("c (1).txt"), "taken").unwrap();

    let download = s.start("/srv", &["a.txt", "b.txt", "c.txt"]);
    let first = conflict(&download);
    assert_eq!(first.name, "a.txt");
    assert_eq!(first.path, s.local("a.txt"));
    assert_eq!(first.suggestion, "a (1).txt");
    // The Download waits for the answer.
    assert!(download.recv_timeout(Duration::from_millis(200)).is_none());
    first.answer(Choice::Skip, false);

    let second = conflict(&download);
    assert_eq!(second.name, "b.txt");
    second.answer(Choice::Overwrite, false);

    let third = conflict(&download);
    assert_eq!(third.suggestion, "c (2).txt", "the next free number");
    let name = third.suggestion.clone();
    third.answer(Choice::Rename(name), false);

    let summary = finished(&download);
    assert_eq!(summary.copied, 2);
    assert_eq!(
        summary.skipped,
        [Note {
            path: "a.txt".into(),
            reason: "Already exists".into()
        }]
    );
    assert!(!summary.is_clean());
    assert_eq!(read(s.local("a.txt")), "local");
    assert_eq!(read(s.local("b.txt")), "remote b.txt");
    assert_eq!(read(s.local("c.txt")), "local");
    assert_eq!(read(s.local("c (1).txt")), "taken");
    assert_eq!(read(s.local("c (2).txt")), "remote c.txt");
}

#[test]
fn apply_to_all_answers_the_remaining_conflicts_but_not_after_a_hand_edit() {
    let s = Setup::new();
    for name in ["a.txt", "b.txt", "c.txt", "d.txt"] {
        s.write(&format!("/srv/{name}"), &format!("remote {name}"));
        std::fs::write(s.local(name), "local").unwrap();
    }

    // A hand-edited name applies to that file only, even with Apply to all.
    let download = s.start("/srv", &["a.txt", "b.txt", "c.txt", "d.txt"]);
    conflict(&download).answer(Choice::Rename("mine.txt".into()), true);
    // An automatic Rename applies to all the rest, each with its own number.
    let b = conflict(&download);
    let name = b.suggestion.clone();
    b.answer(Choice::Rename(name), true);
    let summary = finished(&download);

    assert!(summary.skipped.is_empty());
    assert_eq!(summary.copied, 4);
    assert_eq!(read(s.local("mine.txt")), "remote a.txt");
    assert_eq!(read(s.local("b (1).txt")), "remote b.txt");
    assert_eq!(read(s.local("c (1).txt")), "remote c.txt");
    assert_eq!(read(s.local("d (1).txt")), "remote d.txt");
    assert_eq!(read(s.local("d.txt")), "local");
}

#[test]
fn cancel_at_a_conflict_keeps_finished_files_and_leaves_no_temporary_file() {
    let s = Setup::new();
    s.write("/srv/d/1.txt", "one");
    s.write("/srv/d/2.txt", "two");
    s.write("/srv/d/3.txt", "three");
    std::fs::create_dir_all(s.local("d")).unwrap();
    std::fs::write(s.local("d/2.txt"), "local").unwrap();

    let download = s.start("/srv", &["d"]);
    conflict(&download).answer(Choice::Cancel, false);
    let summary = finished(&download);

    assert_eq!(summary.end, End::Cancelled);
    assert_eq!(summary.copied, 1);
    assert_eq!(tree(s.local.path()), ["d/", "d/1.txt", "d/2.txt"]);
    assert_eq!(read(s.local("d/2.txt")), "local");
}

#[test]
fn cancel_during_a_file_removes_the_file_in_flight() {
    let s = Setup::new();
    std::fs::create_dir_all(s.remote("/srv")).unwrap();
    std::fs::write(s.remote("/srv/big.bin"), vec![7u8; 4 << 20]).unwrap();
    s.fixture.slow_reads(Duration::from_millis(50));

    let download = s.start("/srv", &["big.bin"]);
    let deadline = std::time::Instant::now() + WAIT;
    while download.progress().bytes_done == 0 {
        assert!(std::time::Instant::now() < deadline, "the copy starts");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(download.progress().current.as_deref(), Some("big.bin"));
    download.cancel();
    let summary = finished(&download);

    assert_eq!(summary.end, End::Cancelled);
    assert_eq!(summary.copied, 0);
    assert!(
        tree(s.local.path()).is_empty(),
        "{:?}",
        tree(s.local.path())
    );
}

#[test]
fn a_remote_failure_is_recorded_and_the_download_goes_on() {
    let s = Setup::new();
    s.write("/srv/d/locked.txt", "secret");
    s.write("/srv/d/open.txt", "open");
    s.write("/srv/gone.txt", "soon gone");
    let locked = s.remote("/srv/d/locked.txt");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    // gone.txt is in the Selection but vanishes before the Download reaches it.
    let job = s.job("/srv", &["d", "gone.txt"], Options::default());
    std::fs::remove_file(s.remote("/srv/gone.txt")).unwrap();

    let summary = finished(&download::start(&s.session, job));

    assert_eq!(summary.end, End::Finished);
    assert_eq!(summary.copied, 1);
    assert_eq!(
        summary.failed,
        [
            Note {
                path: "d/locked.txt".into(),
                reason: "Permission denied".into()
            },
            Note {
                path: "gone.txt".into(),
                reason: "No such file".into()
            },
        ]
    );
    assert_eq!(tree(s.local.path()), ["d/", "d/open.txt"]);
}

#[test]
fn a_local_write_error_stops_the_download() {
    let s = Setup::new();
    s.write("/srv/a.txt", "a");
    s.write("/srv/b.txt", "b");
    let readonly = s.local("readonly");
    std::fs::create_dir(&readonly).unwrap();
    std::fs::set_permissions(&readonly, std::fs::Permissions::from_mode(0o555)).unwrap();
    let mut job = s.job("/srv", &["a.txt", "b.txt"], Options::default());
    job.local_folder = readonly.clone();

    let summary = finished(&download::start(&s.session, job));

    assert!(
        matches!(&summary.end, End::Stopped(why) if why.contains("a.txt")),
        "{summary:?}"
    );
    assert_eq!(summary.copied, 0);
    assert!(summary.failed.is_empty(), "b.txt is never tried");
    assert!(tree(&readonly).is_empty());
}

#[test]
fn symlinked_directories_are_followed_only_when_asked_and_no_local_links_are_made() {
    let s = Setup::new();
    s.write("/srv/real/inside.txt", "inside");
    s.write("/srv/file.txt", "file");
    std::os::unix::fs::symlink("real", s.remote("/srv/linked")).unwrap();
    std::os::unix::fs::symlink("file.txt", s.remote("/srv/alias.txt")).unwrap();
    // A link back up would loop; each directory is copied once.
    std::os::unix::fs::symlink("..", s.remote("/srv/real/up")).unwrap();

    let not_followed = finished(&s.start("/srv", &["linked", "alias.txt"]));
    assert_eq!(
        not_followed.skipped,
        [Note {
            path: "linked".into(),
            reason: "symlink, not followed".into()
        }]
    );
    assert_eq!(tree(s.local.path()), ["alias.txt"]);
    assert!(!s.local("alias.txt").is_symlink());
    assert_eq!(read(s.local("alias.txt")), "file");

    std::fs::remove_file(s.local("alias.txt")).unwrap();
    let options = Options {
        follow_links: true,
        ..Options::default()
    };
    let job = s.job("/srv", &["linked"], options);
    let followed = finished(&download::start(&s.session, job));
    assert_eq!(followed.end, End::Finished);
    let local = tree(s.local.path());
    assert!(
        local.contains(&"linked/inside.txt".to_string()),
        "{local:?}"
    );
    assert!(
        local.iter().filter(|p| p.ends_with("inside.txt")).count() == 1,
        "the loop is not followed round: {local:?}"
    );
    assert!(!s.local("linked").is_symlink());
}

#[test]
fn modification_time_and_executable_bit_are_kept_when_on() {
    let s = Setup::new();
    s.write("/srv/run.sh", "#!/bin/sh\n");
    s.write("/srv/plain.txt", "plain");
    let run = s.remote("/srv/run.sh");
    std::fs::set_permissions(&run, std::fs::Permissions::from_mode(0o755)).unwrap();
    let old = std::time::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    for name in ["run.sh", "plain.txt"] {
        let file = std::fs::File::options()
            .write(true)
            .open(s.remote(&format!("/srv/{name}")))
            .unwrap();
        file.set_modified(old).unwrap();
    }

    finished(&s.start("/srv", &["run.sh", "plain.txt"]));
    let mode = |name: &str| {
        std::fs::metadata(s.local(name))
            .unwrap()
            .permissions()
            .mode()
    };
    let modified = |name: &str| {
        std::fs::metadata(s.local(name))
            .unwrap()
            .modified()
            .unwrap()
    };
    assert_ne!(mode("run.sh") & 0o100, 0, "executable");
    assert_eq!(mode("plain.txt") & 0o111, 0);
    assert_eq!(modified("run.sh"), old);

    let off = tempfile::tempdir().unwrap();
    let mut job = s.job(
        "/srv",
        &["run.sh"],
        Options {
            keep_modified: false,
            keep_executable: false,
            follow_links: false,
        },
    );
    job.local_folder = off.path().to_path_buf();
    finished(&download::start(&s.session, job));
    let meta = std::fs::metadata(off.path().join("run.sh")).unwrap();
    assert_eq!(meta.permissions().mode() & 0o111, 0);
    assert_ne!(meta.modified().unwrap(), old);
}

#[test]
fn a_session_queue_runs_its_downloads_in_order() {
    let s = Setup::new();
    std::fs::create_dir_all(s.remote("/srv")).unwrap();
    std::fs::write(s.remote("/srv/one.bin"), vec![1u8; 1 << 20]).unwrap();
    s.write("/srv/two.txt", "two");
    let one = s.job("/srv", &["one.bin"], Options::default());
    let two = s.job("/srv", &["two.txt"], Options::default());
    s.fixture.slow_reads(Duration::from_millis(50));

    let mut queue = Queue::default();
    queue.push(&s.session, one);
    queue.push(&s.session, two);
    assert!(queue.is_busy());
    assert_eq!(queue.queued(), 1);
    assert!(queue.running().is_some());

    let deadline = std::time::Instant::now() + WAIT;
    let mut ended = 0;
    while ended < 2 {
        assert!(std::time::Instant::now() < deadline, "both finish");
        if queue.poll(&s.session) {
            ended += 1;
            if ended == 1 {
                assert!(s.local("one.bin").exists());
                assert!(!s.local("two.txt").exists(), "the second waits its turn");
                assert_eq!(queue.queued(), 0);
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!queue.is_busy());
    assert_eq!(read(s.local("two.txt")), "two");
    assert!(queue.finished.as_ref().is_some_and(Summary::is_clean));
}

#[test]
fn a_late_conflict_keeps_both_sessions_contents_on_rename() {
    let slow = Setup::new();
    let fast = Setup::new();
    let content = "x".repeat(1 << 20);
    slow.write("/srv/report", &content);
    slow.fixture.slow_reads(Duration::from_millis(20));
    fast.write("/srv/report", "short report");
    let mut fast_job = fast.job("/srv", &["report"], Options::default());
    fast_job.local_folder = slow.local.path().to_owned();

    let first = slow.start("/srv", &["report"]);
    let deadline = std::time::Instant::now() + WAIT;
    while first.progress().bytes_done == 0 {
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(finished(&download::start(&fast.session, fast_job)).is_clean());
    let prompt = conflict(&first);
    assert_eq!(prompt.name, "report");
    assert_eq!(read(slow.local("report")), "short report");
    prompt.answer(Choice::Rename("slow report".into()), false);
    assert!(finished(&first).is_clean());
    assert_eq!(read(slow.local("slow report")), content);
    assert_eq!(tree(slow.local.path()), ["report", "slow report"]);
}

#[test]
fn a_blocked_directory_skips_its_descendants_and_copies_the_next_selection() {
    let s = Setup::new();
    s.write("/srv/blocked/nested/data", "data");
    s.write("/srv/ok", "ok");
    let other = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(other.path(), s.local("blocked")).unwrap();
    let summary = finished(&s.start("/srv", &["blocked", "ok"]));
    assert_eq!(summary.end, End::Finished);
    assert_eq!(summary.copied, 1);
    assert_eq!(summary.failed.len(), 1);
    assert_eq!(summary.failed[0].path, "blocked");
    assert_eq!(read(s.local("ok")), "ok");
    assert!(tree(other.path()).is_empty());
}

#[test]
fn a_file_selected_in_search_keeps_its_size_time_and_mode() {
    use sshexplorer::folder::Command;
    use sshexplorer::matches::Matches;
    use sshexplorer::search::{Match, Query};

    let s = Setup::new();
    s.write("/srv/run.sh", "hello");
    let file = std::fs::File::open(s.remote("/srv/run.sh")).unwrap();
    let old = std::time::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
    file.set_modified(old).unwrap();
    file.set_permissions(std::fs::Permissions::from_mode(0o755))
        .unwrap();
    let mut matches = Matches::new(Query {
        start: "/srv".into(),
        text: "run".into(),
        follow: false,
        cap: 10,
    });
    matches.found(vec![Match {
        path: "run.sh".into(),
        is_dir: false,
        link: false,
    }]);
    matches.apply(Command::Click(0));
    let (dir, entries) = matches.selection().unwrap();
    let job = Job {
        dir,
        entries,
        local_folder: s.local.path().to_owned(),
        options: Options::default(),
    };
    let download = download::start(&s.session, job);
    assert!(finished(&download).is_clean());
    assert_eq!(download.progress().bytes_total, Some(5));
    let meta = std::fs::metadata(s.local("run.sh")).unwrap();
    assert_eq!(meta.modified().unwrap(), old);
    assert_ne!(meta.permissions().mode() & 0o100, 0);
}

#[test]
fn rename_suggestion_fits_even_when_the_existing_name_is_full_length() {
    let s = Setup::new();
    let name = format!("{}.tar.gz", "n".repeat(248));
    s.write(&format!("/srv/{name}"), "new");
    std::fs::write(s.local(&name), "original").unwrap();
    let transfer = s.start("/srv", &[&name]);
    let prompt = conflict(&transfer);
    let suggested = prompt.suggestion.clone();
    assert!(suggested.ends_with(" (1).tar.gz"));
    prompt.answer(Choice::Rename(suggested.clone()), false);
    assert!(finished(&transfer).is_clean());
    assert_eq!(read(s.local(&name)), "original");
    assert_eq!(read(s.local(&suggested)), "new");
}
