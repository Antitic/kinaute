//! Shape geometry: preset shapes and custom geometry (with DrawingML guide
//! formulas) turned into tiny-skia paths in a `w` × `h` pixel box.

use kinaute_model::{CustomGeometry, Geometry, PathCmd};
use std::collections::HashMap;
use tiny_skia::{Path, PathBuilder, Rect};

const K: f32 = 0.552_284_75;

fn poly(pts: &[(f32, f32)], close: bool) -> Option<Path> {
    let mut pb = PathBuilder::new();
    let (first, rest) = pts.split_first()?;
    pb.move_to(first.0, first.1);
    for p in rest {
        pb.line_to(p.0, p.1);
    }
    if close {
        pb.close();
    }
    pb.finish()
}

fn rounded_rect(w: f32, h: f32, r: f32) -> Option<Path> {
    let r = r.clamp(0.0, w.min(h) / 2.0);
    let k = r * (1.0 - K);
    let mut pb = PathBuilder::new();
    pb.move_to(r, 0.0);
    pb.line_to(w - r, 0.0);
    pb.cubic_to(w - k, 0.0, w, k, w, r);
    pb.line_to(w, h - r);
    pb.cubic_to(w, h - k, w - k, h, w - r, h);
    pb.line_to(r, h);
    pb.cubic_to(k, h, 0.0, h - k, 0.0, h - r);
    pb.line_to(0.0, r);
    pb.cubic_to(0.0, k, k, 0.0, r, 0.0);
    pb.close();
    pb.finish()
}

fn star(n: usize, inner: f32, w: f32, h: f32) -> Option<Path> {
    let (cx, cy) = (w / 2.0, h / 2.0);
    let mut pts = Vec::with_capacity(n * 2);
    for i in 0..n * 2 {
        let a = -std::f32::consts::FRAC_PI_2 + i as f32 * std::f32::consts::PI / n as f32;
        let r = if i % 2 == 0 { 1.0 } else { inner };
        pts.push((cx + a.cos() * r * w / 2.0, cy + a.sin() * r * h / 2.0));
    }
    poly(&pts, true)
}

fn regular_polygon(n: usize, w: f32, h: f32) -> Option<Path> {
    let (cx, cy) = (w / 2.0, h / 2.0);
    let pts: Vec<_> = (0..n)
        .map(|i| {
            let a = -std::f32::consts::FRAC_PI_2 + i as f32 * 2.0 * std::f32::consts::PI / n as f32;
            (cx + a.cos() * w / 2.0, cy + a.sin() * h / 2.0)
        })
        .collect();
    poly(&pts, true)
}

fn adj(list: &[(String, i64)], name: &str, default: f32) -> f32 {
    list.iter().find(|(n, _)| n == name).map(|(_, v)| *v as f32).unwrap_or(default)
}

/// A rotated/mirrored copy of a right-pointing arrow polygon.
fn arrow(dir: char, w: f32, h: f32, a1: f32, a2: f32) -> Option<Path> {
    // Work in a frame where the arrow points right, then map to the box.
    let (lw, lh) = if matches!(dir, 'u' | 'd') { (h, w) } else { (w, h) };
    let ss = lw.min(lh);
    let shaft = lh * a1 / 100_000.0;
    let (y1, y2) = ((lh - shaft) / 2.0, (lh + shaft) / 2.0);
    let x1 = (lw - ss * a2 / 100_000.0).max(0.0);
    let pts = [(0.0, y1), (x1, y1), (x1, 0.0), (lw, lh / 2.0), (x1, lh), (x1, y2), (0.0, y2)];
    let mapped: Vec<(f32, f32)> = pts
        .iter()
        .map(|&(x, y)| match dir {
            'r' => (x, y),
            'l' => (w - x, y),
            'd' => (y, x),
            _ => (y, h - x), // up
        })
        .collect();
    poly(&mapped, true)
}

/// Path for a preset shape inside a `w` × `h` box.
pub fn preset(name: &str, adjs: &[(String, i64)], w: f32, h: f32) -> Option<Path> {
    let ss = w.min(h);
    let a = |n: &str, d: f32| adj(adjs, n, d);
    match name {
        "rect" | "flowChartProcess" | "flowChartPredefinedProcess" | "flowChartInternalStorage" | "frame" | "textBox"
        | "wedgeRectCallout" | "borderCallout1" | "snip1Rect" | "snip2SameRect" | "snipRoundRect" => {
            Some(PathBuilder::from_rect(Rect::from_xywh(0.0, 0.0, w.max(0.01), h.max(0.01))?))
        }
        "roundRect" | "wedgeRoundRectCallout" => rounded_rect(w, h, ss * a("adj", 16667.0) / 100_000.0),
        "round1Rect" | "round2SameRect" | "round2DiagRect" => rounded_rect(w, h, ss * a("adj1", 16667.0) / 100_000.0),
        "flowChartTerminator" => rounded_rect(w, h, ss / 2.0),
        "ellipse" | "flowChartConnector" | "flowChartSummingJunction" | "flowChartOr" => {
            PathBuilder::from_oval(Rect::from_xywh(0.0, 0.0, w.max(0.01), h.max(0.01))?)
        }
        "triangle" => poly(&[(0.0, h), (w * a("adj", 50000.0) / 100_000.0, 0.0), (w, h)], true),
        "rtTriangle" => poly(&[(0.0, 0.0), (0.0, h), (w, h)], true),
        "diamond" | "flowChartDecision" => poly(&[(w / 2.0, 0.0), (w, h / 2.0), (w / 2.0, h), (0.0, h / 2.0)], true),
        "parallelogram" | "flowChartInputOutput" => {
            let x = ss * a("adj", 25000.0) / 100_000.0;
            poly(&[(0.0, h), (x, 0.0), (w, 0.0), (w - x, h)], true)
        }
        "trapezoid" => {
            let x = ss * a("adj", 25000.0) / 100_000.0;
            poly(&[(0.0, h), (x, 0.0), (w - x, 0.0), (w, h)], true)
        }
        "flowChartManualOperation" => {
            let x = w * 0.2;
            poly(&[(0.0, 0.0), (w, 0.0), (w - x, h), (x, h)], true)
        }
        "pentagon" => regular_polygon(5, w, h),
        "hexagon" => {
            let x = ss * a("adj", 25000.0) / 100_000.0;
            poly(&[(x, 0.0), (w - x, 0.0), (w, h / 2.0), (w - x, h), (x, h), (0.0, h / 2.0)], true)
        }
        "heptagon" => regular_polygon(7, w, h),
        "octagon" => {
            let x = ss * a("adj", 29289.0) / 100_000.0;
            poly(&[(0.0, x), (x, 0.0), (w - x, 0.0), (w, x), (w, h - x), (w - x, h), (x, h), (0.0, h - x)], true)
        }
        "decagon" => regular_polygon(10, w, h),
        "dodecagon" => regular_polygon(12, w, h),
        "star4" => star(4, a("adj", 12500.0) / 50_000.0, w, h),
        "star5" => star(5, 0.382, w, h),
        "star6" => star(6, a("adj", 28868.0) / 50_000.0, w, h),
        "star7" => star(7, 0.4, w, h),
        "star8" => star(8, a("adj", 37500.0) / 50_000.0, w, h),
        "star10" => star(10, 0.5, w, h),
        "star12" => star(12, a("adj", 37500.0) / 50_000.0, w, h),
        "star16" => star(16, a("adj", 37500.0) / 50_000.0, w, h),
        "star24" => star(24, a("adj", 37500.0) / 50_000.0, w, h),
        "rightArrow" => arrow('r', w, h, a("adj1", 50000.0), a("adj2", 50000.0)),
        "leftArrow" => arrow('l', w, h, a("adj1", 50000.0), a("adj2", 50000.0)),
        "upArrow" => arrow('u', w, h, a("adj1", 50000.0), a("adj2", 50000.0)),
        "downArrow" => arrow('d', w, h, a("adj1", 50000.0), a("adj2", 50000.0)),
        "plus" | "mathPlus" => {
            let x = ss * a("adj", 25000.0) / 100_000.0;
            poly(
                &[(x, 0.0), (w - x, 0.0), (w - x, x), (w, x), (w, h - x), (w - x, h - x), (w - x, h), (x, h), (x, h - x), (0.0, h - x), (0.0, x), (x, x)],
                true,
            )
        }
        "chevron" => {
            let x = ss * a("adj", 50000.0) / 100_000.0;
            poly(&[(0.0, 0.0), (w - x, 0.0), (w, h / 2.0), (w - x, h), (0.0, h), (x, h / 2.0)], true)
        }
        "homePlate" => {
            let x = ss * a("adj", 50000.0) / 100_000.0;
            poly(&[(0.0, 0.0), (w - x, 0.0), (w, h / 2.0), (w - x, h), (0.0, h)], true)
        }
        "heart" => {
            let mut pb = PathBuilder::new();
            pb.move_to(w / 2.0, h);
            pb.cubic_to(-w * 0.15, h * 0.55, w * 0.05, -h * 0.1, w / 2.0, h * 0.28);
            pb.cubic_to(w * 0.95, -h * 0.1, w * 1.15, h * 0.55, w / 2.0, h);
            pb.close();
            pb.finish()
        }
        "line" | "straightConnector1" => poly(&[(0.0, 0.0), (w, h)], false),
        "bentConnector2" => poly(&[(0.0, 0.0), (w, 0.0), (w, h)], false),
        "bentConnector3" => {
            let x = w * a("adj1", 50000.0) / 100_000.0;
            poly(&[(0.0, 0.0), (x, 0.0), (x, h), (w, h)], false)
        }
        "curvedConnector3" => {
            let mut pb = PathBuilder::new();
            pb.move_to(0.0, 0.0);
            pb.cubic_to(w / 2.0, 0.0, w / 2.0, h, w, h);
            pb.finish()
        }
        "arc" | "blockArc" => {
            let mut pb = PathBuilder::new();
            pb.push_oval(Rect::from_xywh(0.0, 0.0, w.max(0.01), h.max(0.01))?);
            pb.finish()
        }
        "donut" => {
            let t = ss * a("adj", 25000.0) / 100_000.0;
            let mut pb = PathBuilder::new();
            pb.push_oval(Rect::from_xywh(0.0, 0.0, w.max(0.01), h.max(0.01))?);
            pb.push_oval(Rect::from_xywh(t, t, (w - 2.0 * t).max(0.01), (h - 2.0 * t).max(0.01))?);
            pb.finish()
        }
        other => {
            tracing::debug!("preset geometry `{other}` not drawn natively; using a rectangle");
            Some(PathBuilder::from_rect(Rect::from_xywh(0.0, 0.0, w.max(0.01), h.max(0.01))?))
        }
    }
}

/// Whether a preset is an open line-like shape (stroke only, no fill).
pub fn is_open(name: &str) -> bool {
    matches!(name, "line" | "straightConnector1" | "bentConnector2" | "bentConnector3" | "bentConnector4" | "bentConnector5" | "curvedConnector2" | "curvedConnector3" | "curvedConnector4" | "curvedConnector5" | "arc")
}

// ------------------------------------------------------------ custom paths --

struct Env {
    vars: HashMap<String, f64>,
}

impl Env {
    fn val(&self, tok: &str) -> f64 {
        tok.parse::<f64>().ok().or_else(|| self.vars.get(tok).copied()).unwrap_or(0.0)
    }

    fn eval(&self, fmla: &str) -> f64 {
        let t: Vec<&str> = fmla.split_whitespace().collect();
        let Some(op) = t.first() else { return 0.0 };
        let v = |i: usize| t.get(i).map(|s| self.val(s)).unwrap_or(0.0);
        let ang = |x: f64| (x / 60_000.0).to_radians();
        match *op {
            "val" => v(1),
            "*/" => {
                if v(3) == 0.0 {
                    0.0
                } else {
                    v(1) * v(2) / v(3)
                }
            }
            "+-" => v(1) + v(2) - v(3),
            "+/" => {
                if v(3) == 0.0 {
                    0.0
                } else {
                    (v(1) + v(2)) / v(3)
                }
            }
            "?:" => {
                if v(1) > 0.0 {
                    v(2)
                } else {
                    v(3)
                }
            }
            "abs" => v(1).abs(),
            "at2" => v(2).atan2(v(1)).to_degrees() * 60_000.0,
            "cat2" => v(1) * v(3).atan2(v(2)).cos(),
            "sat2" => v(1) * v(3).atan2(v(2)).sin(),
            "cos" => v(1) * ang(v(2)).cos(),
            "sin" => v(1) * ang(v(2)).sin(),
            "tan" => v(1) * ang(v(2)).tan(),
            "mod" => (v(1) * v(1) + v(2) * v(2) + v(3) * v(3)).sqrt(),
            "pin" => v(2).clamp(v(1).min(v(3)), v(3).max(v(1))),
            "sqrt" => v(1).max(0.0).sqrt(),
            "max" => v(1).max(v(2)),
            "min" => v(1).min(v(2)),
            _ => 0.0,
        }
    }
}

/// Build a path from custom geometry. `w`/`h` are the target size in pixels
/// and `emu_w`/`emu_h` the shape size in EMU (the guide coordinate space).
pub fn custom(g: &CustomGeometry, w: f32, h: f32, emu_w: f64, emu_h: f64) -> Option<Path> {
    let mut vars: HashMap<String, f64> = HashMap::new();
    let (ew, eh) = (emu_w, emu_h);
    for (k, v) in [
        ("w", ew), ("h", eh), ("l", 0.0), ("t", 0.0), ("r", ew), ("b", eh), ("hc", ew / 2.0), ("vc", eh / 2.0),
        ("wd2", ew / 2.0), ("hd2", eh / 2.0), ("wd3", ew / 3.0), ("hd3", eh / 3.0), ("wd4", ew / 4.0), ("hd4", eh / 4.0),
        ("wd5", ew / 5.0), ("hd5", eh / 5.0), ("wd6", ew / 6.0), ("hd6", eh / 6.0), ("wd8", ew / 8.0), ("hd8", eh / 8.0),
        ("wd10", ew / 10.0), ("hd10", eh / 10.0), ("ss", ew.min(eh)), ("ls", ew.max(eh)), ("ssd2", ew.min(eh) / 2.0),
        ("ssd4", ew.min(eh) / 4.0), ("ssd8", ew.min(eh) / 8.0), ("cd2", 10_800_000.0), ("cd4", 5_400_000.0),
        ("cd8", 2_700_000.0), ("3cd4", 16_200_000.0), ("3cd8", 8_100_000.0), ("5cd8", 13_500_000.0), ("7cd8", 18_900_000.0),
    ] {
        vars.insert(k.to_string(), v);
    }
    let mut env = Env { vars };
    for (n, f) in g.av.iter().chain(g.gd.iter()) {
        let v = env.eval(f);
        env.vars.insert(n.clone(), v);
    }
    let mut pb = PathBuilder::new();
    let mut any = false;
    for path in &g.paths {
        let pw = path.w.filter(|v| *v > 0).map(|v| v as f64).unwrap_or(ew.max(1.0));
        let ph = path.h.filter(|v| *v > 0).map(|v| v as f64).unwrap_or(eh.max(1.0));
        let (sx, sy) = (w as f64 / pw, h as f64 / ph);
        let pt = |x: &str, y: &str| ((env.val(x) * sx) as f32, (env.val(y) * sy) as f32);
        let mut cur = (0.0f32, 0.0f32);
        for c in &path.cmds {
            match c {
                PathCmd::MoveTo(x, y) => {
                    cur = pt(x, y);
                    pb.move_to(cur.0, cur.1);
                    any = true;
                }
                PathCmd::LineTo(x, y) => {
                    cur = pt(x, y);
                    pb.line_to(cur.0, cur.1);
                }
                PathCmd::QuadTo([a, b]) => {
                    let (p1, p2) = (pt(&a.0, &a.1), pt(&b.0, &b.1));
                    pb.quad_to(p1.0, p1.1, p2.0, p2.1);
                    cur = p2;
                }
                PathCmd::CubicTo([a, b, c]) => {
                    let (p1, p2, p3) = (pt(&a.0, &a.1), pt(&b.0, &b.1), pt(&c.0, &c.1));
                    pb.cubic_to(p1.0, p1.1, p2.0, p2.1, p3.0, p3.1);
                    cur = p3;
                }
                PathCmd::ArcTo { wr, hr, st, sw } => {
                    let (rx, ry) = ((env.val(wr) * sx) as f32, (env.val(hr) * sy) as f32);
                    let (st, sw) = ((env.val(st) / 60_000.0).to_radians() as f32, (env.val(sw) / 60_000.0).to_radians() as f32);
                    let (cx, cy) = (cur.0 - rx * st.cos(), cur.1 - ry * st.sin());
                    let segs = ((sw.abs() / std::f32::consts::FRAC_PI_2).ceil() as usize).max(1);
                    let step = sw / segs as f32;
                    let kappa = 4.0 / 3.0 * (step / 4.0).tan();
                    let mut a0 = st;
                    for _ in 0..segs {
                        let a1 = a0 + step;
                        let (p0x, p0y) = (cx + rx * a0.cos(), cy + ry * a0.sin());
                        let (p3x, p3y) = (cx + rx * a1.cos(), cy + ry * a1.sin());
                        pb.cubic_to(
                            p0x - kappa * rx * a0.sin(),
                            p0y + kappa * ry * a0.cos(),
                            p3x + kappa * rx * a1.sin(),
                            p3y - kappa * ry * a1.cos(),
                            p3x,
                            p3y,
                        );
                        a0 = a1;
                        cur = (p3x, p3y);
                    }
                }
                PathCmd::Close => pb.close(),
            }
        }
    }
    if any {
        pb.finish()
    } else {
        None
    }
}

/// Build the outline path for a geometry.
pub fn path_for(g: &Geometry, w: f32, h: f32, emu_w: f64, emu_h: f64) -> Option<Path> {
    match g {
        Geometry::Preset { name, adj } => preset(name, adj, w, h),
        Geometry::Custom(c) => custom(c, w, h, emu_w, emu_h),
    }
}
