//! Read-only host information sampled once per SSH connection.
use super::{Output, Remote, Request, Session};
use std::time::{Duration, Instant};

const LIMIT: usize = 8192;
const TIMEOUT: Duration = Duration::from_secs(5);

// Read os-release as data. The login shell only needs to launch POSIX sh.
const COMMAND: &str = r#"exec sh -c '
kernel=$(uname -sr)
printf "kernel=%s\n" "$kernel"
case "$kernel" in
    Linux*)
        release=/etc/os-release
        [ -r "$release" ] || release=/usr/lib/os-release
        sed -n "/^PRETTY_NAME=/p" "$release"
        if read -r seconds rest < /proc/uptime; then printf "seconds=%s\n" "$seconds"; fi
        ;;
    *)
        case "$kernel" in Darwin*) version=$(sw_vers -productVersion) && printf "PRETTY_NAME=macOS %s\n" "$version";; esac
        printf "boot=%s\nnow=%s\n" "$(sysctl -n kern.boottime)" "$(date +%s)"
        ;;
esac
exit 0'"#;

#[derive(Default)]
pub(crate) struct HostInfo {
    pub os: Option<String>,
    uptime: Option<(u64, Instant)>,
}

impl HostInfo {
    pub fn uptime(&self) -> Option<Duration> {
        let (seconds, sampled) = self.uptime?;
        Some(Duration::from_secs(seconds).saturating_add(sampled.elapsed()))
    }
}

impl Session {
    pub(crate) fn host_info(&self) -> Request<HostInfo> {
        let (tx, answer) = std::sync::mpsc::channel();
        let remote = self.remote();
        let wake = self.waker();
        let task = self.runtime.spawn(async move {
            let result = tokio::time::timeout(TIMEOUT, sample(&remote))
                .await
                .map_err(|_| "Host information timed out".to_owned());
            let _ = tx.send(result);
            wake();
        });
        Request {
            answer,
            task: Some(task.abort_handle()),
        }
    }
}

async fn sample(remote: &Remote) -> HostInfo {
    if let Some(info) = from_shell(remote).await {
        return info;
    }
    // SFTP-only Linux hosts sometimes expose these files inside their SFTP root.
    let (release, uptime) = tokio::join!(
        remote.read("/etc/os-release".into(), 0, LIMIT),
        remote.read("/proc/uptime".into(), 0, 128)
    );
    let release = String::from_utf8_lossy(release.as_deref().unwrap_or_default());
    let uptime = String::from_utf8_lossy(uptime.as_deref().unwrap_or_default());
    let seconds = uptime.split_whitespace().next().and_then(seconds_field);
    HostInfo {
        os: field(&release, "PRETTY_NAME=").and_then(os_name),
        uptime: seconds.map(|n| (n, Instant::now())),
    }
}

async fn from_shell(remote: &Remote) -> Option<HostInfo> {
    let mut exec = remote.exec(COMMAND).await.ok()?;
    let mut stdout = Vec::new();
    let mut success = false;
    while let Some(event) = exec.next().await {
        match event {
            Output::Stdout(chunk) if chunk.len() <= LIMIT - stdout.len() => stdout.extend(chunk),
            Output::Stdout(_) => return None,
            Output::Exit(status) => success = status == 0,
            Output::Stderr(_) => {}
        }
    }
    let info = parse(&String::from_utf8_lossy(&stdout));
    (success && (info.os.is_some() || info.uptime.is_some())).then_some(info)
}

fn field<'a>(output: &'a str, prefix: &str) -> Option<&'a str> {
    output.lines().find_map(|line| line.strip_prefix(prefix))
}

fn os_name(value: &str) -> Option<String> {
    let value = value.trim().trim_matches(['"', '\'']);
    (!value.is_empty() && !value.chars().any(char::is_control))
        .then(|| value.chars().take(128).collect())
}

fn seconds_field(value: &str) -> Option<u64> {
    let (whole, fraction) = value.split_once('.').unwrap_or((value, "0"));
    if fraction.is_empty() || !fraction.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    whole.parse().ok()
}

fn parse(output: &str) -> HostInfo {
    let os = field(output, "PRETTY_NAME=")
        .and_then(os_name)
        .or_else(|| field(output, "kernel=").and_then(os_name));
    let seconds = field(output, "seconds=")
        .and_then(seconds_field)
        .or_else(|| {
            let boot = field(output, "boot=")?;
            // OpenBSD reports epoch seconds. Darwin and FreeBSD include a timeval.
            let boot = boot
                .strip_prefix("{ sec = ")
                .and_then(|s| s.split_once(','))
                .map_or(boot, |(s, _)| s);
            field(output, "now=")?
                .parse::<u64>()
                .ok()?
                .checked_sub(boot.parse().ok()?)
        });
    HostInfo {
        os,
        uptime: seconds.map(|n| (n, Instant::now())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_os_and_uptime_are_independent() {
        for (reply, os, seconds) in [
            (
                "kernel=Linux 6.1\nPRETTY_NAME=\"Debian GNU/Linux 12\"\nseconds=7234.83",
                "Debian GNU/Linux 12",
                Some(7234),
            ),
            (
                "kernel=Darwin 24\nPRETTY_NAME=macOS 15.4\nboot={ sec = 100, usec = 50 } Tue\nnow=3700",
                "macOS 15.4",
                Some(3600),
            ),
            (
                "kernel=OpenBSD 7.8\nboot=100\nnow=3700",
                "OpenBSD 7.8",
                Some(3600),
            ),
            (
                "kernel=FreeBSD 14\nboot={ sec = 200, usec = 0 }\nnow=100",
                "FreeBSD 14",
                None,
            ),
        ] {
            let info = parse(reply);
            assert_eq!(info.os.as_deref(), Some(os));
            assert_eq!(info.uptime.map(|(n, _)| n), seconds);
        }
        assert_eq!(seconds_field("12.3x"), None);
    }
}
