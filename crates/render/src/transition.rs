//! Slide transitions: pure pixel composition of two rendered slides.

use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, PixmapPaint, Rect, Transform};

/// Cubic ease-in-out.
pub fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

/// Transition kinds Kinaute can show (all exist in PowerPoint).
pub const KINDS: &[(&str, &str)] = &[
    ("none", "Aucune"),
    ("fade", "Fondu"),
    ("push", "Poussée"),
    ("cover", "Recouvrement"),
    ("pull", "Dévoilement"),
    ("wipe", "Balayage"),
    ("split", "Fractionner"),
    ("dissolve", "Dissolution"),
    ("zoom", "Zoom"),
    ("cut", "Coupure"),
];

fn hash(x: u32, y: u32) -> f32 {
    let mut h = x.wrapping_mul(0x9E37_79B1) ^ y.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    (h & 0xFFFF) as f32 / 65535.0
}

/// Compose the frame at progress `t` (0 = `from`, 1 = `to`, linear in time;
/// easing is applied here). Both pixmaps must have the same size.
pub fn compose(from: &Pixmap, to: &Pixmap, kind: &str, dir: Option<&str>, t: f32) -> Pixmap {
    let (w, h) = (from.width(), from.height());
    let e = ease(t);
    let mut out = Pixmap::new(w, h).unwrap_or_else(|| from.clone());
    let (wf, hf) = (w as f32, h as f32);
    let plain = PixmapPaint::default();
    let draw = |out: &mut Pixmap, src: &Pixmap, dx: f32, dy: f32| {
        out.draw_pixmap(dx.round() as i32, dy.round() as i32, src.as_ref(), &plain, Transform::identity(), None);
    };
    // Unit vector of the content motion for push/cover/pull/wipe.
    let (ux, uy) = match dir.unwrap_or("l") {
        "r" | "ld" | "rd" => (1.0, 0.0),
        "u" | "lu" | "ru" => (0.0, -1.0),
        "d" => (0.0, 1.0),
        _ => (-1.0, 0.0),
    };
    match kind {
        "none" | "cut" => {
            return if t >= 1.0 { to.clone() } else { from.clone() };
        }
        "push" => {
            // Content moves along (ux, uy); the new slide enters from the opposite side.
            draw(&mut out, from, ux * wf * e, uy * hf * e);
            draw(&mut out, to, ux * wf * (e - 1.0), uy * hf * (e - 1.0));
        }
        "cover" => {
            draw(&mut out, from, 0.0, 0.0);
            draw(&mut out, to, ux * wf * (e - 1.0), uy * hf * (e - 1.0));
        }
        "pull" => {
            draw(&mut out, to, 0.0, 0.0);
            draw(&mut out, from, ux * wf * e, uy * hf * e);
        }
        "wipe" => {
            draw(&mut out, from, 0.0, 0.0);
            let rect = if ux != 0.0 {
                let wv = wf * e;
                if ux < 0.0 { Rect::from_xywh(wf - wv, 0.0, wv, hf) } else { Rect::from_xywh(0.0, 0.0, wv, hf) }
            } else {
                let hv = hf * e;
                if uy < 0.0 { Rect::from_xywh(0.0, hf - hv, wf, hv) } else { Rect::from_xywh(0.0, 0.0, wf, hv) }
            };
            clip_draw(&mut out, to, rect);
        }
        "split" => {
            draw(&mut out, from, 0.0, 0.0);
            let vertical = matches!(dir, Some("vert") | Some("u") | Some("d"));
            let rect = if vertical {
                let wv = wf * e;
                Rect::from_xywh((wf - wv) / 2.0, 0.0, wv, hf)
            } else {
                let hv = hf * e;
                Rect::from_xywh(0.0, (hf - hv) / 2.0, wf, hv)
            };
            clip_draw(&mut out, to, rect);
        }
        "dissolve" => {
            let (a, b) = (from.data(), to.data());
            let o = out.data_mut();
            let wu = w as usize;
            for y in 0..h as usize {
                for x in 0..wu {
                    let src = if hash(x as u32 / 3, y as u32 / 3) < t { b } else { a };
                    let i = (y * wu + x) * 4;
                    o[i..i + 4].copy_from_slice(&src[i..i + 4]);
                }
            }
        }
        "zoom" => {
            let zoom_in = !matches!(dir, Some("out"));
            if zoom_in {
                draw(&mut out, from, 0.0, 0.0);
                let s = 0.4 + 0.6 * e;
                let p = PixmapPaint { opacity: e, quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() };
                let tr = Transform::from_translate(wf * (1.0 - s) / 2.0, hf * (1.0 - s) / 2.0).pre_scale(s, s);
                out.draw_pixmap(0, 0, to.as_ref(), &p, tr, None);
            } else {
                draw(&mut out, to, 0.0, 0.0);
                let s = 1.0 + 0.6 * e;
                let p = PixmapPaint { opacity: 1.0 - e, quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() };
                let tr = Transform::from_translate(wf * (1.0 - s) / 2.0, hf * (1.0 - s) / 2.0).pre_scale(s, s);
                out.draw_pixmap(0, 0, from.as_ref(), &p, tr, None);
            }
        }
        // `fade` and anything else Kinaute cannot reproduce exactly.
        _ => {
            draw(&mut out, from, 0.0, 0.0);
            let p = PixmapPaint { opacity: e, ..Default::default() };
            out.draw_pixmap(0, 0, to.as_ref(), &p, Transform::identity(), None);
        }
    }
    out
}

fn clip_draw(out: &mut Pixmap, src: &Pixmap, rect: Option<Rect>) {
    let Some(rect) = rect else { return };
    let mut pb = PathBuilder::new();
    pb.push_rect(rect);
    let Some(path) = pb.finish() else { return };
    let Some(mut mask) = tiny_skia::Mask::new(out.width(), out.height()) else { return };
    mask.fill_path(&path, FillRule::Winding, false, Transform::identity());
    out.draw_pixmap(0, 0, src.as_ref(), &PixmapPaint::default(), Transform::identity(), Some(&mask));
    let _ = Paint::default();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(r: u8) -> Pixmap {
        let mut p = Pixmap::new(40, 20).expect("pixmap");
        p.fill(tiny_skia::Color::from_rgba8(r, 0, 0, 255));
        p
    }

    #[test]
    fn endpoints_match_sources() {
        let (a, b) = (solid(10), solid(200));
        for kind in ["fade", "push", "cover", "pull", "wipe", "split", "dissolve", "zoom"] {
            let s = compose(&a, &b, kind, None, 0.0);
            let e = compose(&a, &b, kind, None, 1.0);
            assert_eq!(s.pixel(5, 5).map(|p| p.red()), Some(10), "{kind} start");
            assert_eq!(e.pixel(5, 5).map(|p| p.red()), Some(200), "{kind} end");
        }
    }
}
