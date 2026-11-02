//! CPU renderer for slides.
//!
//! Everything is drawn with tiny-skia (anti-aliased vector rasteriser) and
//! cosmic-text (shaping and layout). No GPU is required, so this works on
//! very old machines; it is also the reference for tests and export.

mod geom;
mod text;

pub use text::TextMetrics;

use kinaute_model::resolve::{Ctx, Level};
use kinaute_model::*;
use std::collections::HashMap;
use std::sync::Arc;
use tiny_skia::{
    BlendMode, FillRule, FilterQuality, GradientStop, LinearGradient, Paint, Path, PathBuilder, Pixmap, PixmapPaint,
    Point, RadialGradient, SpreadMode, Stroke, StrokeDash, Transform,
};

const EMU_PT: f32 = 12700.0;

/// Reusable renderer: owns the font system and image caches.
pub struct Renderer {
    /// Device pixels per EMU of the slide being rendered.
    cur_k: f32,
    pub(crate) text: text::TextEngine,
    images: HashMap<(String, u32), Option<Arc<Pixmap>>>,
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-render state.
struct Scene {
    /// Device pixels per EMU of the slide (font sizes and insets use this,
    /// never a group's scale).
    k: f32,
    slide_no: usize,
    slide_count: usize,
}

impl Renderer {
    pub fn new() -> Self {
        Renderer { cur_k: 1.0, text: text::TextEngine::new(), images: HashMap::new() }
    }

    /// Render slide `index` at `width` pixels wide (height follows the aspect ratio).
    pub fn render_slide(&mut self, pres: &Presentation, index: usize, width: u32) -> Option<Pixmap> {
        let slide = pres.slides.get(index)?;
        let width = width.max(16);
        let height = ((width as f64) * pres.slide_h as f64 / pres.slide_w as f64).round().max(1.0) as u32;
        let mut pm = Pixmap::new(width, height)?;
        let k = width as f32 / pres.slide_w as f32;
        let base = Transform::from_scale(k, k);
        self.cur_k = k;
        let scene = Scene { k, slide_no: index + 1, slide_count: pres.slides.len() };

        let ctx = Ctx::for_slide(pres, slide);
        // Background.
        pm.fill(tiny_skia::Color::WHITE);
        if let Some((fill, ph)) = ctx.background() {
            let full = Xfrm::new(0, 0, pres.slide_w, pres.slide_h);
            self.fill_box(&mut pm, &ctx, &fill, ph.as_ref(), &full, base);
        }
        // Master and layout decoration.
        let layout = pres.layout_of(slide);
        if slide.show_master_shapes {
            if let Some(l) = layout {
                if l.show_master_shapes {
                    if let Some(m) = pres.master_of_layout(l) {
                        let mctx = Ctx::for_master(pres, m);
                        self.draw_shapes(&mut pm, &scene, &mctx, &m.shapes, base, true);
                    }
                }
                let lctx = Ctx::for_layout(pres, l);
                self.draw_shapes(&mut pm, &scene, &lctx, &l.shapes, base, true);
            }
        }
        self.draw_shapes(&mut pm, &scene, &ctx, &slide.shapes, base, false);
        Some(pm)
    }

    fn draw_shapes(&mut self, pm: &mut Pixmap, sc: &Scene, ctx: &Ctx, shapes: &[Shape], base: Transform, skip_ph: bool) {
        for s in shapes {
            if s.hidden || (skip_ph && s.ph.is_some()) {
                continue;
            }
            self.draw_shape(pm, sc, ctx, s, base);
        }
    }

    fn draw_shape(&mut self, pm: &mut Pixmap, sc: &Scene, ctx: &Ctx, s: &Shape, base: Transform) {
        if let ShapeKind::Opaque { fallback } = &s.kind {
            for f in fallback {
                if !f.hidden {
                    self.draw_shape(pm, sc, ctx, f, base);
                }
            }
            return;
        }
        let Some(x) = ctx.xfrm(s) else { return };
        if x.w <= 0 && x.h <= 0 {
            return;
        }
        let t = place(base, &x);
        let (w, h) = (x.w as f32, x.h as f32);

        match &s.kind {
            ShapeKind::Shape(sp) | ShapeKind::Connector(sp) => {
                let geom = sp
                    .geom
                    .clone()
                    .or_else(|| s.ph.as_ref().and_then(|_| ctx.ph_chain(s).iter().find_map(|c| c.sp().and_then(|p| p.geom.clone()))))
                    .unwrap_or_else(|| Geometry::preset("rect"));
                let open = matches!(&geom, Geometry::Preset { name, .. } if geom::is_open(name))
                    || matches!(s.kind, ShapeKind::Connector(_));
                let path = geom::path_for(&geom, w, h, x.w as f64, x.h as f64);
                if let Some(path) = &path {
                    if !open {
                        if let Some((sh, ph)) = ctx.shape_shadow(s) {
                            self.draw_shadow(pm, ctx, path, t, &sh, ph.as_ref());
                        }
                        if let Some((fill, ph)) = ctx.shape_fill(s) {
                            self.fill_path(pm, ctx, path, &fill, ph.as_ref(), &x, t);
                        }
                    }
                    if let Some((line, ph)) = ctx.shape_line(s) {
                        self.stroke_path(pm, ctx, path, &line, ph.as_ref(), t, open);
                    }
                }
                if let Some(tb) = &sp.text {
                    self.draw_text(pm, sc, ctx, s, tb, &x, t, None);
                }
            }
            ShapeKind::Picture(pic) => {
                let geom = pic.geom.clone().unwrap_or_else(|| Geometry::preset("rect"));
                if let Some(path) = geom::path_for(&geom, w, h, x.w as f64, x.h as f64) {
                    if let Some((sh, ph)) = ctx.shape_shadow(s) {
                        self.draw_shadow(pm, ctx, &path, t, &sh, ph.as_ref());
                    }
                    if !self.fill_image(pm, &pic.image, &path, &x, t) {
                        // Missing or undecodable image: draw a neutral placeholder.
                        let mut p = Paint::default();
                        p.set_color_rgba8(0xE6, 0xE6, 0xE6, 0xFF);
                        pm.fill_path(&path, &p, FillRule::Winding, t, None);
                    }
                    if let Some((line, ph)) = ctx.shape_line(s) {
                        self.stroke_path(pm, ctx, &path, &line, ph.as_ref(), t, false);
                    }
                }
            }
            ShapeKind::Group(g) => {
                let ch = g.child;
                let sx = if ch.w != 0 { x.w as f32 / ch.w as f32 } else { 1.0 };
                let sy = if ch.h != 0 { x.h as f32 / ch.h as f32 } else { 1.0 };
                let child_base = t.pre_scale(sx, sy).pre_translate(-ch.x as f32, -ch.y as f32);
                self.draw_shapes(pm, sc, ctx, &g.shapes, child_base, false);
            }
            ShapeKind::Frame(f) => match &f.content {
                FrameContent::Table(tbl) => self.draw_table(pm, sc, ctx, tbl, &x, t),
                FrameContent::Diagram if !f.drawing.is_empty() => self.draw_shapes(pm, sc, ctx, &f.drawing, t, false),
                other => {
                    let label = match other {
                        FrameContent::Chart { .. } => "Graphique",
                        FrameContent::Diagram => "Diagramme SmartArt",
                        FrameContent::Ole => "Objet incorporé",
                        _ => "Contenu non affiché",
                    };
                    self.draw_placeholder(pm, sc, ctx, &x, t, label);
                }
            },
            ShapeKind::Opaque { .. } => {}
        }
    }

    // ----------------------------------------------------------- paints --

    fn solid(&self, ctx: &Ctx, c: &Color, ph: Option<&Color>) -> tiny_skia::Color {
        let [r, g, b, a] = ctx.color_with(c, ph).to_u8();
        tiny_skia::Color::from_rgba8(r, g, b, a)
    }

    fn paint_for(&self, ctx: &Ctx, fill: &Fill, ph: Option<&Color>, x: &Xfrm) -> Option<Paint<'static>> {
        let mut p = Paint::default();
        p.anti_alias = true;
        match fill {
            Fill::None | Fill::Group | Fill::Image(_) => return None,
            Fill::Solid(c) => p.set_color(self.solid(ctx, c, ph)),
            Fill::Pattern { fg, bg, preset } => {
                // Approximate patterns by blending foreground and background.
                let f = fg.as_ref().map(|c| ctx.color_with(c, ph)).unwrap_or(Rgba::BLACK);
                let b = bg.as_ref().map(|c| ctx.color_with(c, ph)).unwrap_or(Rgba::WHITE);
                let k = pattern_coverage(preset);
                let m = |a: f32, b: f32| a * k + b * (1.0 - k);
                let c = Rgba { r: m(f.r, b.r), g: m(f.g, b.g), b: m(f.b, b.b), a: m(f.a, b.a) };
                let [r, g, bl, a] = c.to_u8();
                p.set_color_rgba8(r, g, bl, a);
            }
            Fill::Gradient(g) => {
                let mut raw: Vec<(f32, tiny_skia::Color)> = g
                    .stops
                    .iter()
                    .map(|(pos, c)| ((*pos as f32 / 100_000.0).clamp(0.0, 1.0), self.solid(ctx, c, ph)))
                    .collect();
                raw.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
                let stops: Vec<GradientStop> = raw.iter().map(|(pos, c)| GradientStop::new(*pos, *c)).collect();
                if raw.len() == 1 {
                    p.set_color(raw[0].1);
                } else {
                    let (w, h) = (x.w as f32, x.h as f32);
                    let shader = match &g.kind {
                        GradientKind::Linear { angle, .. } => {
                            let th = (*angle as f32 / 60_000.0).to_radians();
                            let (dx, dy) = (th.cos(), th.sin());
                            let len = (w * dx).abs() + (h * dy).abs();
                            let (cx, cy) = (w / 2.0, h / 2.0);
                            LinearGradient::new(
                                Point::from_xy(cx - dx * len / 2.0, cy - dy * len / 2.0),
                                Point::from_xy(cx + dx * len / 2.0, cy + dy * len / 2.0),
                                stops,
                                SpreadMode::Pad,
                                Transform::identity(),
                            )
                        }
                        GradientKind::Path { focus, .. } => {
                            let fx = (focus.0 as f32 + (100_000 - focus.2) as f32) / 200_000.0;
                            let fy = (focus.1 as f32 + (100_000 - focus.3) as f32) / 200_000.0;
                            let c = Point::from_xy(w * fx, h * fy);
                            let r = (w.max(h) * 0.75).max(1.0);
                            RadialGradient::new(c, 0.0, c, r, stops, SpreadMode::Pad, Transform::identity())
                        }
                    };
                    p.shader = shader?;
                }
            }
        }
        Some(p)
    }

    fn fill_path(&mut self, pm: &mut Pixmap, ctx: &Ctx, path: &Path, fill: &Fill, ph: Option<&Color>, x: &Xfrm, t: Transform) {
        if let Fill::Image(img) = fill {
            let _ = self.fill_image(pm, img, path, x, t);
            return;
        }
        if let Some(p) = self.paint_for(ctx, fill, ph, x) {
            pm.fill_path(path, &p, FillRule::Winding, t, None);
        }
    }

    /// Fill a whole box (backgrounds).
    fn fill_box(&mut self, pm: &mut Pixmap, ctx: &Ctx, fill: &Fill, ph: Option<&Color>, x: &Xfrm, base: Transform) {
        let Some(path) = geom::preset("rect", &[], x.w as f32, x.h as f32) else { return };
        self.fill_path(pm, ctx, &path, fill, ph, x, place(base, x));
    }

    fn stroke_path(&mut self, pm: &mut Pixmap, ctx: &Ctx, path: &Path, line: &Line, ph: Option<&Color>, t: Transform, open: bool) {
        let Some(fill) = &line.fill else { return };
        let Fill::Solid(c) = fill else {
            // Gradient outlines are approximated with their first stop.
            if let Fill::Gradient(g) = fill {
                if let Some((_, c)) = g.stops.first() {
                    self.stroke_with(pm, path, self.solid(ctx, c, ph), line, t, open);
                }
            }
            return;
        };
        self.stroke_with(pm, path, self.solid(ctx, c, ph), line, t, open);
    }

    fn stroke_with(&mut self, pm: &mut Pixmap, path: &Path, color: tiny_skia::Color, line: &Line, t: Transform, open: bool) {
        // Line widths keep their size inside scaled groups.
        let local = (t.sx * t.sy - t.kx * t.ky).abs().sqrt();
        let width = line.width.unwrap_or(12700).max(1) as f32 * if local > 0.0 { self.cur_k / local } else { 1.0 };
        let mut stroke = Stroke { width, ..Default::default() };
        stroke.line_cap = match line.cap.as_deref() {
            Some("rnd") => tiny_skia::LineCap::Round,
            Some("sq") => tiny_skia::LineCap::Square,
            _ => tiny_skia::LineCap::Butt,
        };
        stroke.line_join = match line.join.as_deref() {
            Some("round") => tiny_skia::LineJoin::Round,
            Some("bevel") => tiny_skia::LineJoin::Bevel,
            _ => tiny_skia::LineJoin::Miter,
        };
        if let Some(d) = line.dash.as_deref() {
            let pat: &[f32] = match d {
                "dash" | "sysDash" => &[4.0, 3.0],
                "lgDash" => &[8.0, 3.0],
                "dot" | "sysDot" => &[1.0, 2.0],
                "dashDot" | "sysDashDot" => &[4.0, 3.0, 1.0, 3.0],
                "lgDashDot" => &[8.0, 3.0, 1.0, 3.0],
                "lgDashDotDot" | "sysDashDotDot" => &[8.0, 3.0, 1.0, 3.0, 1.0, 3.0],
                _ => &[],
            };
            if !pat.is_empty() {
                let scaled: Vec<f32> = pat.iter().map(|v| v * width.max(9525.0)).collect();
                stroke.dash = StrokeDash::new(scaled, 0.0);
            }
        }
        let mut p = Paint::default();
        p.anti_alias = true;
        p.set_color(color);
        pm.stroke_path(path, &p, &stroke, t, None);
        if open {
            for (end, kind) in [(false, line.head.as_deref()), (true, line.tail.as_deref())] {
                if let Some(k) = kind.filter(|k| *k != "none") {
                    self.arrow_head(pm, path, end, k, width, &p, t);
                }
            }
        }
    }

    fn arrow_head(&self, pm: &mut Pixmap, path: &Path, at_end: bool, kind: &str, width: f32, paint: &Paint, t: Transform) {
        let pts = path.points();
        if pts.len() < 2 {
            return;
        }
        let (tip, from) = if at_end { (pts[pts.len() - 1], pts[pts.len() - 2]) } else { (pts[0], pts[1]) };
        let (dx, dy) = (tip.x - from.x, tip.y - from.y);
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1e-3 {
            return;
        }
        let (ux, uy) = (dx / len, dy / len);
        let s = (width * 3.0).max(38100.0);
        let mut pb = PathBuilder::new();
        match kind {
            "oval" => {
                if let Some(r) = tiny_skia::Rect::from_xywh(tip.x - s / 2.0, tip.y - s / 2.0, s, s) {
                    pb.push_oval(r);
                }
            }
            "diamond" => {
                pb.move_to(tip.x, tip.y - s / 2.0);
                pb.line_to(tip.x + s / 2.0, tip.y);
                pb.line_to(tip.x, tip.y + s / 2.0);
                pb.line_to(tip.x - s / 2.0, tip.y);
                pb.close();
            }
            _ => {
                let (bx, by) = (tip.x - ux * s, tip.y - uy * s);
                pb.move_to(tip.x, tip.y);
                pb.line_to(bx - uy * s / 2.0, by + ux * s / 2.0);
                pb.line_to(bx + uy * s / 2.0, by - ux * s / 2.0);
                pb.close();
            }
        }
        if let Some(p) = pb.finish() {
            pm.fill_path(&p, paint, FillRule::Winding, t, None);
        }
    }

    fn draw_shadow(&mut self, pm: &mut Pixmap, ctx: &Ctx, path: &Path, t: Transform, sh: &Shadow, ph: Option<&Color>) {
        let k = self.cur_k;
        let dir = (sh.dir as f32 / 60_000.0).to_radians();
        let (ox, oy) = (dir.cos() * sh.dist as f32 * k, dir.sin() * sh.dist as f32 * k);
        let blur = sh.blur as f32 * k;
        let Some(b) = path.clone().transform(t).map(|p| p.bounds()) else { return };
        let pad = blur * 2.0 + 2.0;
        let (x0, y0) = ((b.left() + ox - pad).floor(), (b.top() + oy - pad).floor());
        let (w, h) = ((b.width() + pad * 2.0).ceil() as u32 + 1, (b.height() + pad * 2.0).ceil() as u32 + 1);
        if w == 0 || h == 0 || w > 8192 || h > 8192 {
            return;
        }
        let Some(mut tmp) = Pixmap::new(w, h) else { return };
        let mut p = Paint::default();
        p.anti_alias = true;
        p.set_color(self.solid(ctx, &sh.color, ph));
        tmp.fill_path(path, &p, FillRule::Winding, t.post_translate(ox - x0, oy - y0), None);
        box_blur(&mut tmp, (blur / 2.0).round() as usize, 3);
        pm.draw_pixmap(x0 as i32, y0 as i32, tmp.as_ref(), &PixmapPaint::default(), Transform::identity(), None);
    }

    // ----------------------------------------------------------- images --

    fn image_for(&mut self, media: &str, pres_media: &MediaStore, want_w: u32) -> Option<Arc<Pixmap>> {
        let bucket = (want_w.max(1) + 63) / 64 * 64;
        let key = (media.to_string(), bucket);
        if let Some(v) = self.images.get(&key) {
            return v.clone();
        }
        let out = (|| {
            let data = pres_media.get(media)?;
            let img = image::load_from_memory(data).ok()?;
            let rgba = img.to_rgba8();
            let (w, h) = rgba.dimensions();
            let rgba = if w > bucket {
                let nh = ((h as f64) * bucket as f64 / w as f64).round().max(1.0) as u32;
                image::imageops::resize(&rgba, bucket, nh, image::imageops::FilterType::Triangle)
            } else {
                rgba
            };
            let (w, h) = rgba.dimensions();
            let mut pm = Pixmap::new(w, h)?;
            for (dst, src) in pm.pixels_mut().iter_mut().zip(rgba.pixels()) {
                let [r, g, b, a] = src.0;
                *dst = tiny_skia::ColorU8::from_rgba(r, g, b, a).premultiply();
            }
            Some(Arc::new(pm))
        })();
        if out.is_none() {
            tracing::warn!("image `{media}` could not be decoded");
        }
        self.images.insert(key, out.clone());
        out
    }

    /// Draw an image clipped to `path`. Returns false when it could not be drawn.
    fn fill_image(&mut self, pm: &mut Pixmap, img: &ImageFill, path: &Path, x: &Xfrm, t: Transform) -> bool {
        let Some(media) = &img.media else { return false };
        let k = (t.sx * t.sx + t.kx * t.kx).sqrt();
        let (l, tp, r, b) = img.crop;
        let vis_w = (1.0 - (l + r) as f32 / 100_000.0).max(0.01);
        let want = (x.w as f32 * k / vis_w).ceil() as u32;
        let Some(src) = self.image_for(media, &self.media_ref(), want) else { return false };
        let (iw, ih) = (src.width() as f32, src.height() as f32);
        let (cl, ct) = (l as f32 / 100_000.0 * iw, tp as f32 / 100_000.0 * ih);
        let (cw, ch) = (iw * (1.0 - (l + r) as f32 / 100_000.0), ih * (1.0 - (tp + b) as f32 / 100_000.0));
        if cw <= 0.0 || ch <= 0.0 {
            return false;
        }
        let tr = Transform::from_scale(x.w as f32 / cw, x.h as f32 / ch).pre_translate(-cl, -ct);
        let pat = tiny_skia::Pattern::new(
            src.as_ref().as_ref(),
            SpreadMode::Pad,
            FilterQuality::Bilinear,
            (img.alpha as f32 / 100_000.0).clamp(0.0, 1.0),
            tr,
        );
        let mut p = Paint::default();
        p.shader = pat;
        p.anti_alias = true;
        pm.fill_path(path, &p, FillRule::Winding, t, None);
        true
    }

    fn media_ref(&self) -> Arc<MediaStore> {
        self.text.media.clone()
    }

    // ----------------------------------------------------------- tables --

    fn draw_table(&mut self, pm: &mut Pixmap, sc: &Scene, ctx: &Ctx, tbl: &Table, x: &Xfrm, t: Transform) {
        let col_x: Vec<i64> = std::iter::once(0).chain(tbl.cols.iter().scan(0, |a, w| { *a += w; Some(*a) })).collect();
        let row_y: Vec<i64> = std::iter::once(0).chain(tbl.rows.iter().scan(0, |a, r| { *a += r.height; Some(*a) })).collect();
        let dummy = Shape { id: 0, name: String::new(), descr: None, hidden: false, ph: None, xfrm: None, kind: ShapeKind::Opaque { fallback: vec![] }, source: None };
        let accent = Color::scheme("accent1");
        let styled = tbl.style_id.is_some() || tbl.first_row || tbl.band_row;
        for (ri, row) in tbl.rows.iter().enumerate() {
            let mut ci = 0usize;
            for cell in &row.cells {
                let this_c = ci;
                ci += 1;
                if cell.h_merge || cell.v_merge {
                    continue;
                }
                let c1 = (this_c + cell.grid_span as usize).min(tbl.cols.len());
                let r1 = (ri + cell.row_span as usize).min(tbl.rows.len());
                let (cx0, cx1) = (col_x.get(this_c).copied().unwrap_or(0), col_x.get(c1).copied().unwrap_or(0));
                let (cy0, cy1) = (row_y.get(ri).copied().unwrap_or(0), row_y.get(r1).copied().unwrap_or(0));
                let cell_x = Xfrm::new(cx0, cy0, cx1 - cx0, cy1 - cy0);
                let ct = t.pre_translate(cx0 as f32, cy0 as f32);
                let Some(rect) = geom::preset("rect", &[], cell_x.w as f32, cell_x.h as f32) else { continue };
                // Fill: explicit, or the default table style.
                let header = tbl.first_row && ri == 0;
                let mut extra = RunProps::default();
                if let Some(f) = &cell.fill {
                    self.fill_path(pm, ctx, &rect, f, None, &cell_x, ct);
                } else if styled {
                    if header {
                        self.fill_path(pm, ctx, &rect, &Fill::Solid(accent.clone()), None, &cell_x, ct);
                    } else {
                        let tint = if tbl.band_row && (ri - tbl.first_row as usize) % 2 == 0 { 40000 } else { 20000 };
                        let c = Color { base: kinaute_model::ColorBase::Scheme("accent1".into()), mods: vec![("tint".into(), tint)] };
                        self.fill_path(pm, ctx, &rect, &Fill::Solid(c), None, &cell_x, ct);
                    }
                }
                if header && cell.fill.is_none() {
                    extra.bold = Some(true);
                    extra.fill = Some(Fill::Solid(Color::scheme("lt1")));
                }
                // Borders.
                let white = Line { width: Some(12700), fill: Some(Fill::Solid(Color::scheme("lt1"))), ..Default::default() };
                let (w, h) = (cell_x.w as f32, cell_x.h as f32);
                let sides: [(Option<&Line>, [(f32, f32); 2]); 4] = [
                    (cell.borders[0].as_ref(), [(0.0, 0.0), (0.0, h)]),
                    (cell.borders[1].as_ref(), [(w, 0.0), (w, h)]),
                    (cell.borders[2].as_ref(), [(0.0, 0.0), (w, 0.0)]),
                    (cell.borders[3].as_ref(), [(0.0, h), (w, h)]),
                ];
                for (line, pts) in sides {
                    let line = match line {
                        Some(l) => l,
                        None if styled && cell.fill.is_none() => &white,
                        None => continue,
                    };
                    let mut pb = PathBuilder::new();
                    pb.move_to(pts[0].0, pts[0].1);
                    pb.line_to(pts[1].0, pts[1].1);
                    if let Some(p) = pb.finish() {
                        self.stroke_path(pm, ctx, &p, line, None, ct, false);
                    }
                }
                let mut tb = cell.text.clone();
                if tb.body.insets == [None; 4] {
                    tb.body.insets = cell.margins;
                }
                tb.body.anchor = cell.anchor.or(tb.body.anchor);
                self.draw_text(pm, sc, ctx, &dummy, &tb, &cell_x, ct, Some(extra));
            }
        }
        let _ = x;
    }

    fn draw_placeholder(&mut self, pm: &mut Pixmap, sc: &Scene, ctx: &Ctx, x: &Xfrm, t: Transform, label: &str) {
        let Some(rect) = geom::preset("rect", &[], x.w as f32, x.h as f32) else { return };
        let mut p = Paint::default();
        p.set_color_rgba8(0xF2, 0xF2, 0xF2, 0xFF);
        pm.fill_path(&rect, &p, FillRule::Winding, t, None);
        let line = Line { width: Some(9525), dash: Some("dash".into()), fill: Some(Fill::Solid(Color::srgb("A0A0A0"))), ..Default::default() };
        self.stroke_path(pm, ctx, &rect, &line, None, t, false);
        let mut tb = TextBody::default();
        tb.body.anchor = Some(Anchor::Middle);
        tb.paragraphs.push(Paragraph {
            props: ParaProps { align: Some(Align::Center), ..Default::default() },
            runs: vec![Run { props: RunProps { size: Some(1400), fill: Some(Fill::Solid(Color::srgb("707070"))), ..Default::default() }, ..Run::text(label) }],
            ..Default::default()
        });
        let dummy = Shape { id: 0, name: String::new(), descr: None, hidden: false, ph: None, xfrm: None, kind: ShapeKind::Opaque { fallback: vec![] }, source: None };
        self.draw_text(pm, sc, ctx, &dummy, &tb, x, t, None);
    }

    /// Give the renderer access to the presentation media (call before rendering).
    pub fn set_media(&mut self, media: Arc<MediaStore>) {
        if !Arc::ptr_eq(&self.text.media, &media) {
            self.text.media = media;
            self.images.clear();
        }
    }

    /// Drop cached images (e.g. after the zoom changes a lot).
    pub fn clear_image_cache(&mut self) {
        self.images.clear();
    }
}

/// Placement transform of a shape box inside its parent space: after it,
/// coordinates are local to the box (origin at its top-left, EMU units).
fn place(base: Transform, x: &Xfrm) -> Transform {
    let (cx, cy) = (x.x as f32 + x.w as f32 / 2.0, x.y as f32 + x.h as f32 / 2.0);
    let mut t = base.pre_translate(cx, cy);
    if x.rot != 0 {
        t = t.pre_rotate(x.rot as f32 / 60_000.0);
    }
    if x.flip_h || x.flip_v {
        t = t.pre_scale(if x.flip_h { -1.0 } else { 1.0 }, if x.flip_v { -1.0 } else { 1.0 });
    }
    t.pre_translate(-(x.w as f32) / 2.0, -(x.h as f32) / 2.0)
}

fn pattern_coverage(p: &str) -> f32 {
    match p {
        "pct5" => 0.05,
        "pct10" => 0.1,
        "pct20" => 0.2,
        "pct25" => 0.25,
        "pct30" => 0.3,
        "pct40" => 0.4,
        "pct50" => 0.5,
        "pct60" => 0.6,
        "pct70" => 0.7,
        "pct75" => 0.75,
        "pct80" => 0.8,
        "pct90" => 0.9,
        "solidDmnd" => 1.0,
        _ => 0.35,
    }
}

/// In-place box blur approximating a Gaussian (`passes` iterations).
fn box_blur(pm: &mut Pixmap, radius: usize, passes: usize) {
    if radius == 0 {
        return;
    }
    let (w, h) = (pm.width() as usize, pm.height() as usize);
    let data = pm.data_mut();
    let mut tmp = vec![0u8; data.len()];
    for _ in 0..passes {
        blur_axis(data, &mut tmp, w, h, radius, true);
        blur_axis(&tmp, data, w, h, radius, false);
    }
}

fn blur_axis(src: &[u8], dst: &mut [u8], w: usize, h: usize, r: usize, horizontal: bool) {
    let (n, lines) = if horizontal { (w, h) } else { (h, w) };
    let win = (2 * r + 1) as u32;
    for line in 0..lines {
        let idx = |i: usize| -> usize { 4 * if horizontal { line * w + i } else { i * w + line } };
        let mut acc = [0u32; 4];
        for i in 0..=r.min(n - 1) {
            for c in 0..4 {
                acc[c] += src[idx(i) + c] as u32;
            }
        }
        for i in 0..n {
            for c in 0..4 {
                dst[idx(i) + c] = (acc[c] / win) as u8;
            }
            let add = i + r + 1;
            if add < n {
                for c in 0..4 {
                    acc[c] += src[idx(add) + c] as u32;
                }
            }
            if i >= r {
                for c in 0..4 {
                    acc[c] -= src[idx(i - r) + c] as u32;
                }
            }
        }
    }
}

/// Effective top-level shapes with their bounds for hit testing and editing.
pub mod query {
    use super::*;

    /// Effective transform of a top-level shape (including inherited ones).
    pub fn shape_xfrm(pres: &Presentation, slide: usize, id: u32) -> Option<Xfrm> {
        let s = pres.slides.get(slide)?;
        let ctx = Ctx::for_slide(pres, s);
        let sh = s.shapes.iter().find(|x| x.id == id)?;
        ctx.xfrm(sh)
    }

    /// Topmost top-level shape under a point (slide EMU coordinates).
    pub fn hit_test(pres: &Presentation, slide: usize, px: f64, py: f64) -> Option<u32> {
        let s = pres.slides.get(slide)?;
        let ctx = Ctx::for_slide(pres, s);
        for sh in s.shapes.iter().rev() {
            if sh.hidden {
                continue;
            }
            let Some(x) = ctx.xfrm(sh) else { continue };
            let (cx, cy) = x.center();
            let a = -x.rot_deg().to_radians();
            let (dx, dy) = (px - cx, py - cy);
            let (lx, ly) = (dx * a.cos() - dy * a.sin(), dx * a.sin() + dy * a.cos());
            let tol = 6350.0; // half a point, so thin lines can be picked
            if lx.abs() <= x.w as f64 / 2.0 + tol && ly.abs() <= x.h as f64 / 2.0 + tol {
                return Some(sh.id);
            }
        }
        None
    }
}

pub use query::{hit_test, shape_xfrm};

/// Convert a pixmap to PNG bytes.
pub fn to_png(pm: &Pixmap) -> Option<Vec<u8>> {
    pm.encode_png().ok()
}

#[allow(dead_code)]
const _: (f32, Level, BlendMode) = (EMU_PT, Level::Slide, BlendMode::SourceOver);
