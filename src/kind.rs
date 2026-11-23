//! An entry's Kind: what its icon shows, decided by the name's extension only, never by
//! the contents. Each Kind has one colour of the bundled set (`Tokens::color.kind`).

/// What an entry is, as its icon shows it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Directory,
    Text,
    Source,
    Markdown,
    Image,
    Pdf,
    Archive,
    Audio,
    Video,
    Other,
}

const TEXT: &[&str] = &["txt", "log", "text", "csv", "tsv", "out", "nfo"];
const SOURCE: &[&str] = &[
    "rs", "c", "h", "cc", "cpp", "cxx", "hpp", "hh", "go", "py", "rb", "pl", "php", "java", "kt",
    "kts", "scala", "swift", "m", "mm", "cs", "fs", "js", "mjs", "cjs", "ts", "tsx", "jsx", "lua",
    "sh", "bash", "zsh", "fish", "ps1", "bat", "cmd", "sql", "html", "htm", "css", "scss", "xml",
    "json", "yaml", "yml", "toml", "ini", "conf", "cfg", "nix", "zig", "hs", "ex", "exs", "erl",
    "clj", "el", "vim", "dart", "r", "jl", "tf", "gradle", "cmake", "mk", "diff", "patch",
];
const MARKDOWN: &[&str] = &["md", "markdown", "mdown", "mkd"];
const IMAGE: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "svg", "ico", "tif", "tiff", "avif", "heic",
];
const PDF: &[&str] = &["pdf"];
const ARCHIVE: &[&str] = &[
    "zip", "tar", "gz", "tgz", "bz2", "tbz", "tbz2", "xz", "txz", "zst", "7z", "rar", "jar", "deb",
    "rpm", "iso",
];
const AUDIO: &[&str] = &[
    "mp3", "wav", "flac", "ogg", "oga", "opus", "m4a", "aac", "wma",
];
const VIDEO: &[&str] = &[
    "mp4", "m4v", "mkv", "webm", "mov", "avi", "wmv", "mpg", "mpeg", "ogv", "ts",
];

impl Kind {
    /// The Kind of an entry named `name`: a directory, or by the name's last extension
    /// ignoring case. A dot-name with no other dot has no extension.
    pub fn of(name: &str, is_dir: bool) -> Kind {
        if is_dir {
            return Kind::Directory;
        }
        let Some((stem, ext)) = name.rsplit_once('.') else {
            return Kind::Other;
        };
        if stem.is_empty() || ext.is_empty() {
            return Kind::Other;
        }
        let ext = ext.to_ascii_lowercase();
        let ext = ext.as_str();
        // `.ts` is TypeScript before it is an MPEG transport stream.
        [
            (SOURCE, Kind::Source),
            (TEXT, Kind::Text),
            (MARKDOWN, Kind::Markdown),
            (IMAGE, Kind::Image),
            (PDF, Kind::Pdf),
            (ARCHIVE, Kind::Archive),
            (AUDIO, Kind::Audio),
            (VIDEO, Kind::Video),
        ]
        .into_iter()
        .find(|(exts, _)| exts.contains(&ext))
        .map_or(Kind::Other, |(_, kind)| kind)
    }
}
