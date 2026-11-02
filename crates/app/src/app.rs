//! Editor logic, independent from the UI toolkit (so it can be tested without
//! a window): view transform, selection, dragging, snapping, commands.

use kinaute_model::edit::{bounds, RunPatch, ZOrder};
use kinaute_model::resolve::Ctx;
use kinaute_model::*;
use kinaute_pptx::Doc;
use kinaute_render::Renderer;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use tiny_skia::{FillRule, Paint, PathBuilder, Pixmap, PixmapPaint, Stroke, Transform};

pub const EMU_CM: f64 = 360_000.0;
pub const EMU_PT: f64 = 12_700.0;
const HANDLE_PX: f32 = 9.0;
const ROT_OFFSET_PX: f32 = 26.0;
const SNAP_PX: f64 = 6.0;
const ACCENT: (u8, u8, u8) = (0x2D, 0x7F, 0xF9);
const GUIDE: (u8, u8, u8) = (0xFF, 0xB8, 0x00);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    /// Resize handle with direction (-1, 0, 1) per axis.
    Resize(i8, i8),
    Rotate,
}

#[derive(Debug, Clone)]
enum Drag {
    Move { start: (f64, f64), orig: Vec<(u32, Xfrm)>, moved: bool },
    Resize { id: u32, hx: i8, hy: i8, orig: Xfrm, keep_ratio: bool },
    Rotate { id: u32, orig: Xfrm },
    Marquee { start: (f64, f64), cur: (f64, f64), add: bool },
    Create { start: (f64, f64), cur: (f64, f64) },
    Pan { last: (f32, f32) },
}

/// Static layers cached while objects are being dragged: everything under
/// and above the moving objects, rendered once per drag.
struct Layers {
    key: (u64, usize, u32),
    below: Pixmap,
    above: Pixmap,
}

/// Cached render of the current slide.
struct SlideCache {
    revision: u64,
    slide: usize,
    width: u32,
    pm: Pixmap,
}

/// Everything the inspector needs to know about the selection.
#[derive(Debug, Clone, Default)]
pub struct SelInfo {
    pub count: usize,
    pub has_text: bool,
    pub text: String,
    pub size_pt: f32,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub h_align: i32,
    pub v_align: i32,
    pub fill: Option<[u8; 4]>,
    pub line: Option<([u8; 4], f64)>,
    pub xfrm: Option<Xfrm>,
    pub is_group: bool,
}

pub struct App {
    pub doc: Doc,
    pub path: Option<PathBuf>,
    pub renderer: Rc<RefCell<Renderer>>,
    pub cur: usize,
    pub sel: Vec<u32>,
    pub tool: String,
    pub status: String,
    pub dark: bool,
    // view
    pub canvas: (u32, u32),
    pub zoom: f32,
    pan: (f32, f32),
    drag: Option<Drag>,
    guides: Vec<(bool, f64)>,
    cache: Option<SlideCache>,
    layers: Option<Layers>,
    clipboard: Vec<Shape>,
    drag_key: u64,
    pub cursor_handle: Option<Handle>,
}

/// Axis-aligned bounds (left, top, right, bottom).
type Bounds = (f64, f64, f64, f64);

/// Copy an opaque pixmap into `dst` at (ox, oy), row by row (no blending).
fn blit_opaque(dst: &mut Pixmap, src: &Pixmap, ox: i32, oy: i32) {
    let (dw, dh) = (dst.width() as i32, dst.height() as i32);
    let (sw, sh) = (src.width() as i32, src.height() as i32);
    let (x0, x1) = (ox.max(0), (ox + sw).min(dw));
    let (y0, y1) = (oy.max(0), (oy + sh).min(dh));
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let n = ((x1 - x0) * 4) as usize;
    let sdata = src.data();
    let ddata = dst.data_mut();
    for y in y0..y1 {
        let si = (((y - oy) * sw + (x0 - ox)) * 4) as usize;
        let di = ((y * dw + x0) * 4) as usize;
        ddata[di..di + n].copy_from_slice(&sdata[si..si + n]);
    }
}

fn rot_pt(x: f64, y: f64, a: f64) -> (f64, f64) {
    (x * a.cos() - y * a.sin(), x * a.sin() + y * a.cos())
}

impl App {
    pub fn new(doc: Doc, path: Option<PathBuf>, renderer: Rc<RefCell<Renderer>>) -> Self {
        renderer.borrow_mut().set_media(doc.pres().media.clone());
        let status = if doc.report.is_empty() {
            String::new()
        } else {
            format!("{} remarque(s) à l'ouverture", doc.report.len())
        };
        App {
            doc,
            path,
            renderer,
            cur: 0,
            sel: Vec::new(),
            tool: "select".into(),
            status,
            dark: true,
            canvas: (800, 600),
            zoom: 1.0,
            pan: (0.0, 0.0),
            drag: None,
            guides: Vec::new(),
            cache: None,
            layers: None,
            clipboard: Vec::new(),
            drag_key: 0,
            cursor_handle: None,
        }
    }

    pub fn pres(&self) -> &Presentation {
        self.doc.history.current()
    }

    pub fn slide_count(&self) -> usize {
        self.pres().slides.len()
    }

    pub fn dirty(&self) -> bool {
        self.doc.history.is_dirty()
    }

    // ------------------------------------------------------------- view --

    /// (origin x, origin y, pixels per EMU).
    pub fn view(&self) -> (f32, f32, f32) {
        let p = self.pres();
        let (cw, ch) = (self.canvas.0 as f32, self.canvas.1 as f32);
        let margin = 28.0;
        let fit = ((cw - 2.0 * margin) / p.slide_w as f32).min((ch - 2.0 * margin) / p.slide_h as f32).max(1e-9);
        let k = fit * self.zoom;
        let (sw, sh) = (p.slide_w as f32 * k, p.slide_h as f32 * k);
        (((cw - sw) / 2.0 + self.pan.0).round(), ((ch - sh) / 2.0 + self.pan.1).round(), k)
    }

    pub fn to_emu(&self, x: f32, y: f32) -> (f64, f64) {
        let (ox, oy, k) = self.view();
        (((x - ox) / k) as f64, ((y - oy) / k) as f64)
    }

    pub fn set_canvas(&mut self, w: u32, h: u32) {
        self.canvas = (w.max(64), h.max(64));
    }

    pub fn zoom_percent(&self) -> i32 {
        let p = self.pres();
        let (cw, ch) = (self.canvas.0 as f32, self.canvas.1 as f32);
        let fit = ((cw - 56.0) / p.slide_w as f32).min((ch - 56.0) / p.slide_h as f32);
        // 100 % = real size at 96 dpi.
        let real = 96.0 / 914_400.0;
        (fit * self.zoom / real * 100.0).round() as i32
    }

    pub fn zoom_fit(&mut self) {
        self.zoom = 1.0;
        self.pan = (0.0, 0.0);
    }

    pub fn zoom_by(&mut self, factor: f32, around: Option<(f32, f32)>) {
        let before = around.map(|(x, y)| self.to_emu(x, y));
        self.zoom = (self.zoom * factor).clamp(0.25, 3.0);
        if self.zoom <= 1.0 {
            self.pan = (0.0, 0.0);
        } else if let (Some((px, py)), Some((ex, ey))) = (around, before) {
            // Keep the point under the cursor fixed.
            let (ox, oy, k) = self.view();
            self.pan.0 += px - (ox + ex as f32 * k);
            self.pan.1 += py - (oy + ey as f32 * k);
        }
    }

    // ----------------------------------------------------------- render --

    /// Render the whole canvas (slide + selection overlay).
    pub fn render_canvas(&mut self) -> Option<Pixmap> {
        let (cw, ch) = self.canvas;
        let mut out = Pixmap::new(cw, ch)?;
        let bg = if self.dark { (0x2B, 0x2B, 0x2E) } else { (0xDD, 0xDF, 0xE3) };
        out.fill(tiny_skia::Color::from_rgba8(bg.0, bg.1, bg.2, 255));
        let (ox, oy, k) = self.view();
        let (sw, sh) = (self.pres().slide_w as f32 * k, self.pres().slide_h as f32 * k);
        let width = sw.round().max(16.0) as u32;
        let rev = self.doc.history.revision();
        // While dragging, only the moving objects are redrawn.
        let layered = match &self.drag {
            Some(Drag::Move { moved: true, .. }) | Some(Drag::Resize { .. }) | Some(Drag::Rotate { .. })
                if !self.sel.is_empty() =>
            {
                let ids = self.sel.clone();
                self.render_layered(width, &ids)
            }
            _ => {
                self.layers = None;
                None
            }
        };
        let stale = layered.is_none()
            && !matches!(&self.cache, Some(c) if c.revision == rev && c.slide == self.cur && c.width == width);
        if stale {
            let pres = self.doc.history.current().clone();
            self.renderer.borrow_mut().show_prompts = true;
            let pm = self.renderer.borrow_mut().render_slide(&pres, self.cur, width);
            self.renderer.borrow_mut().show_prompts = false;
            self.cache = pm.map(|pm| SlideCache { revision: rev, slide: self.cur, width, pm });
        }
        // Soft drop shadow.
        let mut p = Paint::default();
        p.set_color_rgba8(0, 0, 0, 70);
        for i in 1..=4 {
            if let Some(r) = tiny_skia::Rect::from_xywh(
                ox - i as f32 + 1.0,
                oy + i as f32 * 1.5,
                sw + (i * 2) as f32 - 2.0,
                sh + 1.0,
            ) {
                let mut pb = PathBuilder::new();
                pb.push_rect(r);
                if let Some(path) = pb.finish() {
                    let mut q = Paint::default();
                    q.set_color_rgba8(0, 0, 0, (28 / i) as u8 + 6);
                    out.fill_path(&path, &q, FillRule::Winding, Transform::identity(), None);
                }
            }
        }
        if let Some(l) = &layered {
            blit_opaque(&mut out, l, ox as i32, oy as i32);
        } else if let Some(c) = &self.cache {
            blit_opaque(&mut out, &c.pm, ox as i32, oy as i32);
        }
        self.draw_overlay(&mut out);
        Some(out)
    }

    /// Compose below + moving + above for the current slide.
    fn render_layered(&mut self, width: u32, ids: &[u32]) -> Option<Pixmap> {
        let key = (self.drag_key, self.cur, width);
        let pres = self.doc.history.current().clone();
        let mut r = self.renderer.borrow_mut();
        r.show_prompts = true;
        if self.layers.as_ref().map(|l| l.key) != Some(key) {
            let below = r.render_pass(&pres, self.cur, width, kinaute_render::Pass::Below(ids));
            let above = r.render_pass(&pres, self.cur, width, kinaute_render::Pass::Above(ids));
            self.layers = match (below, above) {
                (Some(below), Some(above)) => Some(Layers { key, below, above }),
                _ => None,
            };
        }
        let moving = r.render_pass(&pres, self.cur, width, kinaute_render::Pass::Only(ids));
        r.show_prompts = false;
        drop(r);
        let l = self.layers.as_ref()?;
        let mut out = l.below.clone();
        // Only the area around the moving objects needs compositing.
        let k = width as f64 / self.pres().slide_w as f64;
        let mut bb = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for id in ids {
            if let Some(x) = self.eff_xfrm(*id) {
                let (l0, t0, r0, b0) = bounds(&x);
                bb = (bb.0.min(l0), bb.1.min(t0), bb.2.max(r0), bb.3.max(b0));
            }
        }
        let pad = 56;
        let (x0, y0) = (((bb.0 * k) as i32 - pad).max(0), ((bb.1 * k) as i32 - pad).max(0));
        let (x1, y1) =
            (((bb.2 * k) as i32 + pad).min(out.width() as i32), ((bb.3 * k) as i32 + pad).min(out.height() as i32));
        if x1 > x0 && y1 > y0 {
            if let Some(rect) = tiny_skia::IntRect::from_xywh(x0, y0, (x1 - x0) as u32, (y1 - y0) as u32) {
                let paint = PixmapPaint::default();
                if let Some(m) = moving.as_ref().and_then(|m| m.clone_rect(rect)) {
                    out.draw_pixmap(x0, y0, m.as_ref(), &paint, Transform::identity(), None);
                }
                if let Some(a) = l.above.clone_rect(rect) {
                    out.draw_pixmap(x0, y0, a.as_ref(), &paint, Transform::identity(), None);
                }
            }
        }
        Some(out)
    }

    fn eff_xfrm(&self, id: u32) -> Option<Xfrm> {
        kinaute_render::shape_xfrm(self.pres(), self.cur, id)
    }

    /// Corner points of a transform in canvas pixels (TL, TR, BR, BL).
    fn corners_px(&self, x: &Xfrm) -> [(f32, f32); 4] {
        let (ox, oy, k) = self.view();
        let (cx, cy) = x.center();
        let a = x.rot_deg().to_radians();
        let (hw, hh) = (x.w as f64 / 2.0, x.h as f64 / 2.0);
        let c = |dx: f64, dy: f64| {
            let (rx, ry) = rot_pt(dx, dy, a);
            (ox + (cx + rx) as f32 * k, oy + (cy + ry) as f32 * k)
        };
        [c(-hw, -hh), c(hw, -hh), c(hw, hh), c(-hw, hh)]
    }

    fn handle_points(&self, x: &Xfrm) -> Vec<(Handle, (f32, f32))> {
        let (ox, oy, k) = self.view();
        let (cx, cy) = x.center();
        let a = x.rot_deg().to_radians();
        let (hw, hh) = (x.w as f64 / 2.0, x.h as f64 / 2.0);
        let pt = |dx: f64, dy: f64| {
            let (rx, ry) = rot_pt(dx, dy, a);
            (ox + (cx + rx) as f32 * k, oy + (cy + ry) as f32 * k)
        };
        let mut v = Vec::new();
        for hy in [-1i8, 0, 1] {
            for hx in [-1i8, 0, 1] {
                if hx == 0 && hy == 0 {
                    continue;
                }
                v.push((Handle::Resize(hx, hy), pt(hx as f64 * hw, hy as f64 * hh)));
            }
        }
        // Rotation handle above the top edge.
        let (rx, ry) = rot_pt(0.0, -hh, a);
        let (tx, ty) = (ox + (cx + rx) as f32 * k, oy + (cy + ry) as f32 * k);
        let (ux, uy) = rot_pt(0.0, -1.0, a);
        v.push((Handle::Rotate, (tx + ux as f32 * ROT_OFFSET_PX, ty + uy as f32 * ROT_OFFSET_PX)));
        v
    }

    fn draw_overlay(&self, out: &mut Pixmap) {
        let mut stroke = Stroke { width: 1.5, ..Default::default() };
        let mut blue = Paint::default();
        blue.set_color_rgba8(ACCENT.0, ACCENT.1, ACCENT.2, 255);
        blue.anti_alias = true;
        let mut white = Paint::default();
        white.set_color_rgba8(255, 255, 255, 255);
        white.anti_alias = true;
        for id in &self.sel {
            let Some(x) = self.eff_xfrm(*id) else { continue };
            let c = self.corners_px(&x);
            let mut pb = PathBuilder::new();
            pb.move_to(c[0].0, c[0].1);
            for p in &c[1..] {
                pb.line_to(p.0, p.1);
            }
            pb.close();
            if let Some(path) = pb.finish() {
                out.stroke_path(&path, &blue, &stroke, Transform::identity(), None);
            }
        }
        if self.sel.len() == 1 {
            if let Some(x) = self.eff_xfrm(self.sel[0]) {
                let hp = self.handle_points(&x);
                // Rotation stem.
                if let (Some((_, top)), Some((_, rot))) = (
                    hp.iter().find(|(h, _)| *h == Handle::Resize(0, -1)),
                    hp.iter().find(|(h, _)| *h == Handle::Rotate),
                ) {
                    let mut pb = PathBuilder::new();
                    pb.move_to(top.0, top.1);
                    pb.line_to(rot.0, rot.1);
                    if let Some(p) = pb.finish() {
                        out.stroke_path(
                            &p,
                            &blue,
                            &Stroke { width: 1.0, ..Default::default() },
                            Transform::identity(),
                            None,
                        );
                    }
                }
                stroke.width = 1.5;
                for (h, (px, py)) in hp {
                    let mut pb = PathBuilder::new();
                    if h == Handle::Rotate {
                        if let Some(r) = tiny_skia::Rect::from_xywh(px - 5.0, py - 5.0, 10.0, 10.0) {
                            pb.push_oval(r);
                        }
                    } else if let Some(r) =
                        tiny_skia::Rect::from_xywh(px - HANDLE_PX / 2.0, py - HANDLE_PX / 2.0, HANDLE_PX, HANDLE_PX)
                    {
                        pb.push_rect(r);
                    }
                    if let Some(p) = pb.finish() {
                        out.fill_path(&p, &white, FillRule::Winding, Transform::identity(), None);
                        out.stroke_path(&p, &blue, &stroke, Transform::identity(), None);
                    }
                }
            }
        }
        // Smart guides.
        let (ox, oy, k) = self.view();
        let mut gp = Paint::default();
        gp.set_color_rgba8(GUIDE.0, GUIDE.1, GUIDE.2, 255);
        for (vertical, pos) in &self.guides {
            let mut pb = PathBuilder::new();
            if *vertical {
                let x = (ox + *pos as f32 * k).round() + 0.5;
                pb.move_to(x, 0.0);
                pb.line_to(x, out.height() as f32);
            } else {
                let y = (oy + *pos as f32 * k).round() + 0.5;
                pb.move_to(0.0, y);
                pb.line_to(out.width() as f32, y);
            }
            if let Some(p) = pb.finish() {
                out.stroke_path(&p, &gp, &Stroke { width: 1.0, ..Default::default() }, Transform::identity(), None);
            }
        }
        // Marquee and creation preview.
        let rect = match &self.drag {
            Some(Drag::Marquee { start, cur, .. }) | Some(Drag::Create { start, cur }) => Some((*start, *cur)),
            _ => None,
        };
        if let Some((a, b)) = rect {
            let (x0, y0) = (ox + a.0.min(b.0) as f32 * k, oy + a.1.min(b.1) as f32 * k);
            let (w, h) = ((a.0 - b.0).abs() as f32 * k, (a.1 - b.1).abs() as f32 * k);
            if let Some(r) = tiny_skia::Rect::from_xywh(x0, y0, w.max(0.5), h.max(0.5)) {
                let mut pb = PathBuilder::new();
                pb.push_rect(r);
                if let Some(p) = pb.finish() {
                    let mut f = Paint::default();
                    f.set_color_rgba8(ACCENT.0, ACCENT.1, ACCENT.2, 40);
                    out.fill_path(&p, &f, FillRule::Winding, Transform::identity(), None);
                    out.stroke_path(
                        &p,
                        &blue,
                        &Stroke { width: 1.0, ..Default::default() },
                        Transform::identity(),
                        None,
                    );
                }
            }
        }
    }

    // -------------------------------------------------------- selection --

    pub fn select_slide(&mut self, i: usize) {
        if i < self.slide_count() && i != self.cur {
            self.cur = i;
            self.sel.clear();
            self.doc.history.seal();
        }
    }

    pub fn clamp_state(&mut self) {
        let n = self.slide_count();
        if n == 0 {
            self.cur = 0;
        } else if self.cur >= n {
            self.cur = n - 1;
        }
        let alive: Vec<u32> =
            self.pres().slides.get(self.cur).map(|s| s.shapes.iter().map(|x| x.id).collect()).unwrap_or_default();
        self.sel.retain(|id| alive.contains(id));
    }

    fn shapes(&self) -> &[Shape] {
        self.pres().slides.get(self.cur).map(|s| s.shapes.as_slice()).unwrap_or(&[])
    }

    pub fn select_all(&mut self) {
        let ctx_ids: Vec<u32> = {
            let Some(s) = self.pres().slides.get(self.cur) else { return };
            let ctx = Ctx::for_slide(self.pres(), s);
            s.shapes.iter().filter(|x| !x.hidden && ctx.xfrm(x).is_some()).map(|x| x.id).collect()
        };
        self.sel = ctx_ids;
    }

    fn hit_handle(&self, px: f32, py: f32) -> Option<Handle> {
        if self.sel.len() != 1 {
            return None;
        }
        let x = self.eff_xfrm(self.sel[0])?;
        let tol = HANDLE_PX;
        self.handle_points(&x)
            .into_iter()
            .filter(|(_, (hx, hy))| (px - hx).abs() <= tol && (py - hy).abs() <= tol)
            .min_by(|a, b| {
                let d = |p: (f32, f32)| (p.0 - px).powi(2) + (p.1 - py).powi(2);
                d(a.1).partial_cmp(&d(b.1)).unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(h, _)| h)
    }

    // ------------------------------------------------------------ mouse --

    pub fn hover(&mut self, px: f32, py: f32) {
        self.cursor_handle = self.hit_handle(px, py);
    }

    pub fn press(&mut self, px: f32, py: f32, shift: bool, ctrl: bool) {
        self.layers = None;
        let (ex, ey) = self.to_emu(px, py);
        self.doc.history.seal();
        self.guides.clear();
        if ctrl && self.zoom > 1.0 {
            self.drag = Some(Drag::Pan { last: (px, py) });
            return;
        }
        if self.tool != "select" {
            self.drag = Some(Drag::Create { start: (ex, ey), cur: (ex, ey) });
            return;
        }
        if let Some(h) = self.hit_handle(px, py) {
            let id = self.sel[0];
            if let Some(orig) = self.eff_xfrm(id) {
                self.drag = Some(match h {
                    Handle::Rotate => Drag::Rotate { id, orig },
                    Handle::Resize(hx, hy) => Drag::Resize { id, hx, hy, orig, keep_ratio: shift },
                });
                return;
            }
        }
        match kinaute_render::hit_test(self.pres(), self.cur, ex, ey) {
            Some(id) => {
                if shift {
                    if let Some(p) = self.sel.iter().position(|s| *s == id) {
                        self.sel.remove(p);
                    } else {
                        self.sel.push(id);
                    }
                } else if !self.sel.contains(&id) {
                    self.sel = vec![id];
                }
                let orig: Vec<(u32, Xfrm)> =
                    self.sel.iter().filter_map(|i| self.eff_xfrm(*i).map(|x| (*i, x))).collect();
                if !orig.is_empty() {
                    self.drag = Some(Drag::Move { start: (ex, ey), orig, moved: false });
                }
            }
            None => {
                if !shift {
                    self.sel.clear();
                }
                self.drag = Some(Drag::Marquee { start: (ex, ey), cur: (ex, ey), add: shift });
            }
        }
    }

    /// Snap a moving box (in EMU) against other shapes and the slide.
    /// Returns the correction and records guide lines.
    fn snap(&mut self, bx: (f64, f64, f64, f64), exclude: &[u32], edges_x: &[usize], edges_y: &[usize]) -> (f64, f64) {
        let (_, _, k) = self.view();
        let tol = SNAP_PX / k as f64;
        let p = self.pres();
        let mut xs = vec![0.0, p.slide_w as f64 / 2.0, p.slide_w as f64];
        let mut ys = vec![0.0, p.slide_h as f64 / 2.0, p.slide_h as f64];
        if let Some(s) = p.slides.get(self.cur) {
            let ctx = Ctx::for_slide(p, s);
            for sh in &s.shapes {
                if exclude.contains(&sh.id) || sh.hidden {
                    continue;
                }
                if let Some(x) = ctx.xfrm(sh) {
                    let (l, t, r, b) = bounds(&x);
                    xs.extend([l, (l + r) / 2.0, r]);
                    ys.extend([t, (t + b) / 2.0, b]);
                }
            }
        }
        let mine_x = [bx.0, (bx.0 + bx.2) / 2.0, bx.2];
        let mine_y = [bx.1, (bx.1 + bx.3) / 2.0, bx.3];
        let best = |mine: &[f64; 3], idx: &[usize], cand: &[f64]| -> Option<(f64, f64)> {
            let mut best: Option<(f64, f64)> = None;
            for i in idx {
                for c in cand {
                    let d = c - mine[*i];
                    if d.abs() <= tol && best.map(|b| d.abs() < b.0.abs()).unwrap_or(true) {
                        best = Some((d, *c));
                    }
                }
            }
            best
        };
        self.guides.clear();
        let (mut dx, mut dy) = (0.0, 0.0);
        if let Some((d, g)) = best(&mine_x, edges_x, &xs) {
            dx = d;
            self.guides.push((true, g));
        }
        if let Some((d, g)) = best(&mine_y, edges_y, &ys) {
            dy = d;
            self.guides.push((false, g));
        }
        (dx, dy)
    }

    pub fn drag_to(&mut self, px: f32, py: f32, shift: bool) {
        let (ex, ey) = self.to_emu(px, py);
        let Some(drag) = self.drag.clone() else { return };
        match drag {
            Drag::Pan { last } => {
                self.pan.0 += px - last.0;
                self.pan.1 += py - last.1;
                self.drag = Some(Drag::Pan { last: (px, py) });
            }
            Drag::Marquee { start, add, .. } => {
                self.drag = Some(Drag::Marquee { start, cur: (ex, ey), add });
                let (x0, x1) = (start.0.min(ex), start.0.max(ex));
                let (y0, y1) = (start.1.min(ey), start.1.max(ey));
                let hits: Vec<u32> = {
                    let Some(s) = self.pres().slides.get(self.cur) else { return };
                    let ctx = Ctx::for_slide(self.pres(), s);
                    s.shapes
                        .iter()
                        .filter(|sh| !sh.hidden)
                        .filter_map(|sh| ctx.xfrm(sh).map(|x| (sh.id, bounds(&x))))
                        .filter(|(_, b)| b.0 < x1 && b.2 > x0 && b.1 < y1 && b.3 > y0)
                        .map(|(id, _)| id)
                        .collect()
                };
                if add {
                    for h in hits {
                        if !self.sel.contains(&h) {
                            self.sel.push(h);
                        }
                    }
                } else {
                    self.sel = hits;
                }
            }
            Drag::Create { start, .. } => {
                self.drag = Some(Drag::Create { start, cur: (ex, ey) });
            }
            Drag::Move { start, orig, moved } => {
                let (mut dx, mut dy) = (ex - start.0, ey - start.1);
                if !moved && (dx.abs() + dy.abs()) * (self.view().2 as f64) < 3.0 {
                    return;
                }
                if shift {
                    // Constrain to an axis.
                    if dx.abs() > dy.abs() {
                        dy = 0.0
                    } else {
                        dx = 0.0
                    }
                }
                // Union bounding box of the originals, moved.
                let mut b = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
                for (_, x) in &orig {
                    let (l, t, r, bt) = bounds(x);
                    b = (b.0.min(l), b.1.min(t), b.2.max(r), b.3.max(bt));
                }
                let moved_box = (b.0 + dx, b.1 + dy, b.2 + dx, b.3 + dy);
                let ids: Vec<u32> = orig.iter().map(|(i, _)| *i).collect();
                let (sx, sy) = self.snap(moved_box, &ids, &[0, 1, 2], &[0, 1, 2]);
                let (dx, dy) = (dx + sx, dy + sy);
                let items: Vec<(u32, Xfrm)> = orig
                    .iter()
                    .map(|(i, x)| (*i, Xfrm { x: x.x + dx.round() as i64, y: x.y + dy.round() as i64, ..*x }))
                    .collect();
                self.apply_drag(Command::SetXfrm { slide: self.cur, items });
                self.drag = Some(Drag::Move { start, orig, moved: true });
            }
            Drag::Resize { id, hx, hy, orig, keep_ratio } => {
                let (cx, cy) = orig.center();
                let a = orig.rot_deg().to_radians();
                let (lx, ly) = rot_pt(ex - cx, ey - cy, -a);
                let (w, h) = (orig.w as f64, orig.h as f64);
                let min = 12_700.0;
                let (ax, ay) = (-hx as f64 * w / 2.0, -hy as f64 * h / 2.0);
                let mut nw = if hx != 0 { (hx as f64 * (lx - ax)).max(min) } else { w };
                let mut nh = if hy != 0 { (hy as f64 * (ly - ay)).max(min) } else { h };
                if (keep_ratio || shift) && hx != 0 && hy != 0 && w > 0.0 && h > 0.0 {
                    let s = (nw / w).max(nh / h);
                    nw = w * s;
                    nh = h * s;
                } else if shift && hx != 0 && hy == 0 && w > 0.0 {
                    nh = h * nw / w;
                } else if shift && hy != 0 && hx == 0 && h > 0.0 {
                    nw = w * nh / h;
                }
                // Snap the dragged edges when unrotated.
                if orig.rot == 0 {
                    let (l, t) = (orig.x as f64, orig.y as f64);
                    let left = if hx < 0 { l + w - nw } else { l };
                    let top = if hy < 0 { t + h - nh } else { t };
                    let bx = (left, top, left + nw, top + nh);
                    let ex_idx: Vec<usize> = match hx {
                        -1 => vec![0],
                        1 => vec![2],
                        _ => vec![],
                    };
                    let ey_idx: Vec<usize> = match hy {
                        -1 => vec![0],
                        1 => vec![2],
                        _ => vec![],
                    };
                    let (sx, sy) = self.snap(bx, &[id], &ex_idx, &ey_idx);
                    if hx != 0 {
                        nw = (nw + sx * hx as f64).max(min);
                    }
                    if hy != 0 {
                        nh = (nh + sy * hy as f64).max(min);
                    }
                }
                let (offx, offy) = rot_pt(ax + hx as f64 * nw / 2.0, ay + hy as f64 * nh / 2.0, a);
                let (anx, any) = rot_pt(ax, ay, a);
                let (ncx, ncy) = (cx + anx + (offx - anx), cy + any + (offy - any));
                let nx = Xfrm {
                    x: (ncx - nw / 2.0).round() as i64,
                    y: (ncy - nh / 2.0).round() as i64,
                    w: nw.round() as i64,
                    h: nh.round() as i64,
                    ..orig
                };
                self.apply_drag(Command::SetXfrm { slide: self.cur, items: vec![(id, nx)] });
            }
            Drag::Rotate { id, orig } => {
                let (cx, cy) = orig.center();
                let mut deg = (ey - cy).atan2(ex - cx).to_degrees() + 90.0;
                if shift {
                    deg = (deg / 15.0).round() * 15.0;
                } else {
                    let snapped = (deg / 45.0).round() * 45.0;
                    if (deg - snapped).abs() < 2.5 {
                        deg = snapped;
                    }
                }
                let rot = ((deg.rem_euclid(360.0)) * 60_000.0).round() as i32;
                self.apply_drag(Command::SetXfrm { slide: self.cur, items: vec![(id, Xfrm { rot, ..orig })] });
            }
        }
    }

    fn apply_drag(&mut self, cmd: Command) {
        if self.drag_key == 0 {
            self.drag_key = 1;
        }
        if let Err(e) = self.doc.history.apply_coalesced(cmd, Some(self.drag_key)) {
            self.status = format!("Erreur : {e}");
        }
    }

    pub fn release(&mut self, px: f32, py: f32) {
        let (ex, ey) = self.to_emu(px, py);
        let drag = self.drag.take();
        self.guides.clear();
        self.doc.history.seal();
        self.drag_key = self.drag_key.wrapping_add(1).max(1);
        if let Some(Drag::Create { start, .. }) = drag {
            self.create_at(start, (ex, ey));
        }
    }

    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Create a shape for the active tool in the dragged rectangle.
    fn create_at(&mut self, a: (f64, f64), b: (f64, f64)) {
        let tool = self.tool.clone();
        let click = (a.0 - b.0).abs().max((a.1 - b.1).abs()) * (self.view().2 as f64) < 4.0;
        let (mut x, mut y) = (a.0.min(b.0), a.1.min(b.1));
        let (mut w, mut h) = ((a.0 - b.0).abs(), (a.1 - b.1).abs());
        if click {
            let (dw, dh) = match tool.as_str() {
                "text" => (7.0 * EMU_CM, 1.6 * EMU_CM),
                "line" => (6.0 * EMU_CM, 0.0),
                _ => (4.0 * EMU_CM, 4.0 * EMU_CM),
            };
            x = a.0 - dw / 2.0;
            y = a.1 - dh / 2.0;
            w = dw;
            h = dh;
        }
        let xf = Xfrm::new(
            x.round() as i64,
            y.round() as i64,
            w.round() as i64,
            h.round().max(if tool == "line" { 0.0 } else { 1.0 }) as i64,
        );
        let shape = match tool.as_str() {
            "text" => new::text_box(0, xf, "Texte", 2400),
            "line" => new::line(0, xf),
            "image" => return,
            preset => new::shape(0, xf, preset),
        };
        self.tool = "select".into();
        self.insert_shapes(vec![shape]);
    }

    // --------------------------------------------------------- commands --

    pub fn run(&mut self, cmd: Command) {
        if let Err(e) = self.doc.history.apply(cmd) {
            self.status = format!("Erreur : {e}");
        }
        self.clamp_state();
    }

    pub fn insert_shapes(&mut self, shapes: Vec<Shape>) {
        let before: Vec<u32> = self.shapes().iter().map(|s| s.id).collect();
        self.run(Command::InsertShapes { slide: self.cur, shapes });
        self.sel = self.shapes().iter().map(|s| s.id).filter(|id| !before.contains(id)).collect();
    }

    pub fn undo(&mut self) {
        self.doc.history.undo();
        self.clamp_state();
    }

    pub fn redo(&mut self) {
        self.doc.history.redo();
        self.clamp_state();
    }

    pub fn delete_selection(&mut self) {
        if self.sel.is_empty() {
            return;
        }
        let ids = std::mem::take(&mut self.sel);
        self.run(Command::DeleteShapes { slide: self.cur, ids });
    }

    pub fn copy(&mut self) {
        let shapes: Vec<Shape> = self.shapes().iter().filter(|s| self.sel.contains(&s.id)).cloned().collect();
        if !shapes.is_empty() {
            self.clipboard = shapes;
            self.status = format!("{} objet(s) copié(s)", self.clipboard.len());
        }
    }

    pub fn paste(&mut self) {
        if self.clipboard.is_empty() {
            return;
        }
        let off = (0.5 * EMU_CM) as i64;
        let shapes: Vec<Shape> = self
            .clipboard
            .iter()
            .map(|s| {
                let mut s = s.clone();
                s.id = 0;
                s.source = None;
                if let Some(x) = &mut s.xfrm {
                    x.x += off;
                    x.y += off;
                }
                s
            })
            .collect();
        // Keep the clipboard cascading for repeated pastes.
        for c in &mut self.clipboard {
            if let Some(x) = &mut c.xfrm {
                x.x += off;
                x.y += off;
            }
        }
        self.insert_shapes(shapes);
    }

    pub fn duplicate_selection(&mut self) {
        self.copy();
        self.paste();
    }

    pub fn nudge(&mut self, dx: f64, dy: f64) {
        let items: Vec<(u32, Xfrm)> = self
            .sel
            .iter()
            .filter_map(|id| {
                self.eff_xfrm(*id).map(|x| (*id, Xfrm { x: x.x + dx.round() as i64, y: x.y + dy.round() as i64, ..x }))
            })
            .collect();
        if !items.is_empty() {
            self.drag_key = self.drag_key.wrapping_add(1).max(1);
            let key = 0x4000_0000 + self.drag_key;
            let _ = self.doc.history.apply_coalesced(Command::SetXfrm { slide: self.cur, items }, Some(key));
        }
    }

    pub fn set_xfrm_of_selection(
        &mut self,
        x: Option<f64>,
        y: Option<f64>,
        w: Option<f64>,
        h: Option<f64>,
        rot: Option<f64>,
    ) {
        let Some(&id) = self.sel.first() else { return };
        let Some(cur) = self.eff_xfrm(id) else { return };
        let nx = Xfrm {
            x: x.map(|v| (v * EMU_CM) as i64).unwrap_or(cur.x),
            y: y.map(|v| (v * EMU_CM) as i64).unwrap_or(cur.y),
            w: w.map(|v| ((v * EMU_CM) as i64).max(1000)).unwrap_or(cur.w),
            h: h.map(|v| ((v * EMU_CM) as i64).max(0)).unwrap_or(cur.h),
            rot: rot.map(|v| (v.rem_euclid(360.0) * 60_000.0) as i32).unwrap_or(cur.rot),
            ..cur
        };
        self.run(Command::SetXfrm { slide: self.cur, items: vec![(id, nx)] });
    }

    pub fn arrange(&mut self, what: &str) {
        let ids = self.sel.clone();
        if ids.is_empty() {
            return;
        }
        let slide = self.cur;
        match what {
            "front" => self.run(Command::Reorder { slide, ids, how: ZOrder::Front }),
            "back" => self.run(Command::Reorder { slide, ids, how: ZOrder::Back }),
            "forward" => self.run(Command::Reorder { slide, ids, how: ZOrder::Forward }),
            "backward" => self.run(Command::Reorder { slide, ids, how: ZOrder::Backward }),
            "group" => {
                self.run(Command::Group { slide, ids });
                self.sel = self.shapes().last().map(|s| vec![s.id]).unwrap_or_default();
                // The group is inserted where its first member was; select it by name.
                if let Some(g) = self.shapes().iter().rev().find(|s| s.is_group()) {
                    self.sel = vec![g.id];
                }
            }
            "ungroup" => {
                if let Some(&id) = ids.first() {
                    let kids: Vec<u32> = self
                        .shapes()
                        .iter()
                        .find(|s| s.id == id)
                        .and_then(|s| {
                            if let ShapeKind::Group(g) = &s.kind {
                                Some(g.shapes.iter().map(|c| c.id).collect())
                            } else {
                                None
                            }
                        })
                        .unwrap_or_default();
                    self.run(Command::Ungroup { slide, id });
                    self.sel = kids;
                }
            }
            "al" | "ac" | "ar" | "at" | "am" | "ab" | "dh" | "dv" => self.align(what),
            _ => {}
        }
    }

    fn align(&mut self, what: &str) {
        let items: Vec<(u32, Xfrm)> = self.sel.iter().filter_map(|i| self.eff_xfrm(*i).map(|x| (*i, x))).collect();
        if items.is_empty() {
            return;
        }
        let p = self.pres();
        // One object aligns to the slide, several to their common bounds.
        let (l, t, r, b) = if items.len() == 1 {
            (0.0, 0.0, p.slide_w as f64, p.slide_h as f64)
        } else {
            items.iter().fold((f64::MAX, f64::MAX, f64::MIN, f64::MIN), |a, (_, x)| {
                let bb = bounds(x);
                (a.0.min(bb.0), a.1.min(bb.1), a.2.max(bb.2), a.3.max(bb.3))
            })
        };
        let mut out: Vec<(u32, Xfrm)> = Vec::new();
        if what == "dh" || what == "dv" {
            let horiz = what == "dh";
            let mut v: Vec<(u32, Xfrm, Bounds)> = items.iter().map(|(i, x)| (*i, *x, bounds(x))).collect();
            v.sort_by(|a, b| {
                let (ka, kb) = if horiz { (a.2 .0, b.2 .0) } else { (a.2 .1, b.2 .1) };
                ka.partial_cmp(&kb).unwrap_or(std::cmp::Ordering::Equal)
            });
            let total: f64 = v.iter().map(|(_, _, bb)| if horiz { bb.2 - bb.0 } else { bb.3 - bb.1 }).sum();
            let span = if horiz { r - l } else { b - t };
            let gap = (span - total) / (v.len().saturating_sub(1)).max(1) as f64;
            let mut pos = if horiz { l } else { t };
            for (id, x, bb) in v {
                let size = if horiz { bb.2 - bb.0 } else { bb.3 - bb.1 };
                let delta = pos - if horiz { bb.0 } else { bb.1 };
                let nx = if horiz {
                    Xfrm { x: x.x + delta.round() as i64, ..x }
                } else {
                    Xfrm { y: x.y + delta.round() as i64, ..x }
                };
                out.push((id, nx));
                pos += size + gap;
            }
        } else {
            for (id, x) in &items {
                let bb = bounds(x);
                let (w, h) = (bb.2 - bb.0, bb.3 - bb.1);
                let (nl, nt) = match what {
                    "al" => (l, bb.1),
                    "ac" => ((l + r) / 2.0 - w / 2.0, bb.1),
                    "ar" => (r - w, bb.1),
                    "at" => (bb.0, t),
                    "am" => (bb.0, (t + b) / 2.0 - h / 2.0),
                    _ => (bb.0, b - h),
                };
                out.push((
                    *id,
                    Xfrm { x: x.x + (nl - bb.0).round() as i64, y: x.y + (nt - bb.1).round() as i64, ..*x },
                ));
            }
        }
        self.run(Command::SetXfrm { slide: self.cur, items: out });
    }

    pub fn set_fill(&mut self, c: Option<[u8; 3]>) {
        let fill = Some(match c {
            Some([r, g, b]) => Fill::Solid(Color::srgb(&Rgba::from_u8(r, g, b).to_hex())),
            None => Fill::None,
        });
        let ids = self.sel.clone();
        self.run(Command::SetFill { slide: self.cur, ids, fill });
    }

    pub fn set_line(&mut self, c: Option<[u8; 3]>, width_pt: Option<f64>) {
        let ids = self.sel.clone();
        let cur = self.sel_info().line;
        let width = width_pt.or(cur.map(|l| l.1)).unwrap_or(1.0);
        let line = match c {
            Some([r, g, b]) => Line {
                width: Some((width * EMU_PT) as i64),
                fill: Some(Fill::Solid(Color::srgb(&Rgba::from_u8(r, g, b).to_hex()))),
                ..Default::default()
            },
            None if width_pt.is_some() && cur.is_some() => {
                let rgb = cur.map(|l| l.0).unwrap_or([0, 0, 0, 255]);
                Line {
                    width: Some((width * EMU_PT) as i64),
                    fill: Some(Fill::Solid(Color::srgb(&Rgba::from_u8(rgb[0], rgb[1], rgb[2]).to_hex()))),
                    ..Default::default()
                }
            }
            None => Line { fill: Some(Fill::None), ..Default::default() },
        };
        self.run(Command::SetLine { slide: self.cur, ids, line: Some(line) });
    }

    pub fn set_text(&mut self, id: u32, text: &str) {
        let key = 0x2000_0000 + id as u64;
        let _ = self
            .doc
            .history
            .apply_coalesced(Command::SetText { slide: self.cur, id, text: text.to_string() }, Some(key));
    }

    pub fn patch_run(&mut self, patch: RunPatch) {
        let ids = self.sel.clone();
        self.run(Command::SetRunProps { slide: self.cur, ids, patch });
    }

    pub fn toggle_bold(&mut self) {
        let v = !self.sel_info().bold;
        self.patch_run(RunPatch { bold: Some(v), ..Default::default() });
    }
    pub fn toggle_italic(&mut self) {
        let v = !self.sel_info().italic;
        self.patch_run(RunPatch { italic: Some(v), ..Default::default() });
    }
    pub fn toggle_underline(&mut self) {
        let v = !self.sel_info().underline;
        self.patch_run(RunPatch { underline: Some(v), ..Default::default() });
    }

    pub fn set_halign(&mut self, a: i32) {
        let align = match a {
            1 => Align::Center,
            2 => Align::Right,
            3 => Align::Justify,
            _ => Align::Left,
        };
        let ids = self.sel.clone();
        self.run(Command::SetAlign { slide: self.cur, ids, align });
    }

    pub fn set_valign(&mut self, a: i32) {
        let anchor = match a {
            1 => Anchor::Middle,
            2 => Anchor::Bottom,
            _ => Anchor::Top,
        };
        let ids = self.sel.clone();
        self.run(Command::SetAnchor { slide: self.cur, ids, anchor });
    }

    // ----------------------------------------------------------- slides --

    /// Add a slide after the current one using the same layout (or the
    /// "title and content" layout), with empty placeholders.
    pub fn add_slide(&mut self) {
        let p = self.pres();
        let cur_layout = p.slides.get(self.cur).map(|s| s.layout).unwrap_or(0);
        // After a title slide, default to a content layout.
        let layout_idx = match p.layouts.get(cur_layout) {
            Some(l) if l.kind.as_deref() == Some("title") => {
                p.layouts.iter().position(|l| l.kind.as_deref() == Some("obj")).unwrap_or(cur_layout)
            }
            _ => cur_layout,
        };
        let mut shapes = Vec::new();
        if let Some(l) = p.layouts.get(layout_idx) {
            for s in &l.shapes {
                let Some(ph) = &s.ph else { continue };
                if matches!(ph.kind.as_str(), "dt" | "ftr" | "sldNum") {
                    continue;
                }
                let mut tb = TextBody::default();
                tb.paragraphs.push(Paragraph::default());
                shapes.push(Shape {
                    id: shapes.len() as u32 + 2,
                    name: s.name.clone(),
                    descr: None,
                    hidden: false,
                    ph: Some(ph.clone()),
                    xfrm: None,
                    kind: ShapeKind::Shape(Box::new(SpProps { text: Some(tb), ..Default::default() })),
                    source: None,
                });
            }
        }
        let slide = Slide {
            rels: Vec::new(),
            part: None,
            layout: layout_idx,
            name: None,
            background: None,
            shapes,
            transition: None,
            hidden: false,
            show_master_shapes: true,
            notes: None,
            source: None,
        };
        let at = (self.cur + 1).min(self.slide_count());
        self.run(Command::InsertSlide { at, slide: Box::new(slide) });
        self.cur = at;
        self.sel.clear();
    }

    pub fn duplicate_slide(&mut self) {
        let i = self.cur;
        self.run(Command::DuplicateSlide { index: i });
        self.cur = i + 1;
        self.sel.clear();
    }

    pub fn delete_slide(&mut self) {
        if self.slide_count() <= 1 {
            self.status = "Une présentation doit contenir au moins une diapositive".into();
            return;
        }
        let i = self.cur;
        self.run(Command::DeleteSlide { index: i });
        self.cur = i.min(self.slide_count() - 1);
        self.sel.clear();
    }

    pub fn move_slide(&mut self, delta: i32) {
        let from = self.cur;
        let to = (from as i32 + delta).clamp(0, self.slide_count() as i32 - 1) as usize;
        if to != from {
            self.run(Command::MoveSlide { from, to });
            self.cur = to;
        }
    }

    pub fn move_slide_to(&mut self, from: usize, to: usize) {
        if from != to && from < self.slide_count() {
            self.run(Command::MoveSlide { from, to });
            self.cur = to.min(self.slide_count() - 1);
        }
    }

    // -------------------------------------------------------- inspector --

    pub fn sel_info(&self) -> SelInfo {
        let mut info = SelInfo { count: self.sel.len(), ..Default::default() };
        let Some(&id) = self.sel.first() else { return info };
        let Some(slide) = self.pres().slides.get(self.cur) else { return info };
        let Some(sh) = slide.shapes.iter().find(|s| s.id == id) else { return info };
        let ctx = Ctx::for_slide(self.pres(), slide);
        info.xfrm = ctx.xfrm(sh);
        info.is_group = sh.is_group();
        if let Some(tb) = sh.text() {
            info.has_text = true;
            info.text = tb.plain_text();
            let para = tb.paragraphs.first().cloned().unwrap_or_default();
            let pp = ctx.para_props(sh, tb, &para);
            let run = para
                .runs
                .first()
                .map(|r| r.props.clone())
                .unwrap_or_else(|| para.end_props.clone().unwrap_or_default());
            let rp = ctx.run_props(&pp, &run);
            info.size_pt = rp.size.unwrap_or(1800) as f32 / 100.0;
            info.bold = rp.bold.unwrap_or(false);
            info.italic = rp.italic.unwrap_or(false);
            info.underline = rp.underline.as_deref().map(|u| u != "none").unwrap_or(false);
            info.h_align = match pp.align.unwrap_or(Align::Left) {
                Align::Left => 0,
                Align::Center => 1,
                Align::Right => 2,
                Align::Justify => 3,
            };
            info.v_align = match ctx.body_props(sh).anchor.unwrap_or(Anchor::Top) {
                Anchor::Top => 0,
                Anchor::Middle => 1,
                Anchor::Bottom => 2,
            };
        }
        if let Some((f, ph)) = ctx.shape_fill(sh) {
            match f {
                Fill::Solid(c) => info.fill = Some(ctx.color_with(&c, ph.as_ref()).to_u8()),
                Fill::Gradient(g) => info.fill = g.stops.first().map(|(_, c)| ctx.color_with(c, ph.as_ref()).to_u8()),
                Fill::Pattern { fg, .. } => {
                    info.fill =
                        Some(fg.as_ref().map(|c| ctx.color_with(c, ph.as_ref()).to_u8()).unwrap_or([0, 0, 0, 255]))
                }
                _ => {}
            }
        }
        if let Some((l, ph)) = ctx.shape_line(sh) {
            if let Some(Fill::Solid(c)) = &l.fill {
                info.line = Some((ctx.color_with(c, ph.as_ref()).to_u8(), l.width.unwrap_or(12700) as f64 / EMU_PT));
            }
        }
        info
    }

    pub fn set_transition(&mut self, kind: &str, secs: f64, advance_on_click: bool, all: bool) {
        let t = if kind == "none" {
            None
        } else {
            let prev = self.pres().slides.get(self.cur).and_then(|s| s.transition.clone()).unwrap_or_default();
            Some(Transition {
                kind: kind.to_string(),
                dir: prev.dir,
                duration_ms: (secs * 1000.0).clamp(100.0, 10_000.0) as u32,
                advance_on_click,
                advance_after_ms: prev.advance_after_ms,
            })
        };
        let slides: Vec<usize> = if all { (0..self.slide_count()).collect() } else { vec![self.cur] };
        self.run(Command::SetTransition { slides, transition: t });
    }

    pub fn current_transition(&self) -> Option<Transition> {
        self.pres().slides.get(self.cur).and_then(|s| s.transition.clone())
    }

    pub fn toggle_hidden(&mut self) {
        let h = self.pres().slides.get(self.cur).map(|s| s.hidden).unwrap_or(false);
        self.run(Command::SetHidden { slide: self.cur, hidden: !h });
    }

    // ------------------------------------------------------------- files --

    pub fn save_to(&mut self, path: PathBuf) -> Result<(), String> {
        let bytes = self.doc.save_to_bytes().map_err(|e| e.to_string())?;
        // Write atomically: never leave a half-written presentation.
        let tmp = path.with_extension("pptx.kinaute-tmp");
        std::fs::write(&tmp, &bytes).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &path).map_err(|e| e.to_string())?;
        self.doc.history.mark_saved();
        self.path = Some(path);
        Ok(())
    }

    pub fn add_image(&mut self, path: &std::path::Path) -> Result<(), String> {
        let data = std::fs::read(path).map_err(|e| e.to_string())?;
        let reader =
            image::ImageReader::new(std::io::Cursor::new(&data)).with_guessed_format().map_err(|e| e.to_string())?;
        let (iw, ih) = reader.into_dimensions().map_err(|e| format!("image illisible : {e}"))?;
        let ext = match path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
            Some("jpg") | Some("jpeg") => "jpeg",
            Some("gif") => "gif",
            Some("bmp") => "bmp",
            Some("tif") | Some("tiff") => "tiff",
            _ => "png",
        };
        let media = &self.pres().media;
        let n = (1..).find(|n| !media.contains_key(&format!("ppt/media/kinaute{n}.{ext}"))).unwrap_or(1);
        let part = format!("ppt/media/kinaute{n}.{ext}");
        self.run(Command::AddMedia { part: part.clone(), bytes: Arc::new(data) });
        self.renderer.borrow_mut().set_media(self.pres().media.clone());
        // Fit within 60 % of the slide, centred.
        let (sw, sh) = (self.pres().slide_w as f64, self.pres().slide_h as f64);
        let scale = (sw * 0.6 / iw as f64).min(sh * 0.6 / ih as f64);
        let (w, h) = (iw as f64 * scale, ih as f64 * scale);
        let xf = Xfrm::new(((sw - w) / 2.0) as i64, ((sh - h) / 2.0) as i64, w as i64, h as i64);
        self.insert_shapes(vec![new::picture(0, xf, &part)]);
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    fn app() -> App {
        let doc = kinaute_pptx::blank().expect("blank");
        let mut a = App::new(doc, None, Rc::new(RefCell::new(Renderer::new())));
        a.set_canvas(1000, 700);
        a
    }

    fn center_px(a: &App, id: u32) -> (f32, f32) {
        let x = a.eff_xfrm(id).expect("xfrm");
        let (cx, cy) = x.center();
        let (ox, oy, k) = a.view();
        (ox + cx as f32 * k, oy + cy as f32 * k)
    }

    #[test]
    fn create_move_undo() {
        let mut a = app();
        a.tool = "rect".into();
        let (ox, oy, k) = a.view();
        let p = |ex: f64, ey: f64| (ox + ex as f32 * k, oy + ey as f32 * k);
        let (x0, y0) = p(2e6, 2e6);
        let (x1, y1) = p(4e6, 3.5e6);
        a.press(x0, y0, false, false);
        a.drag_to(x1, y1, false);
        a.release(x1, y1);
        assert_eq!(a.sel.len(), 1, "created shape is selected");
        let id = a.sel[0];
        let before = a.eff_xfrm(id).expect("xfrm");
        assert!((before.w as f64 - 2e6).abs() < 2e4 && (before.h as f64 - 1.5e6).abs() < 2e4);

        // Drag it.
        let (cx, cy) = center_px(&a, id);
        a.press(cx, cy, false, false);
        a.drag_to(cx + 60.0, cy + 40.0, false);
        a.release(cx + 60.0, cy + 40.0);
        let after = a.eff_xfrm(id).expect("xfrm");
        assert!(after.x > before.x && after.y > before.y);

        // One drag = one undo step.
        a.undo();
        assert_eq!(a.eff_xfrm(id).map(|x| (x.x, x.y)), Some((before.x, before.y)));
        a.redo();
        assert_eq!(a.eff_xfrm(id).map(|x| (x.x, x.y)), Some((after.x, after.y)));
    }

    #[test]
    fn resize_rotated_keeps_opposite_corner() {
        let mut a = app();
        a.insert_shapes(vec![new::shape(
            0,
            Xfrm { rot: 30 * 60_000, ..Xfrm::new(3_000_000, 2_000_000, 2_000_000, 1_000_000) },
            "rect",
        )]);
        let id = a.sel[0];
        let orig = a.eff_xfrm(id).expect("xfrm");
        let corner = |x: &Xfrm, sx: f64, sy: f64| {
            let (cx, cy) = x.center();
            let (rx, ry) = rot_pt(sx * x.w as f64 / 2.0, sy * x.h as f64 / 2.0, x.rot_deg().to_radians());
            (cx + rx, cy + ry)
        };
        let anchor_before = corner(&orig, -1.0, -1.0);
        let hp = a.handle_points(&orig);
        let (_, (hx, hy)) = hp.iter().find(|(h, _)| *h == Handle::Resize(1, 1)).copied().expect("handle");
        a.press(hx, hy, false, false);
        assert!(matches!(a.drag, Some(Drag::Resize { .. })), "grabbed the handle");
        a.drag_to(hx + 40.0, hy + 25.0, false);
        a.release(hx + 40.0, hy + 25.0);
        let now = a.eff_xfrm(id).expect("xfrm");
        let anchor_after = corner(&now, -1.0, -1.0);
        assert!(
            (anchor_before.0 - anchor_after.0).abs() < 2000.0 && (anchor_before.1 - anchor_after.1).abs() < 2000.0,
            "{anchor_before:?} vs {anchor_after:?}"
        );
        assert!(now.w > orig.w);
    }

    #[test]
    fn align_group_and_slides() {
        let mut a = app();
        a.insert_shapes(vec![
            new::shape(0, Xfrm::new(1_000_000, 1_000_000, 1_000_000, 1_000_000), "rect"),
            new::shape(0, Xfrm::new(5_000_000, 3_000_000, 1_500_000, 500_000), "ellipse"),
        ]);
        a.select_all();
        let ids = a.sel.clone();
        assert_eq!(ids.len(), 4, "two placeholders + two shapes");
        a.sel = ids[2..].to_vec();
        a.arrange("al");
        let xs: Vec<i64> = a.sel.iter().filter_map(|i| a.eff_xfrm(*i)).map(|x| x.x).collect();
        assert_eq!(xs[0], xs[1]);
        a.arrange("group");
        assert!(a.shapes().iter().any(|s| s.is_group()));
        a.arrange("ungroup");
        assert!(!a.shapes().iter().any(|s| s.is_group()));

        a.add_slide();
        a.add_slide();
        assert_eq!(a.slide_count(), 3);
        a.move_slide(-1);
        assert_eq!(a.cur, 1);
        a.duplicate_slide();
        assert_eq!(a.slide_count(), 4);
        a.delete_slide();
        assert_eq!(a.slide_count(), 3);
        for _ in 0..6 {
            a.undo();
        }
        assert!(a.slide_count() >= 1);
    }

    #[test]
    fn text_edit_coalesces_and_saves() {
        let mut a = app();
        a.insert_shapes(vec![new::text_box(0, Xfrm::new(1_000_000, 1_000_000, 3_000_000, 600_000), "x", 2000)]);
        let id = a.sel[0];
        for t in ["h", "he", "hel", "hello"] {
            a.set_text(id, t);
        }
        assert_eq!(a.sel_info().text, "hello");
        a.undo(); // one step for all typing
        assert_eq!(a.sel_info().text, "x");
        a.redo();
        let dir = std::env::temp_dir().join("kinaute-test-save.pptx");
        a.save_to(dir.clone()).expect("save");
        let reopened = kinaute_pptx::open(&std::fs::read(&dir).expect("read")).expect("reopen");
        let texts: Vec<String> =
            reopened.pres().slides[0].shapes.iter().filter_map(|s| s.text().map(|t| t.plain_text())).collect();
        assert!(texts.iter().any(|t| t == "hello"), "{texts:?}");
        let _ = std::fs::remove_file(dir);
    }

    /// Frame-time benchmark for dragging: `KINAUTE_BENCH_FILE=x.pptx cargo test --release -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn bench_drag_frame_time() {
        let doc = match std::env::var("KINAUTE_BENCH_FILE") {
            Ok(f) => kinaute_pptx::open(&std::fs::read(f).expect("read")).expect("open"),
            Err(_) => kinaute_pptx::blank().expect("blank"),
        };
        let mut a = App::new(doc, None, Rc::new(RefCell::new(Renderer::new())));
        a.set_canvas(1100, 760);
        let _ = a.render_canvas(); // warm-up (fonts, caches)
        a.select_all();
        // Drag a single object, the common case.
        let Some(&id) = a.sel.last() else { return };
        a.sel = vec![id];
        let (cx, cy) = center_px(&a, id);
        a.press(cx, cy, false, false);
        let t0 = std::time::Instant::now();
        let n = 60;
        for i in 0..n {
            a.drag_to(cx + i as f32, cy + i as f32 * 0.5, false);
            let _ = a.render_canvas();
        }
        let per = t0.elapsed() / n;
        a.release(cx, cy);
        println!("drag frame (edit + full canvas render): {per:?} on slide with {} shapes", a.shapes().len());
    }

    #[test]
    fn canvas_renders() {
        let mut a = app();
        let pm = a.render_canvas().expect("canvas");
        assert_eq!((pm.width(), pm.height()), (1000, 700));
    }
}
