# Which file types can an egui panel preview?

Read on 2026-11-07 from the crate docs, READMEs, and source linked below. This note answers which of these kinds a Rust egui panel can preview without embedding a browser: plain text, source code with highlighting, Markdown, common images, PDF, and a listing of a zip or tar archive. It names crates that decode or render each kind, records a size or memory limit only when the source that owns the claim states one, and says whether the result can be drawn with egui widgets.

This note does not choose which kinds sshexplorer will support.

Versions are the docs.rs pages read that day: egui 0.37.0 and egui_extras 0.37.0 (both 15 October 2026), egui_commonmark 0.26.0 and egui_commonmark_backend 0.26.0, image 0.25.10 (17 September 2026), syntect 5.3.0, pulldown-cmark 0.13.6, zip 9.4.1, tar 0.4.46 (27 September 2026), flate2 1.1.11, hayro 0.9.0, hayro-syntax 0.9.0, hayro-interpret 0.9.0, vello_cpu 0.3.0, pdfium-render 0.9.5.

## Short answer

All six kinds can be shown in an egui panel without embedding a browser.

| Kind | Decode or render | Drawn with egui widgets? | Limit a primary source states |
| --- | --- | --- | --- |
| Plain text | The string is already text. egui lays it out. | Yes. `Label`, `TextEdit::multiline`, `ScrollArea`. | None for the text itself. `ScrollArea::show_rows` virtualizes uniform rows. `TextEdit::char_limit` applies only to single-line edits. |
| Source with highlighting | `egui_extras::syntax_highlighting`, optionally `syntect` 5.3. | Yes. The helpers return an `egui::text::LayoutJob`, which a `TextEdit` layouter turns into a galley. `code_view_ui` returns a `Response`. | The highlighting module states no size cap. syntect's README states no maximum input size. It does publish benchmark times. |
| Markdown | `egui_commonmark` 0.26, which parses with `pulldown-cmark` 0.13. | Yes. `CommonMarkViewer::show` takes an `egui::Ui`. | pulldown-cmark's crate docs state no input-size cap. The `embedded_image` feature states that a base64 data URL has no size limit. |
| Common images | `image` 0.25, and for SVG `resvg` behind `egui_extras`'s `svg` feature. | Yes. Decode to `ColorImage`, then `Context::load_texture` and `egui::Image`, or hand bytes to `egui::Image` and the `egui_extras` image loader. | `image::Limits::default` is 512MiB `max_alloc` (non-strict) and no width or height cap. `egui::Image::max_size` clamps the widget, not the decode. |
| PDF | `hayro` 0.9 (pure Rust, CPU pixmap) or `pdfium-render` 0.9.5 (loads the Pdfium library at run time). | Yes, after the page is a bitmap: same `ColorImage` / `egui::Image` path. The egui widgets read for this note have no PDF widget. | `hayro::render` casts the scaled page size to `u16` and returns a `vello_cpu::Pixmap`, whose sides are `u16`. Neither crate states a separate memory ceiling. `pdfium-render`'s native docs state no page-count or byte cap; the caller picks the pixel size. |
| Zip or tar listing | `zip` 9.4.1 for zip. `tar` 0.4.46 for tar. `flate2` 1.1.11 in front of `tar` for one gzip member. | Yes. Names are strings, so `Label` rows inside `ScrollArea::show_rows`. | None of these three crates states an archive-size or entry-count cap. |

## Plain text

`egui::Label::new` takes `impl Into<WidgetText>` and is a `Widget`. The same source says a `LayoutJob` can be passed for full control of the text. ([`label.rs`](https://docs.rs/egui/0.37.0/src/egui/widgets/label.rs.html))

`TextEdit::multiline` is a multi-line text region and implements `Widget`. The docs show a read-only selectable preview by passing `&mut` to a `&str`:

```rust
fn selectable_text(ui: &mut egui::Ui, mut text: &str) {
    ui.add(egui::TextEdit::multiline(&mut text));
}
```

`desired_rows` defaults to 4 for multiline. `char_limit` "only works for singleline `TextEdit`". `clip_text` "only works for singleline `TextEdit`". The `TextEdit` page does not state a maximum multiline length or a memory cap. ([`TextEdit`](https://docs.rs/egui/0.37.0/egui/widgets/text_edit/struct.TextEdit.html))

`ScrollArea::show` says: "If the inner area can be very long, consider using `Self::show_rows` instead." `show_rows` is documented as "Efficiently show only the visible part of a large number of rows." Its example sets `total_rows = 10_000` and paints `ui.label` for each visible row. That number is the example's row count, not a documented maximum. `show_viewport` "can be used to only paint the visible part of the contents." `max_width` and `max_height` are the maximum size of the outer frame, and the default is `f32::INFINITY` so the frame expands to the surrounding `Ui`. ([`ScrollArea`](https://docs.rs/egui/0.37.0/egui/containers/scroll_area/struct.ScrollArea.html))

`show_rows` virtualizes rows of one height. It does not, by the signature above, virtualize a single `TextEdit` that holds the whole file. A panel can still draw plain text with `Label` or `TextEdit::multiline` inside a `ScrollArea`. The pages above do not state a byte cap for that text.

## Source code with highlighting

`egui_extras::syntax_highlighting` says: "Turn on the `syntect` feature for great syntax highlighting of any language. Otherwise, a very simple fallback will be used, that works okish for C, C++, JavaScript, TypeScript, JSON, Python, Rust, shell scripts, SQL, TOML, and YAML." The crate feature is documented as "Enable better syntax highlighting using syntect." ([module](https://docs.rs/egui_extras/0.37.0/egui_extras/syntax_highlighting/index.html), [crate features](https://docs.rs/egui_extras/0.37.0/egui_extras/index.html))

`highlight` and `highlight_with` return a `LayoutJob`. Both say "The results are memoized, so you can call this every frame without performance penalty." `highlight_with` takes `&SyntectSettings` and "the `syntect` settings are memoized by address." `code_view_ui` returns a `Response` and is "View some code with syntax highlighting and selection." The module page does not state a size or memory limit. ([`highlight`](https://docs.rs/egui_extras/0.37.0/egui_extras/syntax_highlighting/fn.highlight.html), [`highlight_with`](https://docs.rs/egui_extras/0.37.0/egui_extras/syntax_highlighting/fn.highlight_with.html), [`code_view_ui`](https://docs.rs/egui_extras/0.37.0/egui_extras/syntax_highlighting/fn.code_view_ui.html))

`TextEdit::layouter` is the documented way to show that `LayoutJob` inside a text widget. The layouter "will be called at least once per frame, so it is strongly suggested that you cache the results of any syntax highlighter." The sample calls `ui.fonts_mut(|f| f.layout_job(layout_job))`. `code_editor()` turns a `TextEdit` into a code editor with a monospaced font and focus lock. ([`TextEdit::layouter`](https://docs.rs/egui/0.37.0/egui/widgets/text_edit/struct.TextEdit.html#method.layouter))

`egui_extras` 0.37.0 depends on optional `syntect` ^5.3 with `default_features = false`. ([docs.rs crate sidebar](https://docs.rs/egui_extras/0.37.0/egui_extras/syntax_highlighting/index.html))

syntect 5.3.0 "is a syntax highlighting library for Rust that uses Sublime Text syntax definitions." Its README performance section gives measurements, including "Highlighting 9200 lines/247kb of jQuery 2.1 takes 600ms" on a mid-2012 15-inch Retina MacBook Pro, and "~1.9ms to parse and highlight the 30 line 791 character `testdata/highlight_test.erb` file." The caching section describes copying parse state "every 1000 lines or so" so that an edit re-highlights "only `999+length of viewport` lines." The README does not state a maximum input size or a memory ceiling. Those figures are timings and a caching strategy. ([syntect 5.3.0 README](https://static.crates.io/readmes/syntect/syntect-5.3.0.html))

## Markdown

`egui_commonmark` 0.26.0 documents this use:

```rust
let mut cache = CommonMarkCache::default();
CommonMarkViewer::new().show(ui, &mut cache, markdown);
```

`CommonMarkViewer::show` takes `&mut egui::Ui`. The same type also has `show_mut`, `show_with_id`, and `show_scrollable`. `show_scrollable`'s docs point at `egui::ScrollArea`. ([crate page](https://docs.rs/egui_commonmark/0.26.0/egui_commonmark/), [`show` source](https://docs.rs/egui_commonmark/0.26.0/src/egui_commonmark/lib.rs.html))

The crate depends on `pulldown-cmark`. Its `pulldown_cmark` feature is documented as a "No-op feature." `better_syntax_highlighting` is "Syntax highlighting for code blocks using syntect" and the crate re-exports `egui_commonmark_backend::syntect`. `load-images` is on by default and says to opt into image formats through the `image` crate. `svg` is "Support loading svg images." `fetch` downloads image URLs. ([crate page](https://docs.rs/egui_commonmark/0.26.0/egui_commonmark/))

`embedded_image` "Allows loading base64 image data urls from within markdown files" and says "No size limit is in place for the maximum allowed data in the url." ([same page](https://docs.rs/egui_commonmark/0.26.0/egui_commonmark/))

The backend's `parser_options()` enables `ENABLE_TABLES`, `ENABLE_TASKLISTS`, `ENABLE_STRIKETHROUGH`, `ENABLE_FOOTNOTES`, and `ENABLE_DEFINITION_LIST`. A comment in the same file calls those "Supported pulldown_cmark options." ([`pulldown.rs`](https://docs.rs/egui_commonmark_backend/0.26.0/src/egui_commonmark_backend/pulldown.rs.html))

pulldown-cmark 0.13.6 is a "Pull parser for CommonMark." "By default, only CommonMark features are enabled. To use extensions like tables, footnotes or task lists, enable them by setting the corresponding flags in the `Options` struct." The crate introduction does not state an input-size limit. ([pulldown-cmark](https://docs.rs/pulldown-cmark/0.13.6/pulldown_cmark/))

## Common images

The `image` 0.25.10 README lists the `default-formats` feature as "Format support for common image formats: AVIF, BMP, DDS, EXR, FF, GIF, HDR, ICO, JPEG, PNG, PNM, QOI, TGA, TIFF, and WebP." The documented read path is `ImageReader::open(...).decode()` or `ImageReader::new(...).with_guessed_format()?.decode()`. ([README at v0.25.10](https://raw.githubusercontent.com/image-rs/image/v0.25.10/README.md))

The codecs index adds three qualifiers. AVIF: "Decoding requires the `avif-native` feature, uses the libdav1d C library." DDS: "Only decoding is supported." WebP: "Only lossless encoding is currently supported." ([codecs](https://docs.rs/image/0.25.10/image/codecs/index.html))

`ImageReader::new`, `with_format`, and `open_impl` each set `limits: Limits::default()`. `decode` reserves `decoder.total_bytes()` against those limits and then calls `decoder.set_limits`. `into_decoder` also calls `decoder.set_limits(self.limits)`. `no_limits()` and `limits()` replace that set. The `make_decoder` comment says that for every format except PNG the limits are applied by `ImageDecoder::set_limits` after construction, while PNG receives them in `PngDecoder::with_limits`. ([`image_reader_type.rs`](https://docs.rs/image/0.25.10/src/image/io/image_reader_type.rs.html))

`Limits::default` is `max_image_width: None`, `max_image_height: None`, and `max_alloc: Some(512 * 1024 * 1024)`. The field docs say width and height are strict and "The default is no limit." `max_alloc` "is non-strict by default and some decoders may ignore it." "The bytes required to store the output image count towards this value. The default is 512MiB." `Default` is documented as "not equivalent to not adding limits." `Limits::no_limits()` sets all three fields to `None`. ([`limits.rs`](https://docs.rs/image/0.25.10/src/image/io/limits.rs.html), [`Limits`](https://docs.rs/image/0.25.10/image/struct.Limits.html))

`egui_extras`'s image loader calls `ImageReader::new(reader).with_guessed_format()?`, then `into_decoder()`, and does not call `no_limits()` or `limits()`. That is the default `Limits` above. ([`egui_extras` `image.rs`](https://docs.rs/egui_extras/0.37.0/src/egui_extras/image.rs.html))

The `image` feature of `egui_extras` is "Add support for loading images with the image crate" and "You also need to ALSO opt-in to the image formats you want." Separate features cover `gif`, `webp`, and `svg`. `svg` is "Support loading svg images." The crate depends on optional `resvg` ^0.45.1, so SVG goes through resvg rather than through the `image` crate's format list. ([features](https://docs.rs/egui_extras/0.37.0/egui_extras/index.html), [dependency sidebar](https://docs.rs/egui_extras/0.37.0/egui_extras/syntax_highlighting/index.html))

egui's load stack is `BytesLoader`, then `ImageLoader` ("decode the bytes into an array of colors"), then `TextureLoader` ("ask the backend to put an image onto the GPU"). `ui.image("file://image.png")` walks that stack. ([`egui::load`](https://docs.rs/egui/0.37.0/egui/load/index.html))

`egui::Image` loads when it is added to a `Ui`. `ImageSource::Uri` and `ImageSource::Bytes` use that asynchronous load; `ImageSource::Texture` uses the texture you pass. Constructors are `from_uri`, `from_bytes`, and `from_texture`. `max_width` and `max_size` say "No matter what the image is scaled to, it will never exceed this limit." That is the widget's display size. The `Image` source read here does not state a decode-size or texture-dimension cap. ([`image.rs`](https://docs.rs/egui/0.37.0/src/egui/widgets/image.rs.html))

`ColorImage` is "A 2D RGBA color image in RAM" with `pixels: Vec<Color32>`. `from_rgba_unmultiplied` "is usually what you want to use after having loaded an image file" and panics if `size[0] * size[1] * 4 != rgba.len()`. The published example still calls `image::io::Reader::open`; the `image` 0.25.10 API used above is `image::ImageReader`. `from_rgba_premultiplied` takes the same size and `&[u8]` shape. The `ColorImage` page does not state a maximum width, height, or byte size. ([`ColorImage`](https://docs.rs/egui/0.37.0/egui/struct.ColorImage.html))

`Context::load_texture` takes `impl Into<ImageData>` and "will hand over the image data to the egui backend." `ColorImage` converts into `ImageData`. The method docs point at `ColorImage::from_rgba_unmultiplied` for loading and say most users should use `Ui::image` or `try_load_texture` instead. They do not state a maximum texture side or a byte cap. ([`Context::load_texture`](https://docs.rs/egui/0.37.0/egui/struct.Context.html#method.load_texture))

## PDF

The egui 0.37 widgets read for this note (`Label`, `TextEdit`, `Image`, `ScrollArea`) expose no PDF type. A page preview is a bitmap fed to the image path above.

### hayro

hayro 0.9.0 "allows you to render pages of a PDF file into bitmaps." "We do not have any dependencies on the GPU. All the rendering happens on the CPU." It calls itself "the most comprehensive and feature-complete implementation of a PDF rasterizer in pure Rust" and says "we are currently still very far away from reaching that goal" because "no effort has been put into performance optimizations." Stated limitations include "lack of support for encrypted/password-protected PDF files, blending and isolation, knockout groups" and "color key masking." ([hayro](https://docs.rs/hayro/0.9.0/hayro/))

`hayro::render` returns a `vello_cpu::Pixmap`. It computes the pixmap side as `(width * pixmap_settings.x_scale) as u16` and `(height * pixmap_settings.y_scale) as u16`, where `(width, height)` comes from `page.render_dimensions()`. The function docs do not describe that cast as a checked limit or as an error. Default `PixmapSettings` uses `x_scale: 1.0` and `y_scale: 1.0`. ([`lib.rs`](https://docs.rs/hayro/0.9.0/src/hayro/lib.rs.html))

`Page::render_dimensions` returns `(f32, f32)`, "the width and height of the page that should be assumed when rendering," based on the media box or the crop box and the page rotation. ([`Page`](https://docs.rs/hayro-syntax/0.9.0/hayro_syntax/page/struct.Page.html))

`vello_cpu::Pixmap` is "a pixmap of premultiplied RGBA8 values." `Pixmap::new(width: u16, height: u16)` creates one. `data_as_u8_slice` "returns a reference to the underlying data as premultiplied RGBA8." `take_rgba8` consumes the pixmap and returns raw RGBA8 bytes for a chosen alpha representation. The `Pixmap` page does not state a memory ceiling beyond the `u16` sides. ([`Pixmap`](https://docs.rs/vello_cpu/0.3.0/vello_cpu/struct.Pixmap.html))

Those premultiplied bytes match `ColorImage::from_rgba_premultiplied`, not the unmultiplied helper documented for the `image` crate. ([`ColorImage`](https://docs.rs/egui/0.37.0/egui/struct.ColorImage.html))

hayro-interpret 0.9.0 "forbids unsafe code" and its `embed-fonts` feature embeds substitutes for the 14 standard PDF fonts, "add[ing] around ~240KB to your binary." The crate says it "lacks pretty much any documentation." ([hayro-interpret](https://docs.rs/hayro-interpret/0.9.0/hayro_interpret/))

The same 0.9.0 line's syntax crate exposes `Pdf::new_with_password(data, password)`, which "Try to read the given PDF file with a password" and returns `Err` if the password is incorrect. The rasterizer crate docs, quoted above, list encrypted files as unsupported. Both statements are in the 0.9.0 docs. This note does not resolve them. ([`Pdf`](https://docs.rs/hayro-syntax/0.9.0/hayro_syntax/struct.Pdf.html), [hayro limitations](https://docs.rs/hayro/0.9.0/hayro/))

### pdfium-render

pdfium-render 0.9.5 "binds to a Pdfium library at run-time" and "does not include Pdfium itself." Binding options are a system dynamic library, a dynamic library packaged next to the executable, or a static library linked at compile time. The crate's example loads with `load_pdf_from_file(path, password)`, builds `PdfRenderConfig::new().set_target_width(2000).set_maximum_height(2000)`, and calls `page.render_with_config(&render_config)?.as_image()`, which "Renders this page to an `image::DynamicImage`." The 2000-pixel values are that example's config, not a documented library maximum. ([pdfium-render](https://docs.rs/pdfium-render/0.9.5/pdfium_render/))

`DynamicImage` then follows the `image` path: `to_rgba8()` and `ColorImage::from_rgba_unmultiplied`. ([`ColorImage` example](https://docs.rs/egui/0.37.0/egui/struct.ColorImage.html))

The multi-threading section says "Pdfium makes no guarantees about thread safety and should be assumed not to be thread safe." The `thread_safe` feature "wraps access to Pdfium behind a mutex." The sections read here do not state a maximum page count or file size for a native build. They do say that the WASM builds of Pdfium at `bblanchon/pdfium-binaries`, at the time of writing, use "a non-growable WASM heap memory allocator" and that "attempting to open a PDF document longer than just a few pages will result in an unrecoverable out of memory error." That sentence is about those WASM builds. The same section says a native build binds a library next to the application. ([same crate page](https://docs.rs/pdfium-render/0.9.5/pdfium_render/))

Loading that library is not embedding a browser. The crate also documents a WASM target that can run beside Pdfium in a browser; that target is separate from the native dynamic or static library.

## Zip and tar listings

A listing is names and sizes drawn as text. The widgets are the plain-text ones: `Label` inside `ScrollArea::show_rows`.

### Zip

zip 9.4.1 "supports reading and writing of simple ZIP files." "The current implementation is based on PKWARE's APPNOTE.TXT v6.3.9." Reading support listed in that README includes Stored, Implode, Shrink, Reduce, Deflate, Deflate64, Bzip2, Zstandard, LZMA, XZ, PPMd, AES encryption, and ZipCrypto. Multi-disk is listed as unsupported. The README does not state a maximum archive size or entry count. ([zip 9.4.1 README](https://static.crates.io/readmes/zip/zip-9.4.1.html))

`ZipArchive::new` "uses the central directory record of the ZIP file, and ignores local file headers." `len` is the "Number of files contained in this zip." `file_names` is "an iterator over all the file and directory names in this archive." `by_index_raw` is "Get a contained file by index without decompressing it." `decompressed_size` is the "Total size of the files in the archive, if it can be known." ([`ZipArchive`](https://docs.rs/zip/9.4.1/zip/read/struct.ZipArchive.html))

`Config` has one public field, `archive_offset`. The struct page does not add a size or entry limit. ([`Config`](https://docs.rs/zip/9.4.1/zip/read/struct.Config.html))

`has_overlapping_files` returns whether compressed data belongs to more than one file, and explains that `m` copies of a block that decompresses to length `n` would occupy `m*n` bytes while the archive stays `O(m + n)`. `has_overlapping_entries` covers header overlaps that "increase the number of files" so extracted metadata is "a larger multiple of the archive's size." Those methods describe overlapping entries. They do not document a maximum size or a maximum entry count. ([`ZipArchive`](https://docs.rs/zip/9.4.1/zip/read/struct.ZipArchive.html))

### Tar

tar 0.4.46 "does not currently handle compression" and is "abstract over all I/O readers and writers." "Great lengths are taken to ensure that the entire contents are never required to be entirely resident in memory all at once." ([crate description on the `Archive` page](https://docs.rs/tar/0.4.46/tar/struct.Archive.html))

`Archive::new` wraps a `Read`. `entries` "construct[s] an iterator over the entries." `entries_with_seek`, for `Read + Seek`, says "Seek will be used to efficiently skip over file contents." `Entry::path` returns the path name. `Entry::size` returns "the size of this entry in the archive," from a pax extension when one is present, otherwise from the header. `Entry` also implements `Read` over the entry bytes. The `Archive` and `Entry` pages do not state an entry-count cap or a byte cap. ([`Archive`](https://docs.rs/tar/0.4.46/tar/struct.Archive.html), [`Entry`](https://docs.rs/tar/0.4.46/tar/struct.Entry.html))

Because tar does not decompress, a `.tar.gz` listing is a gzip reader in front of `Archive`. `flate2::read::GzDecoder` is "a decoder for a single member of a gzip file." It implements `Read`: it "reads compressed data from the underlying `Read` and provides the uncompressed data." After one member it returns `Ok(0)`. `MultiGzDecoder` is the documented type for multiple members. ([`GzDecoder`](https://docs.rs/flate2/1.1.11/flate2/read/struct.GzDecoder.html))

## Sources that were checked and do not state a limit

- egui 0.37 `TextEdit`, `Label`, `ColorImage`, `Image`, and `Context::load_texture`: no maximum file size, texture side, or texture byte size.
- `egui_extras::syntax_highlighting`: no size cap. Memoization is a performance note.
- syntect 5.3.0 README: benchmark times and a parse-state cache, no maximum input size.
- pulldown-cmark 0.13.6 crate introduction: no input-size cap. The `embedded_image` feature of egui_commonmark states the opposite for data URLs: no size limit.
- zip 9.4.1 README and `ZipArchive` / `Config`: no archive-size or entry-count cap. Overlap helpers describe expansion and do not set a cap.
- tar 0.4.46 `Archive` and `Entry`: no entry-count or byte cap. The crate says the whole archive need not be resident in memory.
- hayro 0.9.0 and `vello_cpu::Pixmap`: no memory ceiling other than the `u16` pixmap sides produced by the `as u16` cast in `render`.
- pdfium-render 0.9.5 native sections read here: no page-count or file-size cap. The "few pages" out-of-memory sentence is about specific WASM builds of Pdfium.
