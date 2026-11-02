//! Text layout and drawing.

use super::{Renderer, Scene};
use cosmic_text::{Align as CAlign, Attrs, AttrsOwned, Buffer, Color as CColor, Family, FontSystem, Metrics, Shaping, Style, SwashCache, Weight, Wrap};
use kinaute_model::resolve::{Ctx, DEFAULT_FONT_SIZE, DEFAULT_INSETS};
use kinaute_model::*;
use std::collections::HashMap;
use std::sync::Arc;
use tiny_skia::{Pixmap, PixmapPaint, Transform};

const EMU_PT: f32 = 12700.0;
/// Line height as a multiple of the font size for "single" spacing.
const LINE_FACTOR: f32 = 1.2;

pub struct TextEngine {
    pub fonts: FontSystem,
    pub swash: SwashCache,
    pub media: Arc<MediaStore>,
    families: HashMap<String, FamilyChoice>,
    pub warned: std::collections::BTreeSet<String>,
}

#[derive(Clone)]
enum FamilyChoice {
    Name(String),
    Serif,
    Sans,
    Mono,
}

/// Measured size of laid out text (in EMU).
#[derive(Debug, Clone, Copy, Default)]
pub struct TextMetrics {
    pub width: f32,
    pub height: f32,
}

impl TextEngine {
    pub fn new() -> Self {
        TextEngine {
            fonts: FontSystem::new(),
            swash: SwashCache::new(),
            media: Arc::new(MediaStore::new()),
            families: HashMap::new(),
            warned: Default::default(),
        }
    }

    fn has_family(&self, name: &str) -> bool {
        self.fonts.db().faces().any(|f| f.families.iter().any(|(n, _)| n.eq_ignore_ascii_case(name)))
    }

    /// Map a requested font name to an installed family (metric-compatible
    /// substitutes first), warning once when the font is missing.
    fn family(&mut self, name: &str) -> FamilyChoice {
        if let Some(c) = self.families.get(name) {
            return c.clone();
        }
        let lower = name.to_ascii_lowercase();
        let alias: &[&str] = match lower.as_str() {
            "calibri" | "calibri light" | "segoe ui" | "segoe ui light" => &["Carlito", "Inter", "DejaVu Sans"],
            "cambria" | "cambria math" => &["Caladea", "DejaVu Serif"],
            "arial" | "helvetica" | "helvetica neue" | "arial unicode ms" => &["Liberation Sans", "Arimo", "FreeSans", "DejaVu Sans"],
            "arial narrow" => &["Liberation Sans Narrow", "Liberation Sans"],
            "times new roman" | "times" => &["Liberation Serif", "Tinos", "FreeSerif", "DejaVu Serif"],
            "courier new" | "courier" => &["Liberation Mono", "Cousine", "FreeMono", "DejaVu Sans Mono"],
            "consolas" | "lucida console" | "menlo" | "monaco" => &["JetBrains Mono", "DejaVu Sans Mono"],
            "verdana" | "tahoma" | "trebuchet ms" | "gill sans mt" | "century gothic" | "franklin gothic book" => &["DejaVu Sans", "Inter"],
            "georgia" | "palatino linotype" | "book antiqua" | "garamond" | "constantia" => &["DejaVu Serif", "Caladea"],
            _ => &[],
        };
        let mut choice = None;
        if self.has_family(name) {
            choice = Some(FamilyChoice::Name(name.to_string()));
        } else {
            for a in alias {
                if self.has_family(a) {
                    choice = Some(FamilyChoice::Name(a.to_string()));
                    break;
                }
            }
        }
        let choice = choice.unwrap_or_else(|| {
            self.warned.insert(name.to_string());
            if ["serif", "times", "georgia", "cambria", "garamond", "palatino", "book", "roman"].iter().any(|k| lower.contains(k)) && !lower.contains("sans") {
                FamilyChoice::Serif
            } else if ["mono", "courier", "consolas", "code"].iter().any(|k| lower.contains(k)) {
                FamilyChoice::Mono
            } else {
                FamilyChoice::Sans
            }
        });
        self.families.insert(name.to_string(), choice.clone());
        choice
    }
}

impl Default for TextEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// One pre-processed run, ready for the shaper.
struct Span {
    text: String,
    attrs: AttrsOwned,
    size_px: f32,
    line_px: f32,
}

fn bullet_text(b: &Bullet, number: i32) -> Option<String> {
    match b {
        Bullet::None => None,
        Bullet::Char { ch, font, .. } => {
            let sym = matches!(font.as_deref(), Some(f) if f.eq_ignore_ascii_case("wingdings") || f.eq_ignore_ascii_case("symbol") || f.eq_ignore_ascii_case("wingdings 2") || f.eq_ignore_ascii_case("wingdings 3"));
            let c = if sym {
                match ch.as_str() {
                    "§" => "▪",
                    "Ø" => "➢",
                    "ü" => "✓",
                    "l" => "●",
                    "n" => "■",
                    "q" => "❑",
                    "v" => "❖",
                    "·" => "•",
                    "o" => "○",
                    other => other,
                }
            } else {
                ch.as_str()
            };
            Some(c.to_string())
        }
        Bullet::AutoNum { scheme, .. } => {
            let n = number.max(1);
            let alpha = |n: i32, upper: bool| {
                let mut s = String::new();
                let mut v = n - 1;
                loop {
                    let c = (b'a' + (v % 26) as u8) as char;
                    s.insert(0, if upper { c.to_ascii_uppercase() } else { c });
                    v = v / 26 - 1;
                    if v < 0 {
                        break;
                    }
                }
                s
            };
            let roman = |n: i32, upper: bool| {
                let table = [(1000, "m"), (900, "cm"), (500, "d"), (400, "cd"), (100, "c"), (90, "xc"), (50, "l"), (40, "xl"), (10, "x"), (9, "ix"), (5, "v"), (4, "iv"), (1, "i")];
                let mut s = String::new();
                let mut v = n.min(3999);
                for (val, sym) in table {
                    while v >= val {
                        s.push_str(sym);
                        v -= val;
                    }
                }
                if upper { s.to_uppercase() } else { s }
            };
            let (body, suffix) = match scheme.as_str() {
                s if s.starts_with("alphaLc") => (alpha(n, false), s.strip_prefix("alphaLc")),
                s if s.starts_with("alphaUc") => (alpha(n, true), s.strip_prefix("alphaUc")),
                s if s.starts_with("romanLc") => (roman(n, false), s.strip_prefix("romanLc")),
                s if s.starts_with("romanUc") => (roman(n, true), s.strip_prefix("romanUc")),
                s => (n.to_string(), s.strip_prefix("arabic")),
            };
            let suffix = match suffix.unwrap_or("Period") {
                "ParenR" => ")",
                "ParenBoth" => return Some(format!("({body})")),
                "Plain" => "",
                _ => ".",
            };
            Some(format!("{body}{suffix}"))
        }
    }
}

impl Renderer {
    /// Build shaper spans for a run.
    fn run_span(&mut self, ctx: &Ctx, sc: &Scene, pp: &ParaProps, run: &Run, scale: f32, k: f32, ink: tiny_skia::Color) -> Option<Span> {
        let rp = pp_run(ctx, pp, &run.props);
        let mut size_pt = rp.size.unwrap_or(DEFAULT_FONT_SIZE) as f32 / 100.0 * scale;
        if let Some(b) = rp.baseline.filter(|b| *b != 0) {
            let _ = b;
            size_pt *= 0.65;
        }
        let size_px = (size_pt * EMU_PT * k).clamp(1.0, 2000.0);
        if !size_px.is_finite() {
            return None;
        }
        let mut text = match &run.kind {
            RunKind::Text(t) => t.clone(),
            RunKind::Break => "\n".to_string(),
            RunKind::Field { kind, text } => {
                if kind.starts_with("slidenum") {
                    sc.slide_no.to_string()
                } else if kind == "slidecount" {
                    sc.slide_count.to_string()
                } else {
                    text.clone()
                }
            }
        };
        match rp.caps.as_deref() {
            Some("all") | Some("small") => text = text.to_uppercase(),
            _ => {}
        }
        if text.is_empty() {
            return None;
        }
        let color = match &rp.fill {
            Some(Fill::Solid(c)) => {
                let [r, g, b, a] = ctx.color(c).to_u8();
                CColor::rgba(r, g, b, a)
            }
            Some(Fill::Gradient(g)) => g.stops.first().map(|(_, c)| {
                let [r, g, b, a] = ctx.color(c).to_u8();
                CColor::rgba(r, g, b, a)
            }).unwrap_or(CColor::rgba(0, 0, 0, 255)),
            _ => {
                let c = ink.to_color_u8();
                CColor::rgba(c.red(), c.green(), c.blue(), c.alpha())
            }
        };
        let font = ctx.font_name(rp.font.as_deref().unwrap_or("+mn-lt"));
        let fam = self.text.family(if font.is_empty() { "Calibri" } else { &font });
        let line_h = line_height(&pp.line_spacing, size_px, k);
        let mut attrs = Attrs::new().color(color).metrics(Metrics::new(size_px, line_h));
        if rp.bold.unwrap_or(false) {
            attrs = attrs.weight(Weight::BOLD);
        }
        if rp.italic.unwrap_or(false) {
            attrs = attrs.style(Style::Italic);
        }
        let owned = match &fam {
            FamilyChoice::Name(n) => AttrsOwned::new(&attrs.family(Family::Name(n))),
            FamilyChoice::Serif => AttrsOwned::new(&attrs.family(Family::Serif)),
            FamilyChoice::Sans => AttrsOwned::new(&attrs.family(Family::SansSerif)),
            FamilyChoice::Mono => AttrsOwned::new(&attrs.family(Family::Monospace)),
        };
        Some(Span { text, attrs: owned, size_px, line_px: line_h })
    }

    /// Lay out and draw a text body inside the box of `x` (local EMU space
    /// transformed by `t`). `extra` supplies lowest-priority run defaults
    /// (table headers).
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn draw_text(&mut self, pm: &mut Pixmap, sc: &Scene, ctx: &Ctx, shape: &Shape, body: &TextBody, x: &Xfrm, t: Transform, extra: Option<RunProps>) {
        if body.paragraphs.is_empty() || body.is_empty() && body.paragraphs.len() <= 1 {
            return;
        }
        // `k`: device pixels per EMU for fonts, insets and spacing (the slide's
        // scale). `kw`/`kh`: pixels per local EMU of the box (group scaling).
        let k = sc.k;
        let kw = (t.sx * t.sx + t.kx * t.kx).sqrt();
        let kh = (t.ky * t.ky + t.sy * t.sy).sqrt();
        if !(k > 0.0 && kw > 0.0 && kh > 0.0 && k.is_finite() && kw.is_finite() && kh.is_finite()) {
            return;
        }
        let bp = ctx.body_props(shape);
        let bp = {
            let mut b = body.body.clone();
            b.inherit(&bp);
            b
        };
        let ins = |i: usize| bp.insets[i].unwrap_or(DEFAULT_INSETS[i]) as f32;
        let (il, it, ir, ib) = (ins(0), ins(1), ins(2), ins(3));
        // Inner box in device pixels.
        let inner_w_px = (x.w as f32 * kw - (il + ir) * k).max(1.0);
        let inner_h_px = (x.h as f32 * kh - (it + ib) * k).max(1.0);
        let wrap = bp.wrap.unwrap_or(true);
        let (scale, line_reduce) = match bp.autofit {
            Some(AutoFit::Normal { font_scale, line_reduction }) => (font_scale as f32 / 100_000.0, line_reduction as f32 / 100_000.0),
            _ => (1.0, 0.0),
        };
        let ink = tiny_skia::Color::BLACK;

        // Pass 1: shape every paragraph.
        struct Para {
            buf: Buffer,
            height: f32,
            x_off: f32,
            before: f32,
            after: f32,
            bullet: Option<(Buffer, f32)>,
        }
        let mut paras: Vec<Para> = Vec::new();
        let mut counters: HashMap<u8, i32> = HashMap::new();
        for para in &body.paragraphs {
            let mut pp = ctx.para_props(shape, body, para);
            if let Some(e) = &extra {
                let mut e = e.clone();
                e.inherit(&pp.def_run);
                pp.def_run = e;
            }
            if line_reduce > 0.0 {
                if let Some(Spacing::Percent(p)) = pp.line_spacing {
                    pp.line_spacing = Some(Spacing::Percent(((p as f32) * (1.0 - line_reduce)) as i32));
                }
            }
            let mut spans: Vec<Span> = Vec::new();
            for r in &para.runs {
                if let Some(s) = self.run_span(ctx, sc, &pp, r, scale, k, ink) {
                    spans.push(s);
                }
            }
            // Size of an empty paragraph comes from its end properties.
            let first_size = spans
                .first()
                .map(|s| s.size_px)
                .unwrap_or_else(|| {
                    let rp = para.end_props.as_ref().map(|e| pp_run(ctx, &pp, e)).unwrap_or_else(|| pp_run(ctx, &pp, &RunProps::default()));
                    (rp.size.unwrap_or(DEFAULT_FONT_SIZE) as f32 / 100.0 * scale * EMU_PT * k).clamp(1.0, 2000.0)
                });
            let lh = line_height(&pp.line_spacing, first_size, k);
            let mar_l = pp.mar_l.unwrap_or(0) as f32;
            let indent = pp.indent.unwrap_or(0) as f32;
            let has_bullet = matches!(&pp.bullet, Some(b) if !matches!(b, Bullet::None)) && !spans.is_empty();
            // Auto-numbering.
            if let Some(Bullet::AutoNum { start, .. }) = &pp.bullet {
                let c = counters.entry(para.level).or_insert(*start - 1);
                *c += 1;
                counters.retain(|l, _| *l <= para.level);
            } else {
                counters.retain(|l, _| *l < para.level);
            }
            let text_x = if has_bullet { mar_l } else { mar_l + indent.max(0.0) };
            let avail = if wrap { Some((inner_w_px - text_x * k).max(1.0)) } else { None };

            let default = Attrs::new().metrics(Metrics::new(first_size.max(1.0), lh.max(1.0)));
            let mut buf = Buffer::new_empty(Metrics::new(first_size.max(1.0), lh.max(1.0)));
            buf.set_wrap(if wrap { Wrap::WordOrGlyph } else { Wrap::None });
            buf.set_size(avail, None);
            let align = match pp.align.unwrap_or(Align::Left) {
                Align::Left => if pp.rtl == Some(true) { CAlign::Right } else { CAlign::Left },
                Align::Center => CAlign::Center,
                Align::Right => CAlign::Right,
                Align::Justify => CAlign::Justified,
            };
            let rich: Vec<(&str, Attrs)> = spans.iter().map(|s| (s.text.as_str(), s.attrs.as_attrs())).collect();
            if rich.is_empty() {
                buf.set_text("", &default, Shaping::Advanced, Some(align));
            } else {
                buf.set_rich_text(rich, &default, Shaping::Advanced, Some(align));
            }
            buf.shape_until_scroll(&mut self.text.fonts, false);
            let lines = buf.layout_runs().count().max(1) as f32;
            let mut height = buf.layout_runs().last().map(|r| r.line_top + r.line_height).unwrap_or(lh * lines);
            if spans.is_empty() {
                height = lh;
            }
            // Bullet.
            let bullet = if has_bullet {
                let num = counters.get(&para.level).copied().unwrap_or(1);
                pp.bullet.as_ref().and_then(|b| bullet_text(b, num)).and_then(|txt| {
                    let first = spans.first()?;
                    let (color, size_pct) = match &pp.bullet {
                        Some(Bullet::Char { color, size_pct, .. }) | Some(Bullet::AutoNum { color, size_pct, .. }) => (color.clone(), *size_pct),
                        _ => (None, None),
                    };
                    let mut a = first.attrs.clone();
                    if let Some(c) = color {
                        let [r, g, b, al] = ctx.color(&c).to_u8();
                        a.color_opt = Some(CColor::rgba(r, g, b, al));
                    }
                    if let Some(p) = size_pct {
                        let fs = first.size_px * p as f32 / 100_000.0;
                        a.metrics_opt = Some(Metrics::new(fs.max(1.0), first.line_px).into());
                    }
                    let mut bb = Buffer::new_empty(Metrics::new(first_size.max(1.0), lh.max(1.0)));
                    bb.set_wrap(Wrap::None);
                    bb.set_size(None, None);
                    bb.set_rich_text([(txt.as_str(), a.as_attrs())], &default, Shaping::Advanced, None);
                    bb.shape_until_scroll(&mut self.text.fonts, false);
                    Some((bb, (mar_l + indent).max(0.0)))
                })
            } else {
                None
            };
            let spc = |s: &Option<Spacing>| -> f32 {
                match s {
                    Some(Spacing::Points(p)) => *p as f32 / 100.0 * EMU_PT * k,
                    Some(Spacing::Percent(p)) => first_size * 1.2 * *p as f32 / 100_000.0,
                    None => 0.0,
                }
            };
            paras.push(Para { buf, height, x_off: text_x * k, before: spc(&pp.space_before), after: spc(&pp.space_after), bullet });
        }

        // Pass 2: vertical placement.
        let total: f32 = paras.iter().enumerate().map(|(i, p)| p.height + if i > 0 { p.before } else { 0.0 } + p.after).sum::<f32>() - paras.last().map(|p| p.after).unwrap_or(0.0);
        let box_h_px = inner_h_px;
        let mut y = match bp.anchor.unwrap_or(Anchor::Top) {
            Anchor::Top => 0.0,
            Anchor::Middle => (box_h_px - total) / 2.0,
            Anchor::Bottom => box_h_px - total,
        };
        let _ = &mut y;

        // Target: direct into the slide pixmap when unrotated, else a temp.
        let direct = t.kx == 0.0 && t.ky == 0.0 && t.sx > 0.0 && t.sy > 0.0;
        let box_w_px = (x.w as f32 * kw).ceil().max(1.0) as u32;
        let box_h_full = (x.h as f32 * kh).ceil().max(1.0) as u32;
        let mut tmp: Option<Pixmap> = None;
        if !direct {
            if box_w_px > 8192 || box_h_full > 8192 {
                return;
            }
            tmp = Pixmap::new(box_w_px, box_h_full);
            if tmp.is_none() {
                return;
            }
        }
        let (target, ox, oy): (&mut Pixmap, f32, f32) = match tmp.as_mut() {
            Some(p) => (p, il * k, it * k),
            None => (&mut *pm, t.tx + il * k, t.ty + it * k),
        };
        let (tw, th) = (target.width() as i32, target.height() as i32);
        let mut cy = y;
        for (i, p) in paras.iter_mut().enumerate() {
            if i > 0 {
                cy += p.before;
            }
            let (bx, by) = (ox + p.x_off, oy + cy);
            if let Some((bb, bxo)) = p.bullet.as_mut() {
                let (bxx, byy) = (ox + *bxo * k, by);
                blit_buffer(&mut self.text.fonts, &mut self.text.swash, bb, target, bxx, byy, tw, th);
            }
            blit_buffer(&mut self.text.fonts, &mut self.text.swash, &mut p.buf, target, bx, by, tw, th);
            cy += p.height + p.after;
        }
        if let Some(tmp) = &tmp {
            let paint = PixmapPaint { quality: tiny_skia::FilterQuality::Bilinear, ..Default::default() };
            pm.draw_pixmap(0, 0, tmp.as_ref(), &paint, t.pre_scale(1.0 / kw, 1.0 / kh), None);
        }
    }

    /// Measure the height a text body needs for a given width (EMU).
    pub fn measure_text(&mut self, pres: &Presentation, slide: usize, shape_id: u32) -> Option<TextMetrics> {
        let s = pres.slides.get(slide)?;
        let sh = s.shapes.iter().find(|x| x.id == shape_id)?;
        let body = sh.text()?;
        let ctx = Ctx::for_slide(pres, s);
        let x = ctx.xfrm(sh)?;
        let mut h = 0.0f32;
        for para in &body.paragraphs {
            let pp = ctx.para_props(sh, body, para);
            let size = pp_run(&ctx, &pp, &para.runs.first().map(|r| r.props.clone()).unwrap_or_default()).size.unwrap_or(DEFAULT_FONT_SIZE) as f32 / 100.0 * EMU_PT;
            h += line_height(&pp.line_spacing, size, 1.0);
        }
        Some(TextMetrics { width: x.w as f32, height: h })
    }
}

fn pp_run(ctx: &Ctx, pp: &ParaProps, run: &RunProps) -> RunProps {
    let _ = ctx;
    let mut r = run.clone();
    r.inherit(&pp.def_run);
    r
}

fn line_height(spacing: &Option<Spacing>, size_px: f32, k: f32) -> f32 {
    match spacing {
        Some(Spacing::Percent(p)) => size_px * LINE_FACTOR * *p as f32 / 100_000.0,
        Some(Spacing::Points(p)) => *p as f32 / 100.0 * EMU_PT * k,
        None => size_px * LINE_FACTOR,
    }
}

/// Draw a shaped buffer into a pixmap at (x, y) with source-over blending.
#[allow(clippy::too_many_arguments)]
fn blit_buffer(fonts: &mut FontSystem, swash: &mut SwashCache, buf: &mut Buffer, pm: &mut Pixmap, x: f32, y: f32, w: i32, h: i32) {
    let (xi, yi) = (x.round() as i32, y.round() as i32);
    let data = pm.data_mut();
    buf.draw(fonts, swash, CColor::rgba(0, 0, 0, 255), |gx, gy, gw, gh, c| {
        let a = c.a() as u32;
        if a == 0 {
            return;
        }
        for yy in gy..gy + gh as i32 {
            let py = yy + yi;
            if py < 0 || py >= h {
                continue;
            }
            for xx in gx..gx + gw as i32 {
                let px = xx + xi;
                if px < 0 || px >= w {
                    continue;
                }
                let i = ((py * w + px) * 4) as usize;
                // Premultiplied source-over.
                let inv = 255 - a;
                let (sr, sg, sb) = (c.r() as u32 * a / 255, c.g() as u32 * a / 255, c.b() as u32 * a / 255);
                data[i] = (sr + data[i] as u32 * inv / 255).min(255) as u8;
                data[i + 1] = (sg + data[i + 1] as u32 * inv / 255).min(255) as u8;
                data[i + 2] = (sb + data[i + 2] as u32 * inv / 255).min(255) as u8;
                data[i + 3] = (a + data[i + 3] as u32 * inv / 255).min(255) as u8;
            }
        }
    });
}
