//! Archive listings for Preview: the members of a zip, a tar or a gzipped tar, read from
//! bytes already fetched whole (an archive over the size cap is never fetched).
//!
//! The format comes from the bytes' signature first and the name second, so a zip named
//! `.tar` still lists. A `.gz` not named as a tar lists only when a tar is inside. Other
//! archive formats (7z, rar, xz, …) are not read. A listing is all or nothing: an archive
//! damaged anywhere gives [`Corrupt`], shown as [`CORRUPT`].

use std::io::{Cursor, Read};

use flate2::read::MultiGzDecoder;

/// The line a damaged archive shows.
pub const CORRUPT: &str = "Cannot preview: corrupt archive";

/// What an archive holds, in its own order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listing {
    pub members: Vec<Member>,
}

/// One member of an archive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    /// As the archive records it; a directory's usually ends in `/`.
    pub path: String,
    /// Uncompressed, in bytes.
    pub size: u64,
    pub is_dir: bool,
    pub modified: Option<Stamp>,
}

/// When a member was modified, as its format records it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stamp {
    /// Seconds since the Unix epoch (tar).
    Unix(u64),
    /// A wall-clock time with no time zone (zip).
    Civil {
        year: u16,
        month: u8,
        day: u8,
        hour: u8,
        minute: u8,
    },
}

/// The archive could not be read through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Corrupt;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Format {
    Zip,
    Tar,
    /// A gzipped tar.
    TarGz,
    /// Gzip, holding a tar or something else.
    Gz,
}

/// Whether a file named `name` may be an archive Preview lists, and so worth fetching.
pub fn may_list(name: &str) -> bool {
    by_name(name).is_some()
}

/// The listing of the archive `bytes`, named `name`; `None` when it is not a format
/// Preview lists.
pub fn list(name: &str, bytes: &[u8]) -> Option<Result<Listing, Corrupt>> {
    let format = match by_signature(bytes).or_else(|| by_name(name))? {
        Format::Gz if by_name(name) == Some(Format::TarGz) => Format::TarGz,
        Format::Gz => {
            let mut head = Vec::with_capacity(512);
            let read = MultiGzDecoder::new(bytes).take(512).read_to_end(&mut head);
            match read {
                Err(_) => return Some(Err(Corrupt)),
                Ok(_) if is_tar(&head) => Format::TarGz,
                Ok(_) => return None,
            }
        }
        format => format,
    };
    Some(match format {
        Format::Zip => zip(bytes),
        Format::Tar => tar(bytes),
        Format::TarGz | Format::Gz => tar(MultiGzDecoder::new(bytes)),
    })
}

fn by_signature(bytes: &[u8]) -> Option<Format> {
    let zip: [&[u8]; 3] = [b"PK\x03\x04", b"PK\x05\x06", b"PK\x07\x08"];
    if zip.iter().any(|sig| bytes.starts_with(sig)) {
        Some(Format::Zip)
    } else if bytes.starts_with(b"\x1f\x8b") {
        Some(Format::Gz)
    } else if is_tar(bytes) {
        Some(Format::Tar)
    } else {
        None
    }
}

fn is_tar(head: &[u8]) -> bool {
    head.get(257..262) == Some(b"ustar")
}

fn by_name(name: &str) -> Option<Format> {
    let name = name.to_ascii_lowercase();
    let ends = |suffixes: &[&str]| suffixes.iter().any(|s| name.ends_with(s));
    if ends(&[".zip", ".jar"]) {
        Some(Format::Zip)
    } else if ends(&[".tar"]) {
        Some(Format::Tar)
    } else if ends(&[".tgz", ".tar.gz"]) {
        Some(Format::TarGz)
    } else if ends(&[".gz"]) {
        Some(Format::Gz)
    } else {
        None
    }
}

fn zip(bytes: &[u8]) -> Result<Listing, Corrupt> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| Corrupt)?;
    let mut members = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        // Raw: the central directory's record, nothing decompressed.
        let file = archive.by_index_raw(i).map_err(|_| Corrupt)?;
        let path = match file.name() {
            Ok(name) => name.into_owned(),
            Err(_) => String::from_utf8_lossy(file.name_raw()).into_owned(),
        };
        members.push(Member {
            path,
            size: file.size(),
            is_dir: file.is_dir(),
            modified: file.last_modified().map(|t| Stamp::Civil {
                year: t.year(),
                month: t.month(),
                day: t.day(),
                hour: t.hour(),
                minute: t.minute(),
            }),
        });
    }
    Ok(Listing { members })
}

fn tar(reader: impl Read) -> Result<Listing, Corrupt> {
    let mut archive = tar::Archive::new(reader);
    let mut members = Vec::new();
    for entry in archive.entries().map_err(|_| Corrupt)? {
        let entry = entry.map_err(|_| Corrupt)?;
        let header = entry.header();
        members.push(Member {
            path: String::from_utf8_lossy(&entry.path_bytes()).into_owned(),
            size: header.size().map_err(|_| Corrupt)?,
            is_dir: header.entry_type().is_dir(),
            modified: header.mtime().ok().map(Stamp::Unix),
        });
    }
    Ok(Listing { members })
}
