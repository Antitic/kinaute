//! `.pptx` → model. Parsing is tolerant: anything unknown or malformed is
//! skipped (and reported) rather than failing the whole file, and every
//! parsed object keeps its source XML so the writer can carry it through.

use crate::package::*;
use kinaute_model::color::{ClrMap, ColorScheme};
use kinaute_model::*;
use kinaute_xml::Element;
use std::collections::BTreeMap;
use std::sync::Arc;

/// Relationship lookup for the part being parsed.
pub struct Rd<'a> {
    pub rels: &'a [Rel],
}

impl Rd<'_> {
    pub fn target(&self, id: &str) -> Option<String> {
        self.rels.iter().find(|r| r.id == id && !r.external).map(|r| r.target.clone())
    }
}

// ---------------------------------------------------------------- basics --

fn num(e: &Element, name: &str) -> Option<i64> {
    e.attr(name).and_then(|v| v.trim().parse::<f64>().ok()).map(|v| v as i64)
}

fn num32(e: &Element, name: &str) -> Option<i32> {
    num(e, name).map(|v| v.clamp(i32::MIN as i64, i32::MAX as i64) as i32)
}

fn flag(e: &Element, name: &str) -> Option<bool> {
    e.attr(name).map(|v| matches!(v.trim(), "1" | "true" | "on"))
}

pub fn color_of(el: &Element) -> Option<Color> {
    let val = || el.attr("val").map(str::to_string);
    let base = match el.local() {
        "srgbClr" => ColorBase::Srgb(val()?.to_uppercase()),
        "schemeClr" => ColorBase::Scheme(val()?),
        "sysClr" => ColorBase::System { name: val()?, last: el.attr("lastClr").map(str::to_string) },
        "prstClr" => ColorBase::Preset(val()?),
        "scrgbClr" => ColorBase::ScRgb(num32(el, "r")?, num32(el, "g")?, num32(el, "b")?),
        "hslClr" => ColorBase::Hsl(num32(el, "hue")?, num32(el, "sat")?, num32(el, "lum")?),
        _ => return None,
    };
    let mods = el.elements().map(|m| (m.local().to_string(), num32(m, "val").unwrap_or(0))).collect();
    Some(Color { base, mods })
}

pub fn color_child(el: &Element) -> Option<Color> {
    el.elements().find_map(color_of)
}

pub fn xfrm_of(el: &Element) -> Xfrm {
    let off = el.child("off");
    let ext = el.child("ext");
    Xfrm {
        x: off.and_then(|o| num(o, "x")).unwrap_or(0),
        y: off.and_then(|o| num(o, "y")).unwrap_or(0),
        w: ext.and_then(|o| num(o, "cx")).unwrap_or(0).max(0),
        h: ext.and_then(|o| num(o, "cy")).unwrap_or(0).max(0),
        rot: num32(el, "rot").unwrap_or(0),
        flip_h: flag(el, "flipH").unwrap_or(false),
        flip_v: flag(el, "flipV").unwrap_or(false),
    }
}

pub fn child_xfrm_of(el: &Element) -> Xfrm {
    let off = el.child("chOff");
    let ext = el.child("chExt");
    Xfrm {
        x: off.and_then(|o| num(o, "x")).unwrap_or(0),
        y: off.and_then(|o| num(o, "y")).unwrap_or(0),
        w: ext.and_then(|o| num(o, "cx")).unwrap_or(0),
        h: ext.and_then(|o| num(o, "cy")).unwrap_or(0),
        ..Default::default()
    }
}

// ----------------------------------------------------------------- fills --

pub fn fill_of(el: &Element, rd: &Rd) -> Option<Fill> {
    match el.local() {
        "noFill" => Some(Fill::None),
        "solidFill" => color_child(el).map(Fill::Solid),
        "grpFill" => Some(Fill::Group),
        "gradFill" => {
            let stops: Vec<(i32, Color)> = el
                .path(&["gsLst"])
                .map(|l| l.children_named("gs").filter_map(|g| Some((num32(g, "pos")?, color_child(g)?))).collect())
                .unwrap_or_default();
            if stops.is_empty() {
                return None;
            }
            let kind = if let Some(p) = el.child("path") {
                let f = p.child("fillToRect");
                let g = |n| f.and_then(|f| num32(f, n)).unwrap_or(0);
                GradientKind::Path {
                    path: p.attr("path").unwrap_or("circle").to_string(),
                    focus: (g("l"), g("t"), g("r"), g("b")),
                }
            } else {
                let lin = el.child("lin");
                GradientKind::Linear {
                    angle: lin.and_then(|l| num32(l, "ang")).unwrap_or(0),
                    scaled: lin.and_then(|l| flag(l, "scaled")).unwrap_or(false),
                }
            };
            Some(Fill::Gradient(Gradient { stops, kind, rot_with_shape: flag(el, "rotWithShape").unwrap_or(true) }))
        }
        "blipFill" => Some(Fill::Image(image_of(el, rd))),
        "pattFill" => Some(Fill::Pattern {
            preset: el.attr("prst").unwrap_or("pct50").to_string(),
            fg: el.child("fgClr").and_then(color_child),
            bg: el.child("bgClr").and_then(color_child),
        }),
        _ => None,
    }
}

pub fn image_of(el: &Element, rd: &Rd) -> ImageFill {
    let blip = el.child("blip");
    let media = blip.and_then(|b| b.attr_local("embed").or_else(|| b.attr_local("link"))).and_then(|id| rd.target(id));
    let src = el.child("srcRect");
    let c = |n| src.and_then(|s| num32(s, n)).unwrap_or(0);
    let alpha = blip
        .and_then(|b| b.child("alphaModFix"))
        .and_then(|a| num32(a, "amt"))
        .unwrap_or(100_000);
    ImageFill { media, crop: (c("l"), c("t"), c("r"), c("b")), stretch: el.child("tile").is_none(), alpha }
}

pub fn fill_child(el: &Element, rd: &Rd) -> Option<Fill> {
    el.elements().find_map(|e| fill_of(e, rd))
}

pub fn line_of(ln: &Element, rd: &Rd) -> Line {
    Line {
        width: num(ln, "w"),
        fill: fill_child(ln, rd),
        dash: ln.child("prstDash").and_then(|d| d.attr("val")).map(str::to_string),
        cap: ln.attr("cap").map(str::to_string),
        join: ["round", "bevel", "miter"].iter().find(|j| ln.child(j).is_some()).map(|j| j.to_string()),
        head: ln.child("headEnd").and_then(|e| e.attr("type")).map(str::to_string),
        tail: ln.child("tailEnd").and_then(|e| e.attr("type")).map(str::to_string),
    }
}

pub fn shadow_of(sp_pr_or_effects: &Element) -> Option<Shadow> {
    let eff = if sp_pr_or_effects.local() == "effectLst" { Some(sp_pr_or_effects) } else { sp_pr_or_effects.child("effectLst") }?;
    let s = eff.child("outerShdw")?;
    Some(Shadow {
        blur: num(s, "blurRad").unwrap_or(0),
        dist: num(s, "dist").unwrap_or(0),
        dir: num32(s, "dir").unwrap_or(0),
        color: color_child(s)?,
    })
}

pub fn geometry_of(sp_pr: &Element) -> Option<Geometry> {
    if let Some(p) = sp_pr.child("prstGeom") {
        let adj = p
            .path(&["avLst"])
            .map(|l| {
                l.children_named("gd")
                    .filter_map(|g| {
                        let name = g.attr("name")?.to_string();
                        let v = g.attr("fmla")?.strip_prefix("val ")?.trim().parse().ok()?;
                        Some((name, v))
                    })
                    .collect()
            })
            .unwrap_or_default();
        return Some(Geometry::Preset { name: p.attr("prst").unwrap_or("rect").to_string(), adj });
    }
    let c = sp_pr.child("custGeom")?;
    let gd = |l: Option<&Element>| -> Vec<(String, String)> {
        l.map(|l| {
            l.children_named("gd")
                .filter_map(|g| Some((g.attr("name")?.to_string(), g.attr("fmla")?.to_string())))
                .collect()
        })
        .unwrap_or_default()
    };
    let pt = |e: &Element| -> Option<(String, String)> {
        let p = e.child("pt")?;
        Some((p.attr("x")?.to_string(), p.attr("y")?.to_string()))
    };
    let mut paths = Vec::new();
    if let Some(pl) = c.child("pathLst") {
        for p in pl.children_named("path") {
            let mut cmds = Vec::new();
            for cmd in p.elements() {
                match cmd.local() {
                    "moveTo" => cmds.extend(pt(cmd).map(|(x, y)| PathCmd::MoveTo(x, y))),
                    "lnTo" => cmds.extend(pt(cmd).map(|(x, y)| PathCmd::LineTo(x, y))),
                    "arcTo" => cmds.push(PathCmd::ArcTo {
                        wr: cmd.attr("wR").unwrap_or("0").into(),
                        hr: cmd.attr("hR").unwrap_or("0").into(),
                        st: cmd.attr("stAng").unwrap_or("0").into(),
                        sw: cmd.attr("swAng").unwrap_or("0").into(),
                    }),
                    "quadBezTo" => {
                        let pts: Vec<_> = cmd.children_named("pt").filter_map(|p| Some((p.attr("x")?.to_string(), p.attr("y")?.to_string()))).collect();
                        if let [a, b] = pts.as_slice() {
                            cmds.push(PathCmd::QuadTo([a.clone(), b.clone()]));
                        }
                    }
                    "cubicBezTo" => {
                        let pts: Vec<_> = cmd.children_named("pt").filter_map(|p| Some((p.attr("x")?.to_string(), p.attr("y")?.to_string()))).collect();
                        if let [a, b, c] = pts.as_slice() {
                            cmds.push(PathCmd::CubicTo([a.clone(), b.clone(), c.clone()]));
                        }
                    }
                    "close" => cmds.push(PathCmd::Close),
                    _ => {}
                }
            }
            paths.push(GeomPath {
                w: num(p, "w"),
                h: num(p, "h"),
                fill: p.attr("fill").unwrap_or("norm").to_string(),
                stroke: flag(p, "stroke").unwrap_or(true),
                cmds,
            });
        }
    }
    Some(Geometry::Custom(CustomGeometry { av: gd(c.child("avLst")), gd: gd(c.child("gdLst")), paths }))
}

fn style_of(st: &Element) -> Option<StyleRefs> {
    let r = |n: &str| -> (u32, Option<Color>) {
        match st.child(n) {
            Some(e) => (num(e, "idx").unwrap_or(0).max(0) as u32, color_child(e)),
            None => (0, None),
        }
    };
    let font = st.child("fontRef");
    Some(StyleRefs {
        line: r("lnRef"),
        fill: r("fillRef"),
        effect: r("effectRef"),
        font: (
            font.and_then(|f| f.attr("idx")).unwrap_or("none").to_string(),
            font.and_then(color_child),
        ),
    })
}

// ------------------------------------------------------------------ text --

pub fn run_props_of(r: &Element, rd: &Rd) -> RunProps {
    RunProps {
        size: num32(r, "sz"),
        bold: flag(r, "b"),
        italic: flag(r, "i"),
        underline: r.attr("u").map(str::to_string),
        strike: r.attr("strike").map(str::to_string),
        fill: fill_child(r, rd),
        font: r.child("latin").and_then(|l| l.attr("typeface")).map(str::to_string),
        baseline: num32(r, "baseline"),
        spacing: num32(r, "spc"),
        caps: r.attr("cap").map(str::to_string),
        lang: r.attr("lang").map(str::to_string),
        highlight: r.child("highlight").and_then(color_child),
        outline: r.child("ln").map(|l| line_of(l, rd)),
        link: r.child("hlinkClick").and_then(|h| h.attr_local("id")).and_then(|id| {
            rd.rels.iter().find(|x| x.id == id).map(|x| x.target.clone())
        }),
    }
}

fn spacing_of(e: &Element) -> Option<Spacing> {
    if let Some(p) = e.child("spcPct") {
        return num32(p, "val").map(Spacing::Percent);
    }
    e.child("spcPts").and_then(|p| num32(p, "val")).map(Spacing::Points)
}

pub fn para_props_of(p: &Element, rd: &Rd) -> ParaProps {
    let bu_color = p.child("buClr").and_then(color_child);
    let bu_size = p.child("buSzPct").and_then(|s| num32(s, "val"));
    let bu_font = p.child("buFont").and_then(|f| f.attr("typeface")).map(str::to_string);
    let bullet = if p.child("buNone").is_some() {
        Some(Bullet::None)
    } else if let Some(c) = p.child("buChar") {
        Some(Bullet::Char { ch: c.attr("char").unwrap_or("•").to_string(), font: bu_font, color: bu_color, size_pct: bu_size })
    } else if let Some(a) = p.child("buAutoNum") {
        Some(Bullet::AutoNum {
            scheme: a.attr("type").unwrap_or("arabicPeriod").to_string(),
            start: num32(a, "startAt").unwrap_or(1),
            color: bu_color,
            size_pct: bu_size,
        })
    } else if p.child("buBlip").is_some() {
        Some(Bullet::Char { ch: "•".into(), font: None, color: bu_color, size_pct: bu_size })
    } else {
        None
    };
    ParaProps {
        align: p.attr("algn").map(|a| match a {
            "ctr" => Align::Center,
            "r" => Align::Right,
            "just" | "dist" | "justLow" | "thaiDist" => Align::Justify,
            _ => Align::Left,
        }),
        mar_l: num(p, "marL"),
        indent: num(p, "indent"),
        line_spacing: p.child("lnSpc").and_then(spacing_of),
        space_before: p.child("spcBef").and_then(spacing_of),
        space_after: p.child("spcAft").and_then(spacing_of),
        bullet,
        rtl: flag(p, "rtl"),
        def_run: p.child("defRPr").map(|d| run_props_of(d, rd)).unwrap_or_default(),
    }
}

pub fn list_style_of(l: &Element, rd: &Rd) -> ListStyle {
    let mut ls = ListStyle::default();
    for e in l.elements() {
        let name = e.local();
        if name == "defPPr" {
            ls.default = Some(para_props_of(e, rd));
        } else if let Some(n) = name.strip_prefix("lvl").and_then(|r| r.strip_suffix("pPr")).and_then(|n| n.parse::<usize>().ok()) {
            if (1..=9).contains(&n) {
                ls.levels[n - 1] = Some(para_props_of(e, rd));
            }
        }
    }
    ls
}

pub fn body_props_of(b: &Element) -> BodyProps {
    let autofit = if let Some(n) = b.child("normAutofit") {
        Some(AutoFit::Normal {
            font_scale: num32(n, "fontScale").unwrap_or(100_000),
            line_reduction: num32(n, "lnSpcReduction").unwrap_or(0),
        })
    } else if b.child("spAutoFit").is_some() {
        Some(AutoFit::Shape)
    } else if b.child("noAutofit").is_some() {
        Some(AutoFit::None)
    } else {
        None
    };
    BodyProps {
        anchor: b.attr("anchor").map(|a| match a {
            "ctr" => Anchor::Middle,
            "b" => Anchor::Bottom,
            _ => Anchor::Top,
        }),
        anchor_ctr: flag(b, "anchorCtr"),
        insets: [num(b, "lIns"), num(b, "tIns"), num(b, "rIns"), num(b, "bIns")],
        wrap: b.attr("wrap").map(|w| w != "none"),
        autofit,
        rot: num32(b, "rot"),
        vert: b.attr("vert").map(str::to_string),
        columns: num(b, "numCol").map(|n| n.max(1) as u32),
    }
}

pub fn run_of(r: &Element, rd: &Rd) -> Option<Run> {
    let props = r.child("rPr").map(|p| run_props_of(p, rd)).unwrap_or_default();
    let kind = match r.local() {
        "r" => RunKind::Text(r.child("t").map(|t| t.text()).unwrap_or_default()),
        "br" => RunKind::Break,
        "fld" => RunKind::Field {
            kind: r.attr("type").unwrap_or("").to_string(),
            text: r.child("t").map(|t| t.text()).unwrap_or_default(),
        },
        _ => return None,
    };
    Some(Run { kind, props, source: Some(Arc::new(r.clone())) })
}

pub fn paragraph_of(p: &Element, rd: &Rd) -> Paragraph {
    let ppr = p.child("pPr");
    Paragraph {
        level: ppr.and_then(|e| num(e, "lvl")).unwrap_or(0).clamp(0, 8) as u8,
        props: ppr.map(|e| para_props_of(e, rd)).unwrap_or_default(),
        runs: p.elements().filter_map(|r| run_of(r, rd)).collect(),
        end_props: p.child("endParaRPr").map(|e| run_props_of(e, rd)),
        source: Some(Arc::new(p.clone())),
    }
}

pub fn text_body_of(tb: &Element, rd: &Rd) -> TextBody {
    let body_pr = tb.child("bodyPr");
    let lst = tb.child("lstStyle");
    TextBody {
        body: body_pr.map(body_props_of).unwrap_or_default(),
        list_style: lst.map(|l| list_style_of(l, rd)).unwrap_or_default(),
        paragraphs: tb.children_named("p").map(|p| paragraph_of(p, rd)).collect(),
        source_body_pr: body_pr.map(|b| Arc::new(b.clone())),
        source_lst_style: lst.map(|l| Arc::new(l.clone())),
    }
}

// ---------------------------------------------------------------- shapes --

fn cnv_pr<'a>(el: &'a Element, nv: &str) -> Option<&'a Element> {
    el.child(nv).and_then(|n| n.child("cNvPr"))
}

fn placeholder_of(nv: &Element) -> Option<Placeholder> {
    let ph = nv.child("nvPr")?.child("ph")?;
    Some(Placeholder { kind: ph.attr("type").unwrap_or("obj").to_string(), idx: num(ph, "idx").map(|i| i.max(0) as u32) })
}

fn table_of(tbl: &Element, rd: &Rd) -> Table {
    let pr = tbl.child("tblPr");
    let cols = tbl.child("tblGrid").map(|g| g.children_named("gridCol").map(|c| num(c, "w").unwrap_or(0)).collect()).unwrap_or_default();
    let rows = tbl
        .children_named("tr")
        .map(|tr| TableRow {
            height: num(tr, "h").unwrap_or(0),
            cells: tr
                .children_named("tc")
                .map(|tc| {
                    let pr = tc.child("tcPr");
                    let b = |n: &str| pr.and_then(|p| p.child(n)).map(|l| line_of(l, rd));
                    TableCell {
                        text: tc.child("txBody").map(|t| text_body_of(t, rd)).unwrap_or_default(),
                        fill: pr.and_then(|p| fill_child(p, rd)),
                        borders: [b("lnL"), b("lnR"), b("lnT"), b("lnB")],
                        grid_span: num(tc, "gridSpan").unwrap_or(1).max(1) as u32,
                        row_span: num(tc, "rowSpan").unwrap_or(1).max(1) as u32,
                        h_merge: flag(tc, "hMerge").unwrap_or(false),
                        v_merge: flag(tc, "vMerge").unwrap_or(false),
                        margins: [
                            pr.and_then(|p| num(p, "marL")),
                            pr.and_then(|p| num(p, "marT")),
                            pr.and_then(|p| num(p, "marR")),
                            pr.and_then(|p| num(p, "marB")),
                        ],
                        anchor: pr.and_then(|p| p.attr("anchor")).map(|a| match a {
                            "ctr" => Anchor::Middle,
                            "b" => Anchor::Bottom,
                            _ => Anchor::Top,
                        }),
                    }
                })
                .collect(),
        })
        .collect();
    Table {
        cols,
        rows,
        first_row: pr.and_then(|p| flag(p, "firstRow")).unwrap_or(false),
        band_row: pr.and_then(|p| flag(p, "bandRow")).unwrap_or(false),
        style_id: pr.and_then(|p| p.child("tableStyleId")).map(|s| s.text()),
    }
}

/// Parse one child of a shape tree. Unknown elements become opaque shapes.
pub fn shape_of(el: &Element, rd: &Rd) -> Shape {
    let src = Some(Arc::new(el.clone()));
    let opaque = |id: u32, name: &str, fallback: Vec<Shape>| Shape {
        id,
        name: name.to_string(),
        descr: None,
        hidden: false,
        ph: None,
        xfrm: None,
        kind: ShapeKind::Opaque { fallback },
        source: src.clone(),
    };
    let mk = |nv: &str| -> (u32, String, Option<String>, bool, Option<Placeholder>) {
        let pr = cnv_pr(el, nv);
        (
            pr.and_then(|p| num(p, "id")).unwrap_or(0).max(0) as u32,
            pr.and_then(|p| p.attr("name")).unwrap_or("").to_string(),
            pr.and_then(|p| p.attr("descr")).map(str::to_string),
            pr.and_then(|p| flag(p, "hidden")).unwrap_or(false),
            el.child(nv).and_then(placeholder_of),
        )
    };
    match el.local() {
        "sp" | "cxnSp" => {
            let connector = el.local() == "cxnSp";
            let (id, name, descr, hidden, ph) = mk(if connector { "nvCxnSpPr" } else { "nvSpPr" });
            let pr = el.child("spPr");
            let props = SpProps {
                geom: pr.and_then(geometry_of),
                fill: pr.and_then(|p| fill_child(p, rd)),
                line: pr.and_then(|p| p.child("ln")).map(|l| line_of(l, rd)),
                style: el.child("style").and_then(style_of),
                shadow: pr.and_then(shadow_of),
                text: el.child("txBody").map(|t| text_body_of(t, rd)),
                text_box: el.child("nvSpPr").and_then(|n| n.child("cNvSpPr")).and_then(|c| flag(c, "txBox")).unwrap_or(false),
            };
            Shape {
                id,
                name,
                descr,
                hidden,
                ph,
                xfrm: pr.and_then(|p| p.child("xfrm")).map(xfrm_of),
                kind: if connector { ShapeKind::Connector(Box::new(props)) } else { ShapeKind::Shape(Box::new(props)) },
                source: src,
            }
        }
        "pic" => {
            let (id, name, descr, hidden, ph) = mk("nvPicPr");
            let pr = el.child("spPr");
            let media = el
                .child("nvPicPr")
                .and_then(|n| n.child("nvPr"))
                .and_then(|n| n.elements().find(|e| matches!(e.local(), "videoFile" | "audioFile")))
                .and_then(|e| e.attr_local("link"))
                .and_then(|id| rd.target(id));
            let props = PicProps {
                image: el.child("blipFill").map(|b| image_of(b, rd)).unwrap_or_default(),
                geom: pr.and_then(geometry_of),
                line: pr.and_then(|p| p.child("ln")).map(|l| line_of(l, rd)),
                style: el.child("style").and_then(style_of),
                shadow: pr.and_then(shadow_of),
                media,
            };
            Shape {
                id,
                name,
                descr,
                hidden,
                ph,
                xfrm: pr.and_then(|p| p.child("xfrm")).map(xfrm_of),
                kind: ShapeKind::Picture(Box::new(props)),
                source: src,
            }
        }
        "grpSp" => {
            let (id, name, descr, hidden, ph) = mk("nvGrpSpPr");
            let pr = el.child("grpSpPr");
            let x = pr.and_then(|p| p.child("xfrm"));
            Shape {
                id,
                name,
                descr,
                hidden,
                ph,
                xfrm: x.map(xfrm_of),
                kind: ShapeKind::Group(Box::new(GroupProps {
                    child: x.map(child_xfrm_of).unwrap_or_default(),
                    fill: pr.and_then(|p| fill_child(p, rd)),
                    shapes: tree_shapes(el, rd),
                })),
                source: src,
            }
        }
        "graphicFrame" => {
            let (id, name, descr, hidden, ph) = mk("nvGraphicFramePr");
            let data = el.child("graphic").and_then(|g| g.child("graphicData"));
            let uri = data.and_then(|d| d.attr("uri")).unwrap_or("");
            let content = if let Some(t) = data.and_then(|d| d.child("tbl")) {
                FrameContent::Table(table_of(t, rd))
            } else if let Some(c) = data.and_then(|d| d.child("chart")) {
                FrameContent::Chart { part: c.attr_local("id").and_then(|i| rd.target(i)) }
            } else if uri.ends_with("/diagram") {
                FrameContent::Diagram
            } else if data.map(|d| d.child("oleObj").is_some()).unwrap_or(false) {
                FrameContent::Ole
            } else {
                FrameContent::Other(uri.to_string())
            };
            Shape {
                id,
                name,
                descr,
                hidden,
                ph,
                xfrm: el.child("xfrm").map(xfrm_of),
                kind: ShapeKind::Frame(Box::new(FrameProps { content, drawing: Vec::new() })),
                source: src,
            }
        }
        "AlternateContent" => {
            // Prefer the fallback branch for display; keep everything verbatim.
            let branch = el.child("Fallback").or_else(|| el.child("Choice"));
            let fallback = branch.map(|b| tree_shapes(b, rd)).unwrap_or_default();
            let id = fallback.first().map(|s| s.id).unwrap_or(0);
            opaque(id, "AlternateContent", fallback)
        }
        other => opaque(0, other, Vec::new()),
    }
}

/// Parse all shape children of an `spTree`/`grpSp`-like element, skipping
/// its property header elements.
pub fn tree_shapes(tree: &Element, rd: &Rd) -> Vec<Shape> {
    tree.elements()
        .filter(|e| !matches!(e.local(), "nvGrpSpPr" | "grpSpPr" | "extLst"))
        .map(|e| shape_of(e, rd))
        .collect()
}

fn find_desc<'a>(e: &'a Element, local: &str) -> Option<&'a Element> {
    if e.local() == local {
        return Some(e);
    }
    e.elements().find_map(|c| find_desc(c, local))
}

/// Load the pre-rendered drawing of a SmartArt frame.
fn load_drawing(pkg: &Package, rels: &[Rel], frame: &Element) -> Vec<Shape> {
    let by_id = |id: &str| rels.iter().find(|r| r.id == id).map(|r| r.target.clone());
    let from_model = frame
        .path(&["graphic", "graphicData", "relIds"])
        .and_then(|r| r.attr_local("dm"))
        .and_then(by_id)
        .and_then(|dm| pkg.xml(&dm).ok())
        .and_then(|d| find_desc(&d.root, "dataModelExt").and_then(|e| e.attr("relId").map(str::to_string)))
        .and_then(|id| by_id(&id));
    let part = from_model.or_else(|| rels.iter().find(|r| r.ty.ends_with("/diagramDrawing")).map(|r| r.target.clone()));
    let Some(part) = part else { return Vec::new() };
    let Ok(doc) = pkg.xml(&part) else { return Vec::new() };
    let drels = pkg.rels_of(&part);
    doc.root.child("spTree").map(|t| tree_shapes(t, &Rd { rels: &drels })).unwrap_or_default()
}

fn attach_diagrams(pkg: &Package, rels: &[Rel], shapes: &mut [Shape]) {
    for s in shapes {
        match &mut s.kind {
            ShapeKind::Group(g) => attach_diagrams(pkg, rels, &mut g.shapes),
            ShapeKind::Frame(f) if matches!(f.content, FrameContent::Diagram) => {
                if let Some(src) = &s.source {
                    f.drawing = load_drawing(pkg, rels, src);
                }
            }
            _ => {}
        }
    }
}

pub fn background_of(csld: &Element, rd: &Rd) -> Option<Background> {
    let bg = csld.child("bg")?;
    if let Some(p) = bg.child("bgPr") {
        return fill_child(p, rd).map(Background::Fill);
    }
    let r = bg.child("bgRef")?;
    Some(Background::Ref { idx: num(r, "idx").unwrap_or(0).max(0) as u32, color: color_child(r) })
}

// -------------------------------------------------------------- transition --

const TRANSITION_KINDS: &[&str] = &[
    "blinds", "checker", "circle", "dissolve", "comb", "cover", "cut", "diamond", "fade", "newsflash", "plus", "pull",
    "push", "random", "randomBar", "split", "strips", "wedge", "wheel", "wipe", "zoom", "vortex", "switch", "flip",
    "ripple", "honeycomb", "prism", "doors", "window", "ferris", "gallery", "conveyor", "pan", "glitter", "warp",
    "flythrough", "flash", "shred", "reveal", "wheelReverse", "morph",
];

pub fn transition_of(sld: &Element) -> Option<Transition> {
    let find = |e: &Element| -> Option<Transition> {
        let el = e.children_named("transition").next()?;
        let effect = el.elements().find(|c| TRANSITION_KINDS.contains(&c.local()));
        let spd = el.attr("spd").unwrap_or("fast");
        let dur = el
            .attr_local("dur")
            .and_then(|d| d.parse::<u32>().ok())
            .unwrap_or(match spd {
                "slow" => 1000,
                "med" => 750,
                _ => 500,
            });
        Some(Transition {
            kind: effect.map(|c| c.local().to_string()).unwrap_or_else(|| "none".into()),
            dir: effect.and_then(|c| c.attr("dir").or_else(|| c.attr("orient"))).map(str::to_string),
            duration_ms: dur,
            advance_on_click: flag(el, "advClick").unwrap_or(true),
            advance_after_ms: num(el, "advTm").map(|v| v.max(0) as u32),
        })
    };
    if let Some(t) = find(sld) {
        return Some(t);
    }
    // Transitions with extensions live in mc:AlternateContent; prefer the Choice
    // branch (it carries the richest effect), then Fallback.
    let ac = sld.child("AlternateContent")?;
    ac.child("Choice").and_then(find).or_else(|| ac.child("Fallback").and_then(find))
}

// ---------------------------------------------------------------- theme ---

pub fn theme_of(root: &Element, part: &str, rd: &Rd) -> Theme {
    let te = root.child("themeElements");
    let mut colors = ColorScheme::default();
    if let Some(cs) = te.and_then(|t| t.child("clrScheme")) {
        colors.name = cs.attr("name").unwrap_or("").to_string();
        for slot in cs.elements() {
            if let Some(c) = slot.elements().find_map(color_of) {
                let ctx_scheme = ColorScheme::default();
                let map = ClrMap::default();
                let rgba = kinaute_model::color::ColorCtx { scheme: &ctx_scheme, map: &map, ph_clr: None }.resolve(&c);
                colors.slots.insert(slot.local().to_string(), rgba);
            }
        }
    }
    let font = |n: &str| {
        te.and_then(|t| t.path(&["fontScheme", n, "latin"]))
            .and_then(|l| l.attr("typeface"))
            .unwrap_or("")
            .to_string()
    };
    let fmt = te.and_then(|t| t.child("fmtScheme"));
    let fills = |name: &str| -> Vec<Fill> {
        fmt.and_then(|f| f.child(name))
            .map(|l| l.elements().filter_map(|e| fill_of(e, rd)).collect())
            .unwrap_or_default()
    };
    let lines = fmt
        .and_then(|f| f.child("lnStyleLst"))
        .map(|l| l.children_named("ln").map(|ln| line_of(ln, rd)).collect())
        .unwrap_or_default();
    let effects = fmt
        .and_then(|f| f.child("effectStyleLst"))
        .map(|l| l.children_named("effectStyle").map(|e| e.child("effectLst").and_then(shadow_of)).collect())
        .unwrap_or_default();
    Theme {
        part: part.to_string(),
        name: root.attr("name").unwrap_or("").to_string(),
        colors,
        fonts: FontScheme { major: font("majorFont"), minor: font("minorFont") },
        fill_styles: fills("fillStyleLst"),
        line_styles: lines,
        bg_fill_styles: fills("bgFillStyleLst"),
        effect_shadows: effects,
    }
}

fn clr_map_of(e: &Element) -> ClrMap {
    let mut m = ClrMap::default();
    for (k, v) in &e.attrs {
        m.0.insert(k.clone(), v.clone());
    }
    m
}

// ---------------------------------------------------------------- open ----

/// The loaded document: model plus the raw package it came from.
pub struct Opened {
    pub pres: Presentation,
    pub package: Package,
    /// Non-fatal problems found while importing (never silent loss).
    pub report: Vec<String>,
}

fn text_styles_of(root: &Element, rd: &Rd) -> TextStyles {
    let ts = root.child("txStyles");
    let get = |n: &str| ts.and_then(|t| t.child(n)).map(|l| list_style_of(l, rd)).unwrap_or_default();
    TextStyles { title: get("titleStyle"), body: get("bodyStyle"), other: get("otherStyle") }
}

pub fn open(bytes: &[u8]) -> Result<Opened, PptxError> {
    let package = Package::read(bytes)?;
    let mut report = Vec::new();

    if !package.contains("[Content_Types].xml") {
        return Err(PptxError::NotPresentation("no [Content_Types].xml"));
    }
    let root_rels = package.rels_of("");
    let main = root_rels
        .iter()
        .find(|r| r.ty == REL_OFFICE_DOC)
        .map(|r| r.target.clone())
        .ok_or(PptxError::NotPresentation("no main document relationship"))?;
    let pxml = package.xml(&main)?;
    if pxml.root.local() != "presentation" {
        return Err(PptxError::NotPresentation("main part is not a presentation"));
    }
    let prels = package.rels_of(&main);
    let prd = Rd { rels: &prels };

    let sz = pxml.root.child("sldSz");
    let slide_w = sz.and_then(|s| num(s, "cx")).filter(|v| *v > 0).unwrap_or(9_144_000);
    let slide_h = sz.and_then(|s| num(s, "cy")).filter(|v| *v > 0).unwrap_or(6_858_000);
    let default_text = pxml.root.child("defaultTextStyle").map(|d| list_style_of(d, &prd)).unwrap_or_default();

    // Masters, their themes and layouts.
    let mut themes: Vec<Arc<Theme>> = Vec::new();
    let mut masters: Vec<Arc<Master>> = Vec::new();
    let mut layouts: Vec<Arc<Layout>> = Vec::new();
    let mut layout_index: BTreeMap<String, usize> = BTreeMap::new();

    let master_ids: Vec<String> = pxml
        .root
        .child("sldMasterIdLst")
        .map(|l| l.elements().filter_map(|m| m.attr_ns("id").map(str::to_string)).collect())
        .unwrap_or_default();
    for rid in master_ids {
        let Some(mpart) = prels.iter().find(|r| r.id == rid).map(|r| r.target.clone()) else {
            report.push(format!("master relationship {rid} not found"));
            continue;
        };
        let mdoc = match package.xml(&mpart) {
            Ok(d) => d,
            Err(e) => {
                report.push(format!("master `{mpart}` unreadable: {e}"));
                continue;
            }
        };
        let mrels = package.rels_of(&mpart);
        let mrd = Rd { rels: &mrels };
        // Theme.
        let theme_idx = match mrels.iter().find(|r| r.ty == REL_THEME) {
            Some(tr) => match package.xml(&tr.target) {
                Ok(t) => {
                    let trels = package.rels_of(&tr.target);
                    themes.push(Arc::new(theme_of(&t.root, &tr.target, &Rd { rels: &trels })));
                    themes.len() - 1
                }
                Err(e) => {
                    report.push(format!("theme `{}` unreadable: {e}", tr.target));
                    themes.push(Arc::new(Theme::default()));
                    themes.len() - 1
                }
            },
            None => {
                report.push(format!("master `{mpart}` has no theme"));
                themes.push(Arc::new(Theme::default()));
                themes.len() - 1
            }
        };
        let csld = mdoc.root.child("cSld");
        let master_idx = masters.len();
        masters.push(Arc::new(Master {
            part: mpart.clone(),
            theme: theme_idx,
            background: csld.and_then(|c| background_of(c, &mrd)),
            shapes: csld.and_then(|c| c.child("spTree")).map(|t| tree_shapes(t, &mrd)).unwrap_or_default(),
            clr_map: mdoc.root.child("clrMap").map(clr_map_of).unwrap_or_default(),
            text_styles: text_styles_of(&mdoc.root, &mrd),
        }));
        // Layouts of this master.
        for lrel in mrels.iter().filter(|r| r.ty == REL_LAYOUT) {
            let ldoc = match package.xml(&lrel.target) {
                Ok(d) => d,
                Err(e) => {
                    report.push(format!("layout `{}` unreadable: {e}", lrel.target));
                    continue;
                }
            };
            let lrels = package.rels_of(&lrel.target);
            let lrd = Rd { rels: &lrels };
            let csld = ldoc.root.child("cSld");
            layout_index.insert(lrel.target.clone(), layouts.len());
            layouts.push(Arc::new(Layout {
                part: lrel.target.clone(),
                name: csld.and_then(|c| c.attr("name")).unwrap_or("").to_string(),
                kind: ldoc.root.attr("type").map(str::to_string),
                master: master_idx,
                background: csld.and_then(|c| background_of(c, &lrd)),
                shapes: csld.and_then(|c| c.child("spTree")).map(|t| tree_shapes(t, &lrd)).unwrap_or_default(),
                show_master_shapes: ldoc.root.attr("showMasterSp").map(|v| v != "0" && v != "false").unwrap_or(true),
                clr_map_override: ldoc.root.child("clrMapOvr").and_then(|c| c.child("overrideClrMapping")).map(clr_map_of),
            }));
        }
    }

    // Media.
    let mut media = MediaStore::new();
    for (name, data) in &package.parts {
        if name.starts_with("ppt/media/") || name.starts_with("ppt/embeddings/") {
            media.insert(name.clone(), data.clone());
        }
    }

    // Slides.
    let mut slides: Vec<Arc<Slide>> = Vec::new();
    // `r:id` is the namespaced attribute; plain `id` is the numeric slide id.
    let slide_rids: Vec<String> = pxml
        .root
        .child("sldIdLst")
        .map(|l| l.elements().filter_map(|s| s.attr_ns("id").map(str::to_string)).collect())
        .unwrap_or_default();
    for rid in slide_rids {
        let Some(spart) = prels.iter().find(|r| r.id == rid).map(|r| r.target.clone()) else {
            report.push(format!("slide relationship {rid} not found"));
            continue;
        };
        let sdoc = match package.xml(&spart) {
            Ok(d) => d,
            Err(e) => {
                report.push(format!("slide `{spart}` unreadable: {e}"));
                continue;
            }
        };
        let srels = package.rels_of(&spart);
        let srd = Rd { rels: &srels };
        let csld = sdoc.root.child("cSld");
        let layout = srels
            .iter()
            .find(|r| r.ty == REL_LAYOUT)
            .and_then(|r| layout_index.get(&r.target).copied())
            .unwrap_or_else(|| {
                report.push(format!("slide `{spart}` has no valid layout"));
                usize::MAX
            });
        let notes = srels.iter().find(|r| r.ty == REL_NOTES).and_then(|r| {
            let nd = package.xml(&r.target).ok()?;
            let nrels = package.rels_of(&r.target);
            let shapes = nd.root.child("cSld").and_then(|c| c.child("spTree")).map(|t| tree_shapes(t, &Rd { rels: &nrels }))?;
            let texts: Vec<String> = shapes
                .iter()
                .filter(|s| s.ph.as_ref().map(|p| p.kind == "body").unwrap_or(false))
                .filter_map(|s| s.text().map(|t| t.plain_text()))
                .collect();
            Some(texts.join("\n")).filter(|t| !t.is_empty())
        });
        let mut shapes = csld.and_then(|c| c.child("spTree")).map(|t| tree_shapes(t, &srd)).unwrap_or_default();
        attach_diagrams(&package, &srels, &mut shapes);
        for s in &shapes {
            if let ShapeKind::Frame(f) = &s.kind {
                match &f.content {
                    FrameContent::Diagram if f.drawing.is_empty() => report.push(format!("`{spart}`: SmartArt `{}` kept as-is (no drawing to display)", s.name)),
                    FrameContent::Ole => report.push(format!("`{spart}`: embedded object `{}` kept as-is", s.name)),
                    _ => {}
                }
            }
        }
        slides.push(Arc::new(Slide {
            rels: srels.clone(),
            part: Some(spart),
            layout,
            name: csld.and_then(|c| c.attr("name")).map(str::to_string),
            background: csld.and_then(|c| background_of(c, &srd)),
            shapes,
            transition: transition_of(&sdoc.root),
            hidden: sdoc.root.attr("show").map(|v| v == "0" || v == "false").unwrap_or(false),
            show_master_shapes: sdoc.root.attr("showMasterSp").map(|v| v != "0" && v != "false").unwrap_or(true),
            notes,
            source: Some(Arc::new(sdoc.root.clone())),
        }));
    }

    if slides.is_empty() && pxml.root.child("sldIdLst").is_none() {
        report.push("presentation has no slides".into());
    }

    Ok(Opened {
        pres: Presentation {
            slide_w,
            slide_h,
            slides,
            layouts,
            masters,
            themes,
            default_text: Arc::new(default_text),
            media: Arc::new(media),
        },
        package,
        report,
    })
}
