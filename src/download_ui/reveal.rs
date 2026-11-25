//! Open the local file manager from a worker so desktop services cannot stall a frame.
use std::{
    io,
    path::{Path, PathBuf},
    process::Command,
    sync::mpsc,
};

pub(super) fn start(ctx: egui::Context, path: PathBuf) -> mpsc::Receiver<Result<(), String>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let result = std::path::absolute(&path).and_then(|path| show(&path));
        let _ = tx.send(result.map_err(|e| format!("Cannot show {}: {e}", path.display())));
        ctx.request_repaint();
    });
    rx
}

#[cfg(target_os = "macos")]
fn show(path: &Path) -> io::Result<()> {
    run(Command::new("open").arg("-R").arg(path))
}

#[cfg(windows)]
fn show(path: &Path) -> io::Result<()> {
    let mut selection = std::ffi::OsString::from("/select,");
    selection.push(path);
    let mut child = Command::new("explorer.exe").arg(selection).spawn()?;
    child.wait().map(|_| ())
}

#[cfg(not(any(windows, target_os = "macos")))]
fn show(path: &Path) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        let uri =
            url::Url::from_file_path(path).map_err(|_| io::Error::other("Invalid local path"))?;
        let selected = zbus::blocking::Connection::session().and_then(|bus| {
            bus.call_method(
                Some("org.freedesktop.FileManager1"),
                "/org/freedesktop/FileManager1",
                Some("org.freedesktop.FileManager1"),
                "ShowItems",
                &(vec![uri.as_str()], ""),
            )
        });
        if selected.is_ok() {
            return Ok(());
        }
    }
    let dir = if path.is_dir() {
        path
    } else {
        path.parent()
            .ok_or_else(|| io::Error::other("Missing parent folder"))?
    };
    run(Command::new("xdg-open").arg(dir))
}

#[cfg(not(windows))]
fn run(cmd: &mut Command) -> io::Result<()> {
    let output = cmd.output()?;
    if output.status.success() {
        return Ok(());
    }
    Err(io::Error::other(format!(
        "{} {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    )))
}
