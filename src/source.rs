//! Source highlighting for Preview, with syntect used directly (issue #33's notes):
//! syntect's default syntaxes, picked by the file's name or its first line (a shebang),
//! and a light/dark theme pair that follows Theme.
//!
//! Highlighting a capped 10 MiB file takes seconds, so it runs on a thread of its own,
//! line by line, while the panel shows the text plain. [`Highlighting`] is that run; it
//! stops when dropped.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, mpsc};

use syntect::easy::HighlightLines;
use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::SyntaxSet;

/// The syntect theme for the light Theme: dark inks that read on Paper's light surfaces.
pub const LIGHT_THEME: &str = "InspiredGitHub";
/// The syntect theme for the dark Theme.
pub const DARK_THEME: &str = "base16-ocean.dark";

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(SyntaxSet::load_defaults_newlines)
}

fn theme(dark: bool) -> &'static Theme {
    static SET: OnceLock<ThemeSet> = OnceLock::new();
    let set = SET.get_or_init(ThemeSet::load_defaults);
    &set.themes[if dark { DARK_THEME } else { LIGHT_THEME }]
}

/// The syntax for the file `name` whose first line is `first_line`: by the extension (or
/// a whole name syntect knows, such as `Makefile`), else by the first line. `None` is
/// text with no known language, shown plain.
pub fn syntax_for(name: &str, first_line: &str) -> Option<&'static str> {
    let set = syntaxes();
    let by_name = name
        .rsplit_once('.')
        .filter(|(stem, ext)| !stem.is_empty() && !ext.is_empty())
        .and_then(|(_, ext)| {
            set.find_syntax_by_extension(ext)
                .or_else(|| set.find_syntax_by_extension(&ext.to_ascii_lowercase()))
        })
        .or_else(|| set.find_syntax_by_extension(name));
    let syntax = by_name.or_else(|| set.find_syntax_by_first_line(first_line))?;
    let plain = set.find_syntax_plain_text();
    (syntax.name != plain.name).then_some(syntax.name.as_str())
}

/// One coloured run of a line: bytes `start..end` of it, in `ink` (RGB).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub ink: [u8; 3],
}

/// Highlights `lines` (each without its line ending) with the syntax named `syntax`, in
/// the dark or light theme. Stops early, returning `None`, when `stop` turns true.
pub fn highlight<'a>(
    lines: impl IntoIterator<Item = &'a str>,
    syntax: &str,
    dark: bool,
    stop: &AtomicBool,
) -> Option<Vec<Vec<Span>>> {
    let set = syntaxes();
    let syntax = set
        .find_syntax_by_name(syntax)
        .unwrap_or_else(|| set.find_syntax_plain_text());
    let mut h = HighlightLines::new(syntax, theme(dark));
    let mut out = Vec::new();
    let mut line_nl = String::new();
    for line in lines {
        if stop.load(Ordering::Relaxed) {
            return None;
        }
        line_nl.clear();
        line_nl.push_str(line);
        line_nl.push('\n');
        let mut spans = Vec::new();
        let mut at = 0;
        // A line the grammar cannot take is left plain.
        if let Ok(regions) = h.highlight_line(&line_nl, set) {
            for (style, piece) in regions {
                let end = (at + piece.len()).min(line.len());
                if end > at {
                    let c = style.foreground;
                    spans.push(Span {
                        start: at,
                        end,
                        ink: [c.r, c.g, c.b],
                    });
                }
                at += piece.len();
            }
        }
        out.push(spans);
    }
    Some(out)
}

/// A highlighting run on its own thread. Dropping it stops the run.
pub struct Highlighting {
    /// The theme it highlights for.
    pub dark: bool,
    answer: mpsc::Receiver<Vec<Vec<Span>>>,
    done: Option<Vec<Vec<Span>>>,
    stop: Arc<AtomicBool>,
}

impl Highlighting {
    /// Starts highlighting the `lines` (byte ranges) of `text`. `wake` is called when it
    /// is done, from the run's thread.
    pub fn start(
        text: Arc<str>,
        lines: Arc<[(usize, usize)]>,
        syntax: &'static str,
        dark: bool,
        wake: impl Fn() + Send + 'static,
    ) -> Highlighting {
        let (sender, answer) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let spawned = std::thread::Builder::new()
            .name("highlight".into())
            .spawn(move || {
                let lines = lines.iter().map(|&(s, e)| &text[s..e]);
                if let Some(spans) = highlight(lines, syntax, dark, &stopped) {
                    let _ = sender.send(spans);
                    wake();
                }
            });
        if spawned.is_err() {
            // Without a thread the text stays plain.
            stop.store(true, Ordering::Relaxed);
        }
        Highlighting {
            dark,
            answer,
            done: None,
            stop,
        }
    }

    /// Takes the run's answer if it has arrived.
    pub fn poll(&mut self) {
        if self.done.is_none() {
            self.done = self.answer.try_recv().ok();
        }
    }

    /// The spans per line, once the run is done and polled.
    pub fn spans(&self) -> Option<&[Vec<Span>]> {
        self.done.as_deref()
    }
}

impl Drop for Highlighting {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
