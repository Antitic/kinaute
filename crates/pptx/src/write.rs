//! Model → `.pptx`. Unchanged objects are written back from their original
//! XML; changed ones are patched in place; new ones are generated. Parts
//! Kinaute does not model are copied byte for byte.

use crate::package::*;
use crate::read::{self, Rd};
use kinaute_model::*;
use kinaute_xml::{Document as Xml, Element, Node};
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

const NS_A: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
const NS_R: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const NS_P: &str = "http://schemas.openxmlformats.org/presentationml/2006/main";
const CT_SLIDE: &str = "application/vnd.openxmlformats-officedocument.presentationml.slide+xml";

const SPPR_ORDER: &[&str] = &[
    "xfrm",
    "custGeom",
    "prstGeom",
    "noFill",
    "solidFill",
    "gradFill",
    "blipFill",
    "pattFill",
    "grpFill",
    "ln",
    "effectLst",
    "effectDag",
    "scene3d",
    "sp3d",
    "extLst",
];
const FILLS: &[&str] = &["noFill", "solidFill", "gradFill", "blipFill", "pattFill", "grpFill"];
const SP_ORDER: &[&str] = &["nvSpPr", "nvCxnSpPr", "nvPicPr", "spPr", "blipFill", "style", "txBody", "extLst"];
const RPR_ORDER: &[&str] = &[
    "ln",
    "noFill",
    "solidFill",
    "gradFill",
    "blipFill",
    "pattFill",
    "grpFill",
    "effectLst",
    "effectDag",
    "highlight",
    "uLnTx",
    "uLn",
    "uFillTx",
    "uFill",
    "latin",
    "ea",
    "cs",
    "sym",
    "hlinkClick",
    "hlinkMouseOver",
    "rtl",
    "extLst",
];
const PPR_ORDER: &[&str] = &[
    "lnSpc",
    "spcBef",
    "spcAft",
    "buClrTx",
    "buClr",
    "buSzTx",
    "buSzPct",
    "buSzPts",
    "buFontTx",
    "buFont",
    "buNone",
    "buAutoNum",
    "buChar",
    "buBlip",
    "tabLst",
    "defRPr",
    "extLst",
];
const SLD_ORDER: &[&str] = &["cSld", "clrMapOvr", "transition", "timing", "extLst"];

/// Per-part writing state: the slide's relationships, extended as needed.
struct Wr {
    rels: Vec<Rel>,
    dir: String,
}

impl Wr {
    fn embed(&mut self, media: &str) -> String {
        if let Some(r) = self.rels.iter().find(|r| !r.external && r.target == media) {
            return r.id.clone();
        }
        let id = self.fresh_id();
        self.rels.push(Rel { id: id.clone(), ty: REL_IMAGE.into(), target: media.to_string(), external: false });
        id
    }

    fn fresh_id(&self) -> String {
        (1..)
            .map(|n| format!("rId{n}"))
            .find(|id| !self.rels.iter().any(|r| &r.id == id))
            .unwrap_or_else(|| "rId1".into())
    }
}

// ------------------------------------------------------------- generators --

pub fn gen_color(c: &Color) -> Element {
    let mut e = match &c.base {
        ColorBase::Srgb(v) => Element::new("a:srgbClr").with_attr("val", v.clone()),
        ColorBase::Scheme(v) => Element::new("a:schemeClr").with_attr("val", v.clone()),
        ColorBase::System { name, last } => {
            let mut e = Element::new("a:sysClr").with_attr("val", name.clone());
            if let Some(l) = last {
                e.attrs.push(("lastClr".into(), l.clone()));
            }
            e
        }
        ColorBase::Preset(v) => Element::new("a:prstClr").with_attr("val", v.clone()),
        ColorBase::ScRgb(r, g, b) => Element::new("a:scrgbClr")
            .with_attr("r", r.to_string())
            .with_attr("g", g.to_string())
            .with_attr("b", b.to_string()),
        ColorBase::Hsl(h, s, l) => Element::new("a:hslClr")
            .with_attr("hue", h.to_string())
            .with_attr("sat", s.to_string())
            .with_attr("lum", l.to_string()),
    };
    for (k, v) in &c.mods {
        let m = Element::new(format!("a:{k}"));
        e = e.with_child(if matches!(k.as_str(), "comp" | "inv" | "gray" | "gamma" | "invGamma") {
            m
        } else {
            m.with_attr("val", v.to_string())
        });
    }
    e
}

fn gen_fill(f: &Fill, wr: &mut Wr) -> Element {
    match f {
        Fill::None => Element::new("a:noFill"),
        Fill::Group => Element::new("a:grpFill"),
        Fill::Solid(c) => Element::new("a:solidFill").with_child(gen_color(c)),
        Fill::Gradient(g) => {
            let mut lst = Element::new("a:gsLst");
            for (pos, c) in &g.stops {
                lst = lst.with_child(Element::new("a:gs").with_attr("pos", pos.to_string()).with_child(gen_color(c)));
            }
            let mut e = Element::new("a:gradFill")
                .with_attr("rotWithShape", if g.rot_with_shape { "1" } else { "0" })
                .with_child(lst);
            match &g.kind {
                GradientKind::Linear { angle, scaled } => {
                    e = e.with_child(
                        Element::new("a:lin")
                            .with_attr("ang", angle.to_string())
                            .with_attr("scaled", if *scaled { "1" } else { "0" }),
                    );
                }
                GradientKind::Path { path, focus } => {
                    e = e.with_child(
                        Element::new("a:path").with_attr("path", path.clone()).with_child(
                            Element::new("a:fillToRect")
                                .with_attr("l", focus.0.to_string())
                                .with_attr("t", focus.1.to_string())
                                .with_attr("r", focus.2.to_string())
                                .with_attr("b", focus.3.to_string()),
                        ),
                    );
                }
            }
            e
        }
        Fill::Image(i) => gen_blip_fill("a:blipFill", i, wr),
        Fill::Pattern { preset, fg, bg } => {
            let mut e = Element::new("a:pattFill").with_attr("prst", preset.clone());
            if let Some(c) = fg {
                e = e.with_child(Element::new("a:fgClr").with_child(gen_color(c)));
            }
            if let Some(c) = bg {
                e = e.with_child(Element::new("a:bgClr").with_child(gen_color(c)));
            }
            e
        }
    }
}

fn gen_blip_fill(name: &str, i: &ImageFill, wr: &mut Wr) -> Element {
    let mut blip = Element::new("a:blip");
    if let Some(m) = &i.media {
        let id = wr.embed(m);
        blip.attrs.push(("r:embed".into(), id));
    }
    if i.alpha != 100_000 {
        blip = blip.with_child(Element::new("a:alphaModFix").with_attr("amt", i.alpha.to_string()));
    }
    let mut e = Element::new(name).with_child(blip);
    if i.crop != (0, 0, 0, 0) {
        let mut s = Element::new("a:srcRect");
        for (k, v) in [("l", i.crop.0), ("t", i.crop.1), ("r", i.crop.2), ("b", i.crop.3)] {
            if v != 0 {
                s.attrs.push((k.into(), v.to_string()));
            }
        }
        e = e.with_child(s);
    }
    e.with_child(if i.stretch {
        Element::new("a:stretch").with_child(Element::new("a:fillRect"))
    } else {
        Element::new("a:tile")
    })
}

fn gen_line(l: &Line, wr: &mut Wr, src: Option<&Element>) -> Element {
    let mut e = src.cloned().unwrap_or_else(|| Element::new("a:ln"));
    e.set_opt_attr("w", l.width.map(|w| w.to_string()));
    e.set_opt_attr("cap", l.cap.clone());
    for f in FILLS {
        e.remove_children(f);
    }
    if let Some(f) = &l.fill {
        let fe = gen_fill(f, wr);
        e.put_child_ordered(
            fe,
            &[
                "noFill",
                "solidFill",
                "gradFill",
                "pattFill",
                "prstDash",
                "custDash",
                "round",
                "bevel",
                "miter",
                "headEnd",
                "tailEnd",
                "extLst",
            ],
        );
    }
    let order = [
        "noFill",
        "solidFill",
        "gradFill",
        "pattFill",
        "prstDash",
        "custDash",
        "round",
        "bevel",
        "miter",
        "headEnd",
        "tailEnd",
        "extLst",
    ];
    e.remove_children("prstDash");
    if let Some(d) = &l.dash {
        e.put_child_ordered(Element::new("a:prstDash").with_attr("val", d.clone()), &order);
    }
    if let Some(j) = &l.join {
        for n in ["round", "bevel", "miter"] {
            e.remove_children(n);
        }
        let mut je = Element::new(format!("a:{j}"));
        if j == "miter" {
            je.attrs.push(("lim".into(), "800000".into()));
        }
        e.put_child_ordered(je, &order);
    }
    for (name, v) in [("headEnd", &l.head), ("tailEnd", &l.tail)] {
        e.remove_children(name);
        if let Some(t) = v {
            e.put_child_ordered(Element::new(format!("a:{name}")).with_attr("type", t.clone()), &order);
        }
    }
    e
}

fn gen_xfrm(x: &Xfrm, name: &str) -> Element {
    let mut e = Element::new(name);
    if x.rot != 0 {
        e.attrs.push(("rot".into(), x.rot.to_string()));
    }
    if x.flip_h {
        e.attrs.push(("flipH".into(), "1".into()));
    }
    if x.flip_v {
        e.attrs.push(("flipV".into(), "1".into()));
    }
    e.with_child(Element::new("a:off").with_attr("x", x.x.to_string()).with_attr("y", x.y.to_string()))
        .with_child(Element::new("a:ext").with_attr("cx", x.w.to_string()).with_attr("cy", x.h.to_string()))
}

fn gen_geom(g: &Geometry) -> Option<Element> {
    match g {
        Geometry::Preset { name, adj } => {
            let mut av = Element::new("a:avLst");
            for (n, v) in adj {
                av = av.with_child(
                    Element::new("a:gd").with_attr("name", n.clone()).with_attr("fmla", format!("val {v}")),
                );
            }
            Some(Element::new("a:prstGeom").with_attr("prst", name.clone()).with_child(av))
        }
        // Custom geometry is never edited; the source element is kept.
        Geometry::Custom(_) => None,
    }
}

fn gen_shadow(s: &Shadow) -> Element {
    Element::new("a:effectLst").with_child(
        Element::new("a:outerShdw")
            .with_attr("blurRad", s.blur.to_string())
            .with_attr("dist", s.dist.to_string())
            .with_attr("dir", s.dir.to_string())
            .with_attr("algn", "ctr")
            .with_attr("rotWithShape", "0")
            .with_child(gen_color(&s.color)),
    )
}

fn gen_style(s: &StyleRefs) -> Element {
    let r = |n: &str, (idx, c): &(u32, Option<Color>)| {
        let mut e = Element::new(n).with_attr("idx", idx.to_string());
        if let Some(c) = c {
            e = e.with_child(gen_color(c));
        }
        e
    };
    let mut font = Element::new("a:fontRef").with_attr("idx", s.font.0.clone());
    if let Some(c) = &s.font.1 {
        font = font.with_child(gen_color(c));
    }
    Element::new("p:style")
        .with_child(r("a:lnRef", &s.line))
        .with_child(r("a:fillRef", &s.fill))
        .with_child(r("a:effectRef", &s.effect))
        .with_child(font)
}

// ------------------------------------------------------------------ text ---

fn apply_rpr(e: &mut Element, p: &RunProps, wr: &mut Wr) {
    e.set_opt_attr("sz", p.size.map(|v| v.to_string()));
    e.set_opt_attr("b", p.bold.map(|v| (if v { "1" } else { "0" }).to_string()));
    e.set_opt_attr("i", p.italic.map(|v| (if v { "1" } else { "0" }).to_string()));
    e.set_opt_attr("u", p.underline.clone());
    e.set_opt_attr("strike", p.strike.clone());
    e.set_opt_attr("baseline", p.baseline.map(|v| v.to_string()));
    e.set_opt_attr("spc", p.spacing.map(|v| v.to_string()));
    e.set_opt_attr("cap", p.caps.clone());
    e.set_opt_attr("lang", p.lang.clone());
    for f in FILLS {
        e.remove_children(f);
    }
    if let Some(f) = &p.fill {
        e.put_child_ordered(gen_fill(f, wr), RPR_ORDER);
    }
    e.remove_children("highlight");
    if let Some(c) = &p.highlight {
        e.put_child_ordered(Element::new("a:highlight").with_child(gen_color(c)), RPR_ORDER);
    }
    e.remove_children("latin");
    if let Some(f) = &p.font {
        e.put_child_ordered(Element::new("a:latin").with_attr("typeface", f.clone()), RPR_ORDER);
    }
}

fn apply_ppr(e: &mut Element, level: u8, p: &ParaProps, wr: &mut Wr) {
    e.set_opt_attr("lvl", (level > 0).then(|| level.to_string()));
    e.set_opt_attr("marL", p.mar_l.map(|v| v.to_string()));
    e.set_opt_attr("indent", p.indent.map(|v| v.to_string()));
    e.set_opt_attr("rtl", p.rtl.map(|v| (if v { "1" } else { "0" }).to_string()));
    e.set_opt_attr(
        "algn",
        p.align.map(|a| {
            match a {
                Align::Left => "l",
                Align::Center => "ctr",
                Align::Right => "r",
                Align::Justify => "just",
            }
            .to_string()
        }),
    );
    for (name, sp) in [("lnSpc", &p.line_spacing), ("spcBef", &p.space_before), ("spcAft", &p.space_after)] {
        e.remove_children(name);
        if let Some(sp) = sp {
            let inner = match sp {
                Spacing::Percent(v) => Element::new("a:spcPct").with_attr("val", v.to_string()),
                Spacing::Points(v) => Element::new("a:spcPts").with_attr("val", v.to_string()),
            };
            e.put_child_ordered(Element::new(format!("a:{name}")).with_child(inner), PPR_ORDER);
        }
    }
    if let Some(b) = &p.bullet {
        for n in ["buClr", "buSzPct", "buFont", "buNone", "buAutoNum", "buChar", "buBlip"] {
            e.remove_children(n);
        }
        match b {
            Bullet::None => e.put_child_ordered(Element::new("a:buNone"), PPR_ORDER),
            Bullet::Char { ch, font, color, size_pct } => {
                if let Some(c) = color {
                    e.put_child_ordered(Element::new("a:buClr").with_child(gen_color(c)), PPR_ORDER);
                }
                if let Some(s) = size_pct {
                    e.put_child_ordered(Element::new("a:buSzPct").with_attr("val", s.to_string()), PPR_ORDER);
                }
                if let Some(f) = font {
                    e.put_child_ordered(Element::new("a:buFont").with_attr("typeface", f.clone()), PPR_ORDER);
                }
                e.put_child_ordered(Element::new("a:buChar").with_attr("char", ch.clone()), PPR_ORDER);
            }
            Bullet::AutoNum { scheme, start, color, size_pct } => {
                if let Some(c) = color {
                    e.put_child_ordered(Element::new("a:buClr").with_child(gen_color(c)), PPR_ORDER);
                }
                if let Some(s) = size_pct {
                    e.put_child_ordered(Element::new("a:buSzPct").with_attr("val", s.to_string()), PPR_ORDER);
                }
                let mut a = Element::new("a:buAutoNum").with_attr("type", scheme.clone());
                if *start != 1 {
                    a.attrs.push(("startAt".into(), start.to_string()));
                }
                e.put_child_ordered(a, PPR_ORDER);
            }
        }
    }
    if p.def_run != RunProps::default() {
        let d = e.ensure_child_ordered("a:defRPr", PPR_ORDER);
        apply_rpr(d, &p.def_run, wr);
    }
}

fn gen_run(r: &Run, wr: &mut Wr) -> Element {
    let rd_unchanged = r.source.as_ref().and_then(|s| {
        let rd = Rd { rels: &wr.rels };
        (read::run_of(s, &rd).as_ref() == Some(r)).then(|| (**s).clone())
    });
    if let Some(e) = rd_unchanged {
        return e;
    }
    let mut e = match (&r.kind, &r.source) {
        (RunKind::Text(_), Some(s)) if s.local() == "r" => (**s).clone(),
        (RunKind::Field { .. }, Some(s)) if s.local() == "fld" => (**s).clone(),
        (RunKind::Break, Some(s)) if s.local() == "br" => (**s).clone(),
        (RunKind::Text(_), _) => Element::new("a:r"),
        (RunKind::Break, _) => Element::new("a:br"),
        (RunKind::Field { kind, .. }, _) => Element::new("a:fld")
            .with_attr("id", "{B6F15528-21DE-4FAA-801E-634DDDAF4B2B}")
            .with_attr("type", kind.clone()),
    };
    // Properties.
    let has_props = r.props != RunProps::default();
    if has_props || e.child("rPr").is_some() {
        let rpr = e.ensure_child_ordered("a:rPr", &["rPr", "pPr", "t"]);
        apply_rpr(rpr, &r.props, wr);
        if rpr.attrs.is_empty() && rpr.children.is_empty() && !has_props {
            e.remove_children("rPr");
        }
    }
    match &r.kind {
        RunKind::Text(t) | RunKind::Field { text: t, .. } => {
            e.remove_children("t");
            e.children.push(Node::Element(Element::new("a:t").with_text(t.clone())));
        }
        RunKind::Break => {}
    }
    e
}

fn gen_paragraph(p: &Paragraph, wr: &mut Wr) -> Element {
    let unchanged = p.source.as_ref().and_then(|s| {
        let rd = Rd { rels: &wr.rels };
        (&read::paragraph_of(s, &rd) == p).then(|| (**s).clone())
    });
    if let Some(e) = unchanged {
        return e;
    }
    let mut e = Element::new("a:p");
    let mut ppr = p.source.as_ref().and_then(|s| s.child("pPr").cloned()).unwrap_or_else(|| Element::new("a:pPr"));
    apply_ppr(&mut ppr, p.level, &p.props, wr);
    if !ppr.attrs.is_empty() || !ppr.children.is_empty() {
        e = e.with_child(ppr);
    }
    for r in &p.runs {
        e.children.push(Node::Element(gen_run(r, wr)));
    }
    if let Some(ep) = &p.end_props {
        let mut x = p
            .source
            .as_ref()
            .and_then(|s| s.child("endParaRPr").cloned())
            .unwrap_or_else(|| Element::new("a:endParaRPr"));
        apply_rpr(&mut x, ep, wr);
        e.children.push(Node::Element(x));
    }
    e
}

fn gen_body_pr(b: &BodyProps, src: Option<&Element>) -> Element {
    let mut e = src.cloned().unwrap_or_else(|| Element::new("a:bodyPr"));
    e.set_opt_attr(
        "anchor",
        b.anchor.map(|a| {
            match a {
                Anchor::Top => "t",
                Anchor::Middle => "ctr",
                Anchor::Bottom => "b",
            }
            .to_string()
        }),
    );
    e.set_opt_attr("anchorCtr", b.anchor_ctr.map(|v| (if v { "1" } else { "0" }).to_string()));
    for (name, v) in ["lIns", "tIns", "rIns", "bIns"].iter().zip(b.insets) {
        e.set_opt_attr(name, v.map(|v| v.to_string()));
    }
    e.set_opt_attr("wrap", b.wrap.map(|w| (if w { "square" } else { "none" }).to_string()));
    e.set_opt_attr("rot", b.rot.map(|v| v.to_string()));
    e.set_opt_attr("vert", b.vert.clone());
    e.set_opt_attr("numCol", b.columns.map(|v| v.to_string()));
    if let Some(a) = b.autofit {
        for n in ["noAutofit", "normAutofit", "spAutoFit"] {
            e.remove_children(n);
        }
        let af = match a {
            AutoFit::None => Element::new("a:noAutofit"),
            AutoFit::Shape => Element::new("a:spAutoFit"),
            AutoFit::Normal { font_scale, line_reduction } => {
                let mut n = Element::new("a:normAutofit");
                if font_scale != 100_000 {
                    n.attrs.push(("fontScale".into(), font_scale.to_string()));
                }
                if line_reduction != 0 {
                    n.attrs.push(("lnSpcReduction".into(), line_reduction.to_string()));
                }
                n
            }
        };
        e.put_child_ordered(
            af,
            &["prstTxWarp", "noAutofit", "normAutofit", "spAutoFit", "scene3d", "sp3d", "flatTx", "extLst"],
        );
    }
    e
}

fn gen_lst_style(ls: &ListStyle, wr: &mut Wr) -> Element {
    let mut e = Element::new("a:lstStyle");
    let mut push = |name: String, p: &ParaProps, wr: &mut Wr| {
        let mut pe = Element::new(name);
        apply_ppr(&mut pe, 0, p, wr);
        e.children.push(Node::Element(pe));
    };
    if let Some(d) = &ls.default {
        push("a:defPPr".into(), d, wr);
    }
    for (i, l) in ls.levels.iter().enumerate() {
        if let Some(l) = l {
            push(format!("a:lvl{}pPr", i + 1), l, wr);
        }
    }
    e
}

fn gen_tx_body(name: &str, tb: &TextBody, wr: &mut Wr) -> Element {
    let rd_body = tb.source_body_pr.as_ref().filter(|s| read::body_props_of(s) == tb.body);
    let body_pr = match rd_body {
        Some(s) => (**s).clone(),
        None => gen_body_pr(&tb.body, tb.source_body_pr.as_deref()),
    };
    let mut e = Element::new(name).with_child(body_pr);
    let lst_unchanged =
        tb.source_lst_style.as_ref().filter(|s| read::list_style_of(s, &Rd { rels: &wr.rels }) == tb.list_style);
    match lst_unchanged {
        Some(s) => e.children.push(Node::Element((**s).clone())),
        None => {
            if !tb.list_style.is_empty() || tb.source_lst_style.is_some() {
                let l = gen_lst_style(&tb.list_style, wr);
                e.children.push(Node::Element(l));
            }
        }
    }
    for p in &tb.paragraphs {
        e.children.push(Node::Element(gen_paragraph(p, wr)));
    }
    if tb.paragraphs.is_empty() {
        e.children.push(Node::Element(Element::new("a:p")));
    }
    e
}

// ----------------------------------------------------------------- shapes ---

fn set_cnv_pr(e: &mut Element, nv: &str, s: &Shape) {
    let nv_el = e.ensure_child_ordered(nv, SP_ORDER);
    let pr = nv_el.ensure_child_ordered(
        "p:cNvPr",
        &["cNvPr", "cNvSpPr", "cNvCxnSpPr", "cNvPicPr", "cNvGrpSpPr", "cNvGraphicFramePr", "nvPr"],
    );
    pr.set_attr("id", s.id.to_string());
    pr.set_attr("name", s.name.clone());
    pr.set_opt_attr("descr", s.descr.clone());
    pr.set_opt_attr("hidden", s.hidden.then(|| "1".to_string()));
}

fn new_nv(kind: &str, s: &Shape) -> Element {
    let (nv, cnv) = match kind {
        "sp" => ("p:nvSpPr", Element::new("p:cNvSpPr")),
        "cxnSp" => ("p:nvCxnSpPr", Element::new("p:cNvCxnSpPr")),
        "pic" => (
            "p:nvPicPr",
            Element::new("p:cNvPicPr").with_child(Element::new("a:picLocks").with_attr("noChangeAspect", "1")),
        ),
        "grpSp" => ("p:nvGrpSpPr", Element::new("p:cNvGrpSpPr")),
        _ => ("p:nvGraphicFramePr", Element::new("p:cNvGraphicFramePr")),
    };
    let mut nvpr = Element::new("p:nvPr");
    if let Some(ph) = &s.ph {
        let mut p = Element::new("p:ph");
        if ph.kind != "obj" {
            p.attrs.push(("type".into(), ph.kind.clone()));
        }
        if let Some(i) = ph.idx {
            p.attrs.push(("idx".into(), i.to_string()));
        }
        nvpr = nvpr.with_child(p);
    }
    Element::new(nv)
        .with_child(Element::new("p:cNvPr").with_attr("id", s.id.to_string()).with_attr("name", s.name.clone()))
        .with_child(cnv)
        .with_child(nvpr)
}

fn patch_sp_pr(
    sp_pr: &mut Element,
    xfrm: Option<&Xfrm>,
    geom: Option<&Geometry>,
    fill: Option<&Fill>,
    line: Option<&Line>,
    shadow: Option<&Shadow>,
    wr: &mut Wr,
) {
    if let Some(x) = xfrm {
        let name = if sp_pr.prefix().is_empty() { "xfrm".to_string() } else { format!("{}:xfrm", "a") };
        sp_pr.put_child_ordered(gen_xfrm(x, &name), SPPR_ORDER);
    }
    if let Some(g) = geom.and_then(gen_geom) {
        sp_pr.remove_children("custGeom");
        sp_pr.put_child_ordered(g, SPPR_ORDER);
    }
    for f in FILLS {
        sp_pr.remove_children(f);
    }
    if let Some(f) = fill {
        sp_pr.put_child_ordered(gen_fill(f, wr), SPPR_ORDER);
    }
    sp_pr.remove_children("ln");
    if let Some(l) = line {
        sp_pr.put_child_ordered(gen_line(l, wr, None), SPPR_ORDER);
    }
    sp_pr.remove_children("effectLst");
    if let Some(s) = shadow {
        sp_pr.put_child_ordered(gen_shadow(s), SPPR_ORDER);
    }
}

/// Build the XML for a shape.
fn gen_shape(s: &Shape, wr: &mut Wr) -> Element {
    // Unchanged → verbatim.
    if let Some(src) = &s.source {
        let rd = Rd { rels: &wr.rels };
        if &read::shape_of(src, &rd) == s {
            return (**src).clone();
        }
    }
    match &s.kind {
        ShapeKind::Opaque { .. } => s.source.as_ref().map(|x| (**x).clone()).unwrap_or_else(|| Element::new("p:sp")),
        ShapeKind::Shape(sp) | ShapeKind::Connector(sp) => {
            let tag = if matches!(s.kind, ShapeKind::Connector(_)) { "cxnSp" } else { "sp" };
            let nvn = if tag == "sp" { "nvSpPr" } else { "nvCxnSpPr" };
            let mut e = match &s.source {
                Some(src) if src.local() == tag => (**src).clone(),
                _ => Element::new(format!("p:{tag}")).with_child(new_nv(tag, s)).with_child(Element::new("p:spPr")),
            };
            set_cnv_pr(&mut e, nvn, s);
            if sp.text_box {
                if let Some(c) = e.child_mut(nvn).and_then(|n| n.child_mut("cNvSpPr")) {
                    c.set_attr("txBox", "1");
                }
            }
            let pr = e.ensure_child_ordered("p:spPr", SP_ORDER);
            patch_sp_pr(
                pr,
                s.xfrm.as_ref(),
                sp.geom.as_ref(),
                sp.fill.as_ref(),
                sp.line.as_ref(),
                sp.shadow.as_ref(),
                wr,
            );
            e.remove_children("style");
            if let Some(st) = &sp.style {
                e.put_child_ordered(gen_style(st), SP_ORDER);
            }
            e.remove_children("txBody");
            if let Some(tb) = &sp.text {
                e.put_child_ordered(gen_tx_body("p:txBody", tb, wr), SP_ORDER);
            }
            e
        }
        ShapeKind::Picture(pic) => {
            let mut e = match &s.source {
                Some(src) if src.local() == "pic" => (**src).clone(),
                _ => Element::new("p:pic").with_child(new_nv("pic", s)).with_child(Element::new("p:spPr")),
            };
            set_cnv_pr(&mut e, "nvPicPr", s);
            e.remove_children("blipFill");
            e.put_child_ordered(gen_blip_fill("p:blipFill", &pic.image, wr), SP_ORDER);
            let pr = e.ensure_child_ordered("p:spPr", SP_ORDER);
            patch_sp_pr(pr, s.xfrm.as_ref(), pic.geom.as_ref(), None, pic.line.as_ref(), pic.shadow.as_ref(), wr);
            e
        }
        ShapeKind::Group(g) => {
            let mut e = match &s.source {
                Some(src) if src.local() == "grpSp" => (**src).clone(),
                _ => Element::new("p:grpSp").with_child(new_nv("grpSp", s)).with_child(Element::new("p:grpSpPr")),
            };
            set_cnv_pr(&mut e, "nvGrpSpPr", s);
            if let Some(pr) = e.child_mut("grpSpPr") {
                if let Some(x) = &s.xfrm {
                    let mut xe = gen_xfrm(x, "a:xfrm");
                    xe = xe
                        .with_child(
                            Element::new("a:chOff")
                                .with_attr("x", g.child.x.to_string())
                                .with_attr("y", g.child.y.to_string()),
                        )
                        .with_child(
                            Element::new("a:chExt")
                                .with_attr("cx", g.child.w.to_string())
                                .with_attr("cy", g.child.h.to_string()),
                        );
                    pr.put_child_ordered(
                        xe,
                        &[
                            "xfrm",
                            "noFill",
                            "solidFill",
                            "gradFill",
                            "blipFill",
                            "pattFill",
                            "grpFill",
                            "effectLst",
                            "effectDag",
                            "scene3d",
                            "extLst",
                        ],
                    );
                }
            }
            write_tree_children(&mut e, &g.shapes, wr);
            e
        }
        ShapeKind::Frame(_) => {
            let mut e = s.source.as_ref().map(|x| (**x).clone()).unwrap_or_else(|| Element::new("p:graphicFrame"));
            set_cnv_pr(&mut e, "nvGraphicFramePr", s);
            if let Some(x) = &s.xfrm {
                e.put_child_ordered(gen_xfrm(x, "p:xfrm"), &["nvGraphicFramePr", "xfrm", "graphic", "extLst"]);
            }
            e
        }
    }
}

/// Replace the shape children of a tree element, keeping its header and
/// trailing `extLst`.
fn write_tree_children(tree: &mut Element, shapes: &[Shape], wr: &mut Wr) {
    let mut head = Vec::new();
    let mut tail = Vec::new();
    for n in std::mem::take(&mut tree.children) {
        match &n {
            Node::Element(e) if matches!(e.local(), "nvGrpSpPr" | "grpSpPr") => head.push(n),
            Node::Element(e) if e.local() == "extLst" => tail.push(n),
            Node::Element(_) => {}
            _ => {}
        }
    }
    tree.children = head;
    for s in shapes {
        tree.children.push(Node::Element(gen_shape(s, wr)));
    }
    tree.children.extend(tail);
}

fn gen_transition(t: &Transition) -> Element {
    let spd = if t.duration_ms <= 600 {
        "fast"
    } else if t.duration_ms <= 900 {
        "med"
    } else {
        "slow"
    };
    let mut e = Element::new("p:transition").with_attr("spd", spd);
    if !t.advance_on_click {
        e.attrs.push(("advClick".into(), "0".into()));
    }
    if let Some(ms) = t.advance_after_ms {
        e.attrs.push(("advTm".into(), ms.to_string()));
    }
    if t.kind != "none" {
        let mut k = Element::new(format!("p:{}", t.kind));
        if let Some(d) = &t.dir {
            k.attrs.push((
                if matches!(t.kind.as_str(), "split" | "blinds" | "checker" | "comb" | "randomBar") {
                    "orient"
                } else {
                    "dir"
                }
                .into(),
                d.clone(),
            ));
        }
        e = e.with_child(k);
    }
    e
}

fn gen_background(b: &Background, wr: &mut Wr) -> Element {
    match b {
        Background::Fill(f) => Element::new("p:bg")
            .with_child(Element::new("p:bgPr").with_child(gen_fill(f, wr)).with_child(Element::new("a:effectLst"))),
        Background::Ref { idx, color } => {
            let mut r = Element::new("p:bgRef").with_attr("idx", idx.to_string());
            if let Some(c) = color {
                r = r.with_child(gen_color(c));
            }
            Element::new("p:bg").with_child(r)
        }
    }
}

fn new_slide_root() -> Element {
    Element::new("p:sld")
        .with_attr("xmlns:a", NS_A)
        .with_attr("xmlns:r", NS_R)
        .with_attr("xmlns:p", NS_P)
        .with_child(
            Element::new("p:cSld").with_child(
                Element::new("p:spTree")
                    .with_child(
                        Element::new("p:nvGrpSpPr")
                            .with_child(Element::new("p:cNvPr").with_attr("id", "1").with_attr("name", ""))
                            .with_child(Element::new("p:cNvGrpSpPr"))
                            .with_child(Element::new("p:nvPr")),
                    )
                    .with_child(
                        Element::new("p:grpSpPr").with_child(
                            gen_xfrm(&Xfrm::default(), "a:xfrm")
                                .with_child(Element::new("a:chOff").with_attr("x", "0").with_attr("y", "0"))
                                .with_child(Element::new("a:chExt").with_attr("cx", "0").with_attr("cy", "0")),
                        ),
                    ),
            ),
        )
        .with_child(Element::new("p:clrMapOvr").with_child(Element::new("a:masterClrMapping")))
}

fn write_slide(slide: &Slide, wr: &mut Wr) -> Element {
    let mut root = slide.source.as_ref().map(|s| (**s).clone()).unwrap_or_else(new_slide_root);
    if slide.hidden {
        root.set_attr("show", "0");
    } else {
        root.remove_attr("show");
    }
    if !slide.show_master_shapes {
        root.set_attr("showMasterSp", "0");
    } else {
        root.remove_attr("showMasterSp");
    }
    let rd_bg = |wr: &Wr| root.child("cSld").and_then(|c| read::background_of(c, &Rd { rels: &wr.rels }));
    let bg_changed = rd_bg(wr) != slide.background;
    if let Some(csld) = root.child_mut("cSld") {
        csld.set_opt_attr("name", slide.name.clone());
        if bg_changed {
            csld.remove_children("bg");
            if let Some(b) = &slide.background {
                let bg = gen_background(b, wr);
                csld.children.insert(0, Node::Element(bg));
            }
        }
        if let Some(tree) = csld.child_mut("spTree") {
            write_tree_children(tree, &slide.shapes, wr);
        }
    }
    // Transition: rewrite only if it changed.
    if read::transition_of(&root) != slide.transition {
        root.remove_children("transition");
        root.remove_children("AlternateContent");
        if let Some(t) = &slide.transition {
            root.put_child_ordered(gen_transition(t), SLD_ORDER);
        }
    }
    root
}

// ------------------------------------------------------------------ save ---

fn media_ext_default(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "svg" => "image/svg+xml",
        "webp" => "image/webp",
        _ => return None,
    })
}

/// Serialise a presentation back to `.pptx` bytes.
pub fn save(pres: &Presentation, original: &Package) -> Result<Vec<u8>, PptxError> {
    let mut pkg = original.clone();
    let root_rels = pkg.rels_of("");
    let main = root_rels
        .iter()
        .find(|r| r.ty == REL_OFFICE_DOC)
        .map(|r| r.target.clone())
        .ok_or(PptxError::NotPresentation("no main document relationship"))?;
    let mut pxml = pkg.xml(&main)?;
    let mut prels = pkg.rels_of(&main);
    let mut ctypes = pkg.xml("[Content_Types].xml")?;

    // Existing slide list: part -> (numeric id, rId).
    let mut ids: HashMap<String, (u64, String)> = HashMap::new();
    let mut max_id = 255u64;
    if let Some(l) = pxml.root.child("sldIdLst") {
        for s in l.elements() {
            let id = s.attr("id").and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
            max_id = max_id.max(id);
            if let Some(rid) = s.attr_ns("id") {
                if let Some(rel) = prels.iter().find(|r| r.id == rid) {
                    ids.insert(rel.target.clone(), (id, rid.to_string()));
                }
            }
        }
    }

    let used_parts: BTreeSet<String> = pres.slides.iter().filter_map(|s| s.part.clone()).collect();
    // Remove slides that no longer exist.
    let old_slide_rels: Vec<Rel> = prels.iter().filter(|r| r.ty == REL_SLIDE).cloned().collect();
    for r in &old_slide_rels {
        if !used_parts.contains(&r.target) {
            for sr in pkg.rels_of(&r.target) {
                if sr.ty == REL_NOTES && !sr.external {
                    pkg.remove(&sr.target);
                    pkg.remove(&rels_part_name(&sr.target));
                    remove_override(&mut ctypes, &sr.target);
                }
            }
            pkg.remove(&r.target);
            pkg.remove(&rels_part_name(&r.target));
            remove_override(&mut ctypes, &r.target);
            prels.retain(|x| x.id != r.id);
            ids.remove(&r.target);
        }
    }

    let mut next_n = 1;
    let mut alloc = |pkg: &Package| -> String {
        loop {
            let name = format!("ppt/slides/slide{next_n}.xml");
            next_n += 1;
            if !pkg.contains(&name) {
                return name;
            }
        }
    };

    let mut sld_ids = Element::new("p:sldIdLst");
    let mut new_media: BTreeSet<String> = BTreeSet::new();
    for slide in &pres.slides {
        let part = match &slide.part {
            Some(p) => p.clone(),
            None => alloc(&pkg),
        };
        // Relationships: keep the slide's own, retarget the layout.
        let mut rels = slide.rels.clone();
        let layout_part = pres.layouts.get(slide.layout).map(|l| l.part.clone());
        if let Some(lp) = &layout_part {
            match rels.iter_mut().find(|r| r.ty == REL_LAYOUT) {
                Some(r) => r.target = lp.clone(),
                None => {
                    let id = (1..)
                        .map(|n| format!("rId{n}"))
                        .find(|id| !rels.iter().any(|r| &r.id == id))
                        .unwrap_or_default();
                    rels.push(Rel { id, ty: REL_LAYOUT.into(), target: lp.clone(), external: false });
                }
            }
        }
        // A duplicated slide must not share the notes part of the original.
        if slide.part.is_none() {
            rels.retain(|r| r.ty != REL_NOTES);
        }
        let mut wr = Wr { rels, dir: dir_of(&part) };
        let root = write_slide(slide, &mut wr);
        pkg.set_xml(&part, &Xml::new(root));
        for r in &wr.rels {
            if r.ty == REL_IMAGE && !r.external {
                new_media.insert(r.target.clone());
            }
        }
        let _ = &wr.dir;
        pkg.set_rels(&part, &wr.rels);

        let (id, rid) = match ids.get(&part) {
            Some(x) => x.clone(),
            None => {
                max_id += 1;
                let rid =
                    (1..).map(|n| format!("rId{n}")).find(|id| !prels.iter().any(|r| &r.id == id)).unwrap_or_default();
                prels.push(Rel { id: rid.clone(), ty: REL_SLIDE.into(), target: part.clone(), external: false });
                (max_id, rid)
            }
        };
        sld_ids = sld_ids.with_child(Element::new("p:sldId").with_attr("id", id.to_string()).with_attr("r:id", rid));
        add_override(&mut ctypes, &part, CT_SLIDE);
    }

    // Presentation: slide list and size.
    pxml.root.put_child_ordered(
        sld_ids,
        &["sldMasterIdLst", "notesMasterIdLst", "handoutMasterIdLst", "sldIdLst", "sldSz", "notesSz"],
    );
    if let Some(sz) = pxml.root.child_mut("sldSz") {
        sz.set_attr("cx", pres.slide_w.to_string());
        sz.set_attr("cy", pres.slide_h.to_string());
    } else {
        pxml.root.put_child_ordered(
            Element::new("p:sldSz").with_attr("cx", pres.slide_w.to_string()).with_attr("cy", pres.slide_h.to_string()),
            &["sldMasterIdLst", "notesMasterIdLst", "handoutMasterIdLst", "sldIdLst", "sldSz", "notesSz"],
        );
    }
    pkg.set_xml(&main, &pxml);
    pkg.set_rels(&main, &prels);

    // Media added in the editor.
    for (name, data) in pres.media.iter() {
        if new_media.contains(name) && !pkg.contains(name) {
            pkg.set(name, data.clone());
        }
        if let Some(ext) = name.rsplit('.').next().map(str::to_ascii_lowercase) {
            if let Some(ct) = media_ext_default(&ext) {
                add_default(&mut ctypes, &ext, ct);
            }
        }
    }
    pkg.set_xml("[Content_Types].xml", &ctypes);
    pkg.write()
}

fn remove_override(ct: &mut Xml, part: &str) {
    let pn = format!("/{part}");
    ct.root.children.retain(
        |n| !matches!(n, Node::Element(e) if e.local() == "Override" && e.attr("PartName") == Some(pn.as_str())),
    );
}

fn add_override(ct: &mut Xml, part: &str, ty: &str) {
    let pn = format!("/{part}");
    if ct.root.elements().any(|e| e.local() == "Override" && e.attr("PartName") == Some(pn.as_str())) {
        return;
    }
    ct.root
        .children
        .push(Node::Element(Element::new("Override").with_attr("PartName", pn).with_attr("ContentType", ty)));
}

fn add_default(ct: &mut Xml, ext: &str, ty: &str) {
    if ct
        .root
        .elements()
        .any(|e| e.local() == "Default" && e.attr("Extension").map(|x| x.eq_ignore_ascii_case(ext)).unwrap_or(false))
    {
        return;
    }
    // Defaults must come before Overrides by convention.
    ct.root
        .children
        .insert(0, Node::Element(Element::new("Default").with_attr("Extension", ext).with_attr("ContentType", ty)));
}

/// Convenience: media bytes for a part name.
pub fn media_arc(pres: &Presentation, name: &str) -> Option<Arc<Vec<u8>>> {
    pres.media.get(name).cloned()
}
