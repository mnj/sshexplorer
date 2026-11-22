//! Small PDF files written by hand for the PDF Preview tests: pages of chosen sizes, each
//! filled with one colour, and an encrypted one whose user password is not empty.
#![allow(dead_code)]

/// A PDF with one page per `(width, height)` in points, each filled grey.
pub fn pages(sizes: &[(u32, u32)]) -> Vec<u8> {
    build(sizes, false)
}

/// A PDF of `sizes` encrypted with a user password, which Preview has no way to ask for.
pub fn encrypted(sizes: &[(u32, u32)]) -> Vec<u8> {
    build(sizes, true)
}

fn build(sizes: &[(u32, u32)], encrypt: bool) -> Vec<u8> {
    let n = sizes.len();
    // 1 catalog, 2 pages, then per page: the page and its contents.
    let mut objects: Vec<String> = Vec::new();
    objects.push("<< /Type /Catalog /Pages 2 0 R >>".into());
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", 3 + 2 * i)).collect();
    objects.push(format!(
        "<< /Type /Pages /Kids [{}] /Count {n} >>",
        kids.join(" ")
    ));
    for (i, (w, h)) in sizes.iter().enumerate() {
        let contents = 4 + 2 * i;
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {w} {h}] /Contents {contents} 0 R >>"
        ));
        let stream = format!("0.5 g 0 0 {w} {h} re f");
        objects.push(format!(
            "<< /Length {} >>\nstream\n{stream}\nendstream",
            stream.len()
        ));
    }
    let encrypt_ref = if encrypt {
        let o = "11".repeat(32);
        let u = "22".repeat(32);
        objects.push(format!(
            "<< /Filter /Standard /V 1 /R 2 /Length 40 /P -4 /O <{o}> /U <{u}> >>"
        ));
        Some(objects.len())
    } else {
        None
    };

    let mut out = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n{body}\nendobj\n", i + 1).as_bytes());
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n", objects.len() + 1).as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    let mut trailer = format!("<< /Size {} /Root 1 0 R", objects.len() + 1);
    if let Some(e) = encrypt_ref {
        let id = "33".repeat(16);
        trailer.push_str(&format!(" /Encrypt {e} 0 R /ID [<{id}> <{id}>]"));
    }
    trailer.push_str(" >>");
    out.extend_from_slice(format!("trailer\n{trailer}\nstartxref\n{xref}\n%%EOF\n").as_bytes());
    out
}
