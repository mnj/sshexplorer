//! Source Preview's highlighting: the language by name or shebang, and the colours of
//! the light and dark themes.

use std::sync::atomic::AtomicBool;

use sshexplorer::source::{self, Span};

#[test]
fn the_language_comes_from_the_extension_or_the_shebang() {
    assert_eq!(source::syntax_for("main.rs", ""), Some("Rust"));
    assert_eq!(source::syntax_for("Main.PY", ""), Some("Python"));
    assert_eq!(source::syntax_for("Makefile", ""), Some("Makefile"));
    assert_eq!(
        source::syntax_for("deploy", "#!/usr/bin/env python3"),
        Some("Python")
    );
    assert_eq!(
        source::syntax_for("run", "#!/bin/bash"),
        Some("Bourne Again Shell (bash)")
    );
    // The extension wins over the first line.
    assert_eq!(source::syntax_for("x.rs", "#!/bin/bash"), Some("Rust"));
    // Text with no known language is plain.
    assert_eq!(source::syntax_for("notes.txt", "hello"), None);
    assert_eq!(source::syntax_for("blob", "hello"), None);
}

fn ink_of(spans: &[Span], line: &str, word: &str) -> [u8; 3] {
    let at = line.find(word).unwrap();
    spans
        .iter()
        .find(|s| s.start <= at && at < s.end)
        .unwrap()
        .ink
}

#[test]
fn highlighting_colours_each_line_in_the_theme_for_light_or_dark() {
    let lines = ["fn main() {", "    let x = \"hi\";", "}"];
    let stop = AtomicBool::new(false);
    let light = source::highlight(lines, "Rust", false, &stop).unwrap();
    let dark = source::highlight(lines, "Rust", true, &stop).unwrap();
    assert_eq!(light.len(), 3);
    // The spans cover each line exactly.
    for (spans, line) in light.iter().zip(lines) {
        assert_eq!(spans.first().map(|s| s.start), Some(0));
        assert_eq!(spans.last().map(|s| s.end), Some(line.len()));
    }
    // A keyword and a string differ in colour, and the themes differ.
    let keyword = ink_of(&light[1], lines[1], "let");
    let string = ink_of(&light[1], lines[1], "hi");
    assert_ne!(keyword, string);
    assert_ne!(keyword, ink_of(&dark[1], lines[1], "let"));

    let stopped = AtomicBool::new(true);
    assert!(source::highlight(lines, "Rust", false, &stopped).is_none());
}
