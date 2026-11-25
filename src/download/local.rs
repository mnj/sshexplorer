//! Local Download operations relative to an already-open destination directory.
use std::{fs::File, io, path::Path};

use cap_fs_ext::DirExt;
use cap_std::fs::{Dir, OpenOptions};

pub(crate) fn valid_name(name: &str) -> bool {
    #[cfg(windows)]
    {
        let stem = name
            .split('.')
            .next()
            .unwrap_or_default()
            .trim_end()
            .to_ascii_uppercase();
        let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || stem
                .strip_prefix("COM")
                .or_else(|| stem.strip_prefix("LPT"))
                .is_some_and(|suffix| {
                    matches!(
                        suffix,
                        "0" | "1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³"
                    )
                });
        if device
            || name.chars().any(|c| c < ' ' || "<>:\"/\\|?*".contains(c))
            || name.ends_with(['.', ' '])
        {
            return false;
        }
    }
    !name.contains('\0') && Path::new(name).file_name() == Some(name.as_ref())
}

/// A blocked directory has no handle, so none of its descendants can be copied.
pub(super) fn child(parent: &Dir, name: &str) -> io::Result<Option<Dir>> {
    #[cfg(windows)]
    let created = std::fs::create_dir(windows_path(parent)?.join(name));
    #[cfg(not(windows))]
    let created = parent.create_dir(name);
    match created {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    match parent.open_dir_nofollow(name) {
        Ok(dir) => Ok(Some(dir)),
        Err(_) if !parent.symlink_metadata(name)?.is_dir() => Ok(None),
        Err(e) => Err(e),
    }
}

pub(super) struct Partial<'a> {
    dir: &'a Dir,
    name: String,
    staging_name_moved: bool,
}

impl<'a> Partial<'a> {
    pub(super) fn create(dir: &'a Dir) -> io::Result<(Self, File)> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        loop {
            let name = format!(".sshexplorer-{:032x}.part", fastrand::u128(..));
            match dir.open_with(&name, &options) {
                Ok(file) => {
                    return Ok((
                        Self {
                            dir,
                            name,
                            staging_name_moved: false,
                        },
                        file.into_std(),
                    ));
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
    }

    pub(super) fn publish(&mut self, name: &str, overwrite: bool) -> io::Result<()> {
        #[cfg(windows)]
        {
            let path = windows_path(self.dir)?;
            let from = path.join(&self.name);
            let to = path.join(name);
            if overwrite {
                std::fs::rename(from, to)?;
            } else {
                renamore::rename_exclusive(from, to)?;
            }
            self.staging_name_moved = true;
        }
        #[cfg(not(windows))]
        {
            if overwrite {
                self.dir.rename(&self.name, self.dir, name)?;
                self.staging_name_moved = true;
            } else {
                self.staging_name_moved = move_exclusive(self.dir, &self.name, name)?;
            }
        }
        Ok(())
    }
}

impl Drop for Partial<'_> {
    fn drop(&mut self) {
        if !self.staging_name_moved {
            #[cfg(not(windows))]
            let _ = self.dir.remove_file(&self.name);
            #[cfg(windows)]
            let _ = windows_path(self.dir).and_then(|p| std::fs::remove_file(p.join(&self.name)));
        }
    }
}

/// Returns true when the staging name moved. A hard link leaves it for Drop.
#[cfg(any(target_os = "linux", target_os = "macos"))]
fn move_exclusive(dir: &Dir, from: &str, to: &str) -> io::Result<bool> {
    use rustix::fs::{RenameFlags, renameat_with};
    match renameat_with(dir, from, dir, to, RenameFlags::NOREPLACE) {
        Ok(()) => Ok(true),
        Err(rustix::io::Errno::NOSYS | rustix::io::Errno::INVAL | rustix::io::Errno::OPNOTSUPP) => {
            link_exclusive(dir, from, to)
        }
        Err(e) => Err(e.into()),
    }
}

/// The verbatim path preserves UNC shares. The directory handle denies delete
/// sharing, pinning the destination while Windows path-based operations run.
#[cfg(windows)]
fn windows_path(dir: &Dir) -> io::Result<std::path::PathBuf> {
    winx::file::get_file_path(&dir.try_clone()?.into_std_file())
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
use link_exclusive as move_exclusive;

#[cfg(not(windows))]
fn link_exclusive(dir: &Dir, from: &str, to: &str) -> io::Result<bool> {
    dir.hard_link(from, dir, to)?;
    Ok(false)
}
