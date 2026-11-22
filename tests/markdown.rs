//! Markdown Preview's rules apart from the window: images become their alt text, and
//! only `http(s)` links leave the app.

use sshexplorer::markdown;

#[test]
fn an_image_becomes_its_alt_text_and_nothing_is_left_to_fetch() {
    assert_eq!(
        markdown::without_images("See ![the logo](../img/logo.png) here.\n"),
        "See the logo here.\n"
    );
    // Absolute and data images too: Preview fetches nothing a Markdown file names.
    assert_eq!(
        markdown::without_images("![badge](https://example.com/b.svg)"),
        "badge"
    );
    // A reference image, and one inside a link, which keeps the link.
    assert_eq!(
        markdown::without_images(
            "[![build](ci.svg)](https://ci.example.com)\n\n![x][r]\n\n[r]: x.png\n"
        ),
        "[build](https://ci.example.com)\n\nx\n\n[r]: x.png\n"
    );
    // Alt text keeps its emphasis; an image with none leaves nothing.
    assert_eq!(
        markdown::without_images("![a *big* cat](c.png)"),
        "a *big* cat"
    );
    assert_eq!(markdown::without_images("a ![](c.png) b"), "a  b");
    // Image syntax in code is code, not an image.
    assert_eq!(
        markdown::without_images("`![x](y.png)`\n\n    ![x](y.png)\n"),
        "`![x](y.png)`\n\n    ![x](y.png)\n"
    );
}

#[test]
fn only_http_and_https_links_open_in_the_browser() {
    assert!(markdown::opens_in_browser("https://example.com/a"));
    assert!(markdown::opens_in_browser("http://example.com"));
    assert!(markdown::opens_in_browser("HTTPS://EXAMPLE.COM"));
    for url in [
        "other.md",
        "../docs/a.md",
        "#usage",
        "/etc/passwd",
        "file:///etc/passwd",
        "mailto:a@b.c",
        "javascript:x",
    ] {
        assert!(!markdown::opens_in_browser(url), "{url}");
    }
}
