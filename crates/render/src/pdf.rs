//! Minimal PDF writer: one JPEG image per page (slides are rasterised).

use std::io::Write;

pub struct Page {
    pub jpeg: Vec<u8>,
    pub width_px: u32,
    pub height_px: u32,
}

/// Assemble a PDF. Page size is given in PostScript points.
pub fn write_pdf(pages: &[Page], page_w_pt: f32, page_h_pt: f32, title: &str) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    let mut offsets: Vec<usize> = Vec::new();
    let _ = out.write_all(b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n");
    let n = pages.len();
    // Object numbering: 1 catalog, 2 pages, 3 info, then per page: page, content, image.
    let page_obj = |i: usize| 4 + i * 3;
    let obj = |out: &mut Vec<u8>, offsets: &mut Vec<usize>, id: usize, body: &[u8]| {
        while offsets.len() < id {
            offsets.push(0);
        }
        offsets[id - 1] = out.len();
        let _ = write!(out, "{id} 0 obj\n");
        let _ = out.write_all(body);
        let _ = out.write_all(b"\nendobj\n");
    };
    obj(&mut out, &mut offsets, 1, b"<< /Type /Catalog /Pages 2 0 R >>");
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", page_obj(i))).collect();
    obj(&mut out, &mut offsets, 2, format!("<< /Type /Pages /Count {n} /Kids [{}] >>", kids.join(" ")).as_bytes());
    let esc: String = title.chars().filter(|c| c.is_ascii() && !matches!(c, '(' | ')' | '\\')).collect();
    obj(&mut out, &mut offsets, 3, format!("<< /Title ({esc}) /Producer (Kinaute) >>").as_bytes());
    for (i, p) in pages.iter().enumerate() {
        let (pg, ct, im) = (page_obj(i), page_obj(i) + 1, page_obj(i) + 2);
        obj(
            &mut out,
            &mut offsets,
            pg,
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {page_w_pt:.2} {page_h_pt:.2}] /Resources << /XObject << /Im0 {im} 0 R >> >> /Contents {ct} 0 R >>"
            )
            .as_bytes(),
        );
        let content = format!("q {page_w_pt:.2} 0 0 {page_h_pt:.2} 0 0 cm /Im0 Do Q");
        obj(&mut out, &mut offsets, ct, format!("<< /Length {} >>\nstream\n{content}\nendstream", content.len()).as_bytes());
        let mut body = format!(
            "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",
            p.width_px,
            p.height_px,
            p.jpeg.len()
        )
        .into_bytes();
        body.extend_from_slice(&p.jpeg);
        body.extend_from_slice(b"\nendstream");
        obj(&mut out, &mut offsets, im, &body);
    }
    let xref = out.len();
    let total = offsets.len() + 1;
    let _ = write!(out, "xref\n0 {total}\n0000000000 65535 f \n");
    for o in &offsets {
        let _ = write!(out, "{o:010} 00000 n \n");
    }
    let _ = write!(out, "trailer\n<< /Size {total} /Root 1 0 R /Info 3 0 R >>\nstartxref\n{xref}\n%%EOF\n");
    out
}

/// Encode an RGBA pixmap as JPEG on a white background.
pub fn pixmap_to_jpeg(pm: &tiny_skia::Pixmap, quality: u8) -> Option<Vec<u8>> {
    let (w, h) = (pm.width(), pm.height());
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    for px in pm.pixels() {
        // Composite premultiplied pixels over white.
        let a = px.alpha() as u32;
        let over = |c: u8| (c as u32 + (255 - a)).min(255) as u8;
        rgb.extend_from_slice(&[over(px.red()), over(px.green()), over(px.blue())]);
    }
    let mut buf = Vec::new();
    let enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut buf, quality);
    image::ImageEncoder::write_image(enc, &rgb, w, h, image::ExtendedColorType::Rgb8).ok()?;
    Some(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_a_structurally_valid_pdf() {
        let mut pm = tiny_skia::Pixmap::new(32, 24).expect("pixmap");
        pm.fill(tiny_skia::Color::from_rgba8(200, 30, 30, 255));
        let jpeg = pixmap_to_jpeg(&pm, 80).expect("jpeg");
        let pdf = write_pdf(&[Page { jpeg, width_px: 32, height_px: 24 }], 720.0, 540.0, "Test");
        let text = String::from_utf8_lossy(&pdf);
        assert!(text.starts_with("%PDF-1.4"));
        assert!(text.contains("/Count 1"));
        assert!(text.trim_end().ends_with("%%EOF"));
    }
}
