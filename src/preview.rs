//! Preview's model, apart from egui and the network: which kind a file is, when its fetch
//! is due, and what the panel shows once the bytes arrive.
//!
//! The panel always shows the Focused entry's details header. Below it is the [`Body`]:
//! nothing for a directory, "Loading preview…" until the bytes arrive, the text, one
//! failure line, or [`UNSUPPORTED`]. The fetch is due once focus has rested [`DELAY`]
//! seconds on a file; focusing another entry first means it never starts, and the caller
//! drops a read in flight when [`Preview::focus`] says the preview changed. Only the
//! current preview is held.
//!
//! Kind detection is local: the extension first, then the first bytes' signature, then
//! "no NUL and valid UTF-8 in the first [`SNIFF`] bytes" for text. Nothing runs on the
//! host. Text, source, Markdown and archives are the kinds with a body so far; other
//! kinds plug in as further [`Body`] variants and get the same [`Fetched`] bytes with the
//! size cap's result. Markdown is rendered, or its source on the Rendered / Source
//! switch. Zip and tar archives show
//! their [`archive`] listing; they are read whole, so one over the cap is not fetched
//! and shows [`too_large`].

use crate::archive::{self, Listing};
use crate::folder::size_text;
use crate::kind::Kind;
use crate::scratch::{Kept, Place};
use std::sync::Arc;

use crate::source::{self, Highlighting};
use crate::ssh::Entry;

/// How long focus rests on a file before its fetch starts, in seconds.
pub const DELAY: f64 = 0.2;
/// How many of the first bytes decide whether an unknown file is text.
pub const SNIFF: usize = 8 * 1024;
/// The line for a file Preview cannot show.
pub const UNSUPPORTED: &str = "No preview for this file type.";

/// The Focused entry, as Preview needs it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Focused {
    /// The entry's absolute path.
    pub path: String,
    /// As the listing shows it: the entry's name, or `.` or `..`.
    pub name: String,
    /// What the host said about it; `None` for `.` and `..`, which are directories.
    pub entry: Option<Entry>,
}

impl Focused {
    pub fn is_dir(&self) -> bool {
        self.entry.as_ref().is_none_or(|e| e.is_dir)
    }

    /// Whether `other` is this entry as it was: same path, size and modified time.
    fn same_as(&self, other: &Focused) -> bool {
        let stamp = |f: &Focused| f.entry.as_ref().map(|e| (e.size, e.modified, e.is_dir));
        self.path == other.path && stamp(self) == stamp(other)
    }
}

/// What Preview holds: the preview of the Focused entry, if there is one.
#[derive(Default)]
pub struct Preview {
    shown: Option<Shown>,
    next_id: u64,
}

/// The preview of one entry.
pub struct Shown {
    pub focused: Focused,
    /// By extension until the bytes arrive; then, for a file without a known
    /// extension, by its first bytes.
    pub kind: Kind,
    pub body: Body,
    /// When focus came to rest here.
    since: f64,
    /// The fetch asked for, whose answer is awaited.
    fetch: Option<u64>,
    /// Whether the fetch has been handed out.
    started: bool,
}

/// What shows under the details header.
pub enum Body {
    /// Nothing: a directory, `.` or `..`.
    Header,
    /// "Loading preview…" and a spinner: waiting for focus to rest, or for the bytes.
    Loading,
    Text(Text),
    Markdown(Markdown),
    /// [`UNSUPPORTED`].
    Unsupported,
    /// One line, such as "Cannot read: permission denied".
    Failed(String),
    /// An archive's members.
    Archive(Listing),
    /// "Too large to preview (2.3 GiB)": a file that is only shown whole, over the cap.
    TooLarge(String),
    /// An image: the panel decodes and shows it.
    Image(Image),
    /// A PDF: the panel renders its pages.
    Pdf(Pdf),
    /// Audio or video: the panel streams it in ranged reads, whatever its size.
    Media(Stream),
}

/// Audio or video to stream: never fetched whole, so exempt from the size cap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stream {
    pub path: String,
    /// The file's size, when the host said.
    pub size: Option<u64>,
}

/// Whether `kind` streams instead of being fetched.
pub fn streams(kind: Kind) -> bool {
    matches!(kind, Kind::Audio | Kind::Video)
}

/// An image's bytes, fetched whole.
pub struct Image {
    pub fetched: Fetched,
}

/// A PDF's bytes, fetched whole.
pub struct Pdf {
    pub fetched: Fetched,
}

/// Whether `kind` is shown only from the whole file, so that over the size cap it is
/// [`Body::TooLarge`] and not fetched.
pub fn whole(kind: Kind) -> bool {
    matches!(kind, Kind::Image | Kind::Pdf | Kind::Archive)
}

impl Body {
    /// The text a text, source or Markdown body holds, which the panel highlights.
    pub fn text_mut(&mut self) -> Option<&mut Text> {
        match self {
            Body::Text(text) => Some(text),
            Body::Markdown(markdown) => Some(&mut markdown.text),
            _ => None,
        }
    }
}

/// A Markdown file: rendered by default, or its highlighted source.
pub struct Markdown {
    /// The source, as Source shows it.
    pub text: Text,
    /// What Rendered shows: the source with each image replaced by its alt text.
    pub rendered: String,
    pub view: MarkdownView,
}

/// The Rendered / Source switch.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MarkdownView {
    #[default]
    Rendered,
    Source,
}

/// A fetch that is due: read up to `len` bytes of `path`, and hand the answer to
/// [`Preview::arrive`] with `id`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fetch {
    pub id: u64,
    pub path: String,
    pub len: u64,
}

/// The bytes fetched for a preview, kept in the Scratch folder, and the cap's result.
pub struct Fetched {
    pub kept: Kept,
    /// The file's size, when the host said.
    pub total: Option<u64>,
    /// Whether the file is larger than what was fetched.
    pub capped: bool,
}

impl Fetched {
    /// "Showing the first 10 MiB of 2.3 GiB", when the file is larger than the cap.
    pub fn banner(&self) -> Option<String> {
        if !self.capped {
            return None;
        }
        let shown = self.kept.len() as u64;
        Some(match self.total {
            Some(total) => capped_banner(shown, total),
            None => format!("Showing the first {}", short_size(shown)),
        })
    }
}

/// A text or source file's contents, split into lines.
pub struct Text {
    pub fetched: Fetched,
    pub text: Arc<str>,
    /// Each line's byte range in `text`, without its line ending.
    pub lines: Arc<[(usize, usize)]>,
    /// The syntect syntax picked by name or shebang; `None` shows plain text.
    pub syntax: Option<&'static str>,
    /// The highlighting run for the Theme in force, which the panel starts.
    pub highlighting: Option<Highlighting>,
}

impl Text {
    fn new(name: &str, fetched: Fetched, bytes: &[u8]) -> Text {
        let text = decode(bytes, fetched.capped);
        let mut lines = Vec::new();
        let mut start = 0;
        for (i, b) in text.bytes().enumerate() {
            if b == b'\n' {
                let end = if i > start && text.as_bytes()[i - 1] == b'\r' {
                    i - 1
                } else {
                    i
                };
                lines.push((start, end));
                start = i + 1;
            }
        }
        if start < text.len() || lines.is_empty() {
            lines.push((start, text.len()));
        }
        let first = text.get(..lines[0].1).unwrap_or_default();
        let syntax = source::syntax_for(name, first);
        Text {
            fetched,
            text: text.into(),
            lines: lines.into(),
            syntax,
            highlighting: None,
        }
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub fn line(&self, i: usize) -> &str {
        let (start, end) = self.lines[i];
        &self.text[start..end]
    }
}

/// `bytes` as text. A capped fetch may end inside a character, which is dropped; any
/// other invalid sequence shows as U+FFFD.
fn decode(bytes: &[u8], capped: bool) -> String {
    let mut end = bytes.len();
    if capped {
        // The last lead byte, if the bytes after it are fewer than it announces.
        for i in (bytes.len().saturating_sub(3)..bytes.len()).rev() {
            let b = bytes[i];
            if b < 0x80 {
                break;
            }
            if b >= 0xc0 {
                let width = if b >= 0xf0 {
                    4
                } else if b >= 0xe0 {
                    3
                } else {
                    2
                };
                if i + width > bytes.len() {
                    end = i;
                }
                break;
            }
        }
    }
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

impl Preview {
    /// Shows `focused`, or nothing. Returns whether the preview changed, so that a read
    /// in flight for the one before must be dropped. The same file with the same size
    /// and modified time keeps its preview; one whose size or time changed reloads.
    pub fn focus(&mut self, focused: Option<Focused>, now: f64) -> bool {
        let Some(focused) = focused else {
            return self.shown.take().is_some();
        };
        if self
            .shown
            .as_ref()
            .is_some_and(|s| s.focused.same_as(&focused))
        {
            if let Some(shown) = &mut self.shown {
                shown.focused = focused;
            }
            return false;
        }
        let kind = match &focused.entry {
            None => Kind::Directory,
            Some(e) => Kind::of(&e.name, e.is_dir),
        };
        let body = match kind {
            Kind::Directory => Body::Header,
            Kind::Text | Kind::Source | Kind::Markdown | Kind::Other => Body::Loading,
            Kind::Image | Kind::Pdf => Body::Loading,
            Kind::Audio | Kind::Video => Body::Loading,
            Kind::Archive if archive::may_list(&focused.name) => Body::Loading,
            _ => Body::Unsupported,
        };
        self.shown = Some(Shown {
            focused,
            kind,
            body,
            since: now,
            fetch: None,
            started: false,
        });
        true
    }

    pub fn shown(&self) -> Option<&Shown> {
        self.shown.as_ref()
    }

    pub fn shown_mut(&mut self) -> Option<&mut Shown> {
        self.shown.as_mut()
    }

    /// The fetch to start, once focus has rested long enough: at most `cap` bytes. It is
    /// handed out once.
    pub fn due(&mut self, now: f64, cap: u64) -> Option<Fetch> {
        let shown = self.shown.as_mut()?;
        if shown.started || !matches!(shown.body, Body::Loading) || now - shown.since < DELAY {
            return None;
        }
        let size = shown.focused.entry.as_ref().and_then(|e| e.size);
        if streams(shown.kind) {
            shown.started = true;
            shown.body = Body::Media(Stream {
                path: shown.focused.path.clone(),
                size,
            });
            return None;
        }
        if let Some(size) = size.filter(|&s| s > cap && whole(shown.kind)) {
            shown.body = Body::TooLarge(too_large(size));
            return None;
        }
        self.next_id += 1;
        shown.started = true;
        shown.fetch = Some(self.next_id);
        Some(Fetch {
            id: self.next_id,
            path: shown.focused.path.clone(),
            len: size.map_or(cap, |s| s.min(cap)),
        })
    }

    /// How long until a fetch is due, for the frame to wake then.
    pub fn wait(&self, now: f64) -> Option<f64> {
        let shown = self.shown.as_ref()?;
        let waiting = !shown.started && matches!(shown.body, Body::Loading);
        waiting.then(|| (shown.since + DELAY - now).max(0.0))
    }

    /// Takes the answer to fetch `id`, keeping the bytes in `place`. An answer for a
    /// preview no longer shown is dropped.
    pub fn arrive(&mut self, id: u64, answer: Result<Vec<u8>, String>, place: &Place) {
        let Some(shown) = self.shown.as_mut() else {
            return;
        };
        if shown.fetch != Some(id) {
            return;
        }
        shown.fetch = None;
        let bytes = match answer {
            Ok(bytes) => bytes,
            Err(e) => {
                shown.body = Body::Failed(failure_line(&e));
                return;
            }
        };
        if shown.kind == Kind::Other {
            shown.kind = sniff(&bytes);
        }
        let total = shown.focused.entry.as_ref().and_then(|e| e.size);
        let fetched_len = bytes.len() as u64;
        let capped = total.is_some_and(|t| t > fetched_len);
        let picture = matches!(shown.kind, Kind::Image | Kind::Pdf);
        if let Some(total) = total.filter(|_| capped && picture) {
            shown.body = Body::TooLarge(too_large(total));
            return;
        }
        if streams(shown.kind) {
            shown.body = Body::Media(Stream {
                path: shown.focused.path.clone(),
                size: total,
            });
            return;
        }
        if picture {
            let pdf = shown.kind == Kind::Pdf;
            shown.body = match place.keep(bytes) {
                Ok(kept) => {
                    let fetched = Fetched {
                        kept,
                        total,
                        capped,
                    };
                    if pdf {
                        Body::Pdf(Pdf { fetched })
                    } else {
                        Body::Image(Image { fetched })
                    }
                }
                Err(e) => Body::Failed(format!("Cannot keep the preview: {e}")),
            };
            return;
        }
        if shown.kind == Kind::Archive {
            let listed = archive::list(&shown.focused.name, &bytes);
            shown.body = match (total.filter(|_| capped), listed) {
                // Sniffed after a capped fetch: only part of it is here.
                (Some(total), _) => Body::TooLarge(too_large(total)),
                (None, None) => Body::Unsupported,
                (None, Some(Ok(listing))) => Body::Archive(listing),
                (None, Some(Err(_))) => Body::Failed(archive::CORRUPT.to_owned()),
            };
            return;
        }
        if !matches!(shown.kind, Kind::Text | Kind::Source | Kind::Markdown) {
            shown.body = Body::Unsupported;
            return;
        }
        let kept = match place.keep(bytes) {
            Ok(kept) => kept,
            Err(e) => {
                shown.body = Body::Failed(format!("Cannot keep the preview: {e}"));
                return;
            }
        };
        let body = match kept.bytes() {
            Ok(bytes) => {
                let bytes = bytes.into_owned();
                let fetched = Fetched {
                    kept,
                    total,
                    capped,
                };
                let text = Text::new(&shown.focused.name, fetched, &bytes);
                if shown.kind == Kind::Markdown {
                    Body::Markdown(Markdown {
                        rendered: crate::markdown::without_images(&text.text),
                        text,
                        view: MarkdownView::Rendered,
                    })
                } else {
                    Body::Text(text)
                }
            }
            Err(e) => Body::Failed(format!("Cannot keep the preview: {e}")),
        };
        shown.body = body;
    }
}

/// The kind of the file `name`, whose first bytes are `head`: by the extension, else by
/// the bytes. [`Kind::Other`] is a file Preview has no kind for.
pub fn detect(name: &str, head: &[u8]) -> Kind {
    match Kind::of(name, false) {
        Kind::Other => sniff(head),
        kind => kind,
    }
}

/// The kind the first bytes show: a known signature, else text when the first [`SNIFF`]
/// bytes have no NUL and are valid UTF-8, else [`Kind::Other`].
pub fn sniff(head: &[u8]) -> Kind {
    const SIGNATURES: &[(usize, &[u8], Kind)] = &[
        (0, b"\x89PNG\r\n\x1a\n", Kind::Image),
        (0, b"\xff\xd8\xff", Kind::Image),
        (0, b"GIF87a", Kind::Image),
        (0, b"GIF89a", Kind::Image),
        (0, b"BM", Kind::Image),
        (0, b"\0\0\x01\0", Kind::Image),
        (0, b"II*\0", Kind::Image),
        (0, b"MM\0*", Kind::Image),
        (0, b"%PDF-", Kind::Pdf),
        (0, b"PK\x03\x04", Kind::Archive),
        (0, b"PK\x05\x06", Kind::Archive),
        (0, b"\x1f\x8b", Kind::Archive),
        (0, b"BZh", Kind::Archive),
        (0, b"\xfd7zXZ\0", Kind::Archive),
        (0, b"\x28\xb5\x2f\xfd", Kind::Archive),
        (0, b"7z\xbc\xaf\x27\x1c", Kind::Archive),
        (0, b"Rar!\x1a\x07", Kind::Archive),
        (257, b"ustar", Kind::Archive),
        (0, b"ID3", Kind::Audio),
        (0, b"fLaC", Kind::Audio),
        (0, b"OggS", Kind::Audio),
        (0, b"\x1a\x45\xdf\xa3", Kind::Video),
    ];
    let at = |offset: usize, sig: &[u8]| head.get(offset..offset + sig.len()) == Some(sig);
    if let Some((_, _, kind)) = SIGNATURES.iter().find(|(o, sig, _)| at(*o, sig)) {
        return *kind;
    }
    if at(0, b"RIFF") {
        match head.get(8..12) {
            Some(b"WEBP") => return Kind::Image,
            Some(b"WAVE") => return Kind::Audio,
            Some(b"AVI ") => return Kind::Video,
            _ => {}
        }
    }
    if at(4, b"ftyp") {
        let brand = head.get(8..12);
        return if matches!(brand, Some(b"M4A ") | Some(b"M4B ")) {
            Kind::Audio
        } else {
            Kind::Video
        };
    }
    // An MPEG audio frame: eleven set sync bits.
    if head.len() >= 2 && head[0] == 0xff && head[1] & 0xe0 == 0xe0 {
        return Kind::Audio;
    }
    let first = &head[..head.len().min(SNIFF)];
    let text = !first.contains(&0)
        && match std::str::from_utf8(first) {
            Ok(_) => true,
            // Cut inside a character at the end: still text.
            Err(e) => e.error_len().is_none(),
        };
    if text { Kind::Text } else { Kind::Other }
}

/// A size as the banner says it: "10 MiB", "2.3 GiB".
pub fn short_size(bytes: u64) -> String {
    size_text(bytes).replace(".0 ", " ")
}

/// "Showing the first 10 MiB of 2.3 GiB".
pub fn capped_banner(shown: u64, total: u64) -> String {
    format!(
        "Showing the first {} of {}",
        short_size(shown),
        short_size(total)
    )
}

/// "Too large to preview (2.3 GiB)", for a file shown only whole that is over the cap.
pub fn too_large(total: u64) -> String {
    format!("Too large to preview ({})", short_size(total))
}

/// The one line a failed read shows, from the reason [`crate::ssh::Session::read`]
/// gives: "No such file" means the file is gone.
pub fn failure_line(reason: &str) -> String {
    if reason == "No such file" {
        return "Cannot read: no longer exists".to_owned();
    }
    let mut chars = reason.chars();
    let reason = match chars.next() {
        Some(first) => first.to_lowercase().chain(chars).collect(),
        None => String::new(),
    };
    format!("Cannot read: {reason}")
}

/// The header's name for a kind.
pub fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Directory => "Directory",
        Kind::Text => "Text",
        Kind::Source => "Source",
        Kind::Markdown => "Markdown",
        Kind::Image => "Image",
        Kind::Pdf => "PDF",
        Kind::Archive => "Archive",
        Kind::Audio => "Audio",
        Kind::Video => "Video",
        Kind::Other => "File",
    }
}
