# hayro renders PDF

The spec leaves PDF Preview to hayro or pdfium-render (issue #3). sshexplorer uses hayro 0.9 (ticket #36). It is pure Rust with `unsafe` forbidden, so nothing is installed on the build machine or shipped beside the app on Linux, macOS or Windows. Its standard fonts and CMaps are embedded. It is licensed Apache-2.0 OR MIT. It covers what Preview needs: open a document, get page sizes, rasterise a page at a scale. It also covers what the spec leaves out: there is no text selection and no password prompt.

## Considered Options

- **pdfium-render:** renders more faithfully and faster, but binds to a Pdfium shared library at run time. Each platform would need its own build shipped with the app (for example from bblanchon/pdfium-binaries), and every build or test machine would need one installed. It is not installed here. Its licences (MIT/Apache, Pdfium BSD-3) would also have been fine.

## Consequences

- hayro's render cache is not `Send`, so each open PDF has one renderer thread that owns the document and its cache (`src/pdf.rs`). The frame thread sends it the pages in view, and only the newest list counts.
- `Pdf::new` tries the empty user password, so a PDF with only an owner password opens. Any other decryption failure shows "Cannot preview: encrypted PDF".
- hayro is young. Some PDFs may render imperfectly, and a panic while reading or rendering one fails that preview with "Cannot preview: not a valid PDF" instead of taking down the app.
- Page sizes are clamped to the GPU's texture limit before rendering, since hayro casts pixel sizes to `u16` without checking.
