//! Inheritance resolution: placeholders (slide → layout → master), text
//! styles, theme fonts, colours and the theme style matrix.

use crate::color::{ClrMap, Color, ColorCtx, ColorScheme, Rgba};
use crate::types::*;

/// Default insets of a text body (0.1" left/right, 0.05" top/bottom).
pub const DEFAULT_INSETS: [Emu; 4] = [91440, 45720, 91440, 45720];
/// Default font size when nothing is specified anywhere (18 pt).
pub const DEFAULT_FONT_SIZE: i32 = 1800;

/// Which level of the hierarchy a shape list belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Slide,
    Layout,
    Master,
}

/// Resolution context for shapes on a slide, layout or master.
pub struct Ctx<'a> {
    pub pres: &'a Presentation,
    pub level: Level,
    pub slide: Option<&'a Slide>,
    pub layout: Option<&'a Layout>,
    pub master: Option<&'a Master>,
    pub theme: Option<&'a Theme>,
    pub clr_map: ClrMap,
    empty_scheme: ColorScheme,
}

impl<'a> Ctx<'a> {
    pub fn for_slide(pres: &'a Presentation, slide: &'a Slide) -> Self {
        let layout = pres.layout_of(slide);
        let master = layout.and_then(|l| pres.master_of_layout(l));
        Self::build(pres, Level::Slide, Some(slide), layout, master)
    }

    pub fn for_layout(pres: &'a Presentation, layout: &'a Layout) -> Self {
        let master = pres.master_of_layout(layout);
        Self::build(pres, Level::Layout, None, Some(layout), master)
    }

    pub fn for_master(pres: &'a Presentation, master: &'a Master) -> Self {
        Self::build(pres, Level::Master, None, None, Some(master))
    }

    fn build(
        pres: &'a Presentation,
        level: Level,
        slide: Option<&'a Slide>,
        layout: Option<&'a Layout>,
        master: Option<&'a Master>,
    ) -> Self {
        let theme = master.and_then(|m| pres.theme_of_master(m)).or_else(|| pres.themes.first().map(|t| t.as_ref()));
        let mut clr_map = master.map(|m| m.clr_map.clone()).unwrap_or_default();
        if let Some(o) = layout.and_then(|l| l.clr_map_override.as_ref()) {
            clr_map = o.clone();
        }
        Ctx { pres, level, slide, layout, master, theme, clr_map, empty_scheme: ColorScheme::default() }
    }

    fn scheme(&self) -> &ColorScheme {
        self.theme.map(|t| &t.colors).unwrap_or(&self.empty_scheme)
    }

    /// Resolve a colour to RGBA.
    pub fn color(&self, c: &Color) -> Rgba {
        ColorCtx { scheme: self.scheme(), map: &self.clr_map, ph_clr: None }.resolve(c)
    }

    /// Resolve a colour where `phClr` stands for `ph`.
    pub fn color_with(&self, c: &Color, ph: Option<&Color>) -> Rgba {
        ColorCtx { scheme: self.scheme(), map: &self.clr_map, ph_clr: ph }.resolve(c)
    }

    /// Resolve a theme font reference (`+mj-lt`, `+mn-lt`, ...).
    pub fn font_name(&self, name: &str) -> String {
        let fonts = self.theme.map(|t| &t.fonts);
        match name {
            n if n.starts_with("+mj") => fonts.map(|f| f.major.clone()).unwrap_or_else(|| "Calibri Light".into()),
            n if n.starts_with("+mn") => fonts.map(|f| f.minor.clone()).unwrap_or_else(|| "Calibri".into()),
            n => n.to_string(),
        }
    }

    // ------------------------------------------------------ placeholders --

    /// The layout placeholder a slide placeholder inherits from.
    pub fn layout_ph(&self, ph: &Placeholder) -> Option<&'a Shape> {
        if self.level != Level::Slide {
            return None;
        }
        find_ph(&self.layout?.shapes, ph, false)
    }

    /// The master placeholder a slide or layout placeholder inherits from.
    pub fn master_ph(&self, ph: &Placeholder) -> Option<&'a Shape> {
        if self.level == Level::Master {
            return None;
        }
        find_ph(&self.master?.shapes, ph, true)
    }

    /// Inheritance chain for a shape, most specific first (excluding itself).
    pub fn ph_chain(&self, shape: &Shape) -> Vec<&'a Shape> {
        let mut v = Vec::new();
        if let Some(ph) = &shape.ph {
            if let Some(l) = self.layout_ph(ph) {
                v.push(l);
            }
            if let Some(m) = self.master_ph(ph) {
                v.push(m);
            }
        }
        v
    }

    /// Effective transform (inherited from the placeholder chain).
    pub fn xfrm(&self, shape: &Shape) -> Option<Xfrm> {
        shape.xfrm.or_else(|| self.ph_chain(shape).iter().find_map(|s| s.xfrm))
    }

    /// Effective body properties of a text shape.
    pub fn body_props(&self, shape: &Shape) -> BodyProps {
        let mut b = shape.text().map(|t| t.body.clone()).unwrap_or_default();
        for s in self.ph_chain(shape) {
            if let Some(t) = s.text() {
                b.inherit(&t.body);
            }
        }
        b
    }

    /// Master text style family for a shape.
    fn master_style(&self, shape: &Shape) -> Option<&'a ListStyle> {
        let m = self.master?;
        Some(match &shape.ph {
            Some(ph) if ph.is_title() => &m.text_styles.title,
            Some(ph) if matches!(ph.kind.as_str(), "dt" | "ftr" | "sldNum") => &m.text_styles.other,
            Some(_) => &m.text_styles.body,
            None => &m.text_styles.other,
        })
    }

    /// Fully resolved paragraph properties (including `def_run`), for a
    /// paragraph of `body` belonging to `shape`.
    pub fn para_props(&self, shape: &Shape, body: &TextBody, para: &Paragraph) -> ParaProps {
        self.para_props_lvl(shape, body, para.level as usize, Some(&para.props))
    }

    pub fn para_props_lvl(&self, shape: &Shape, body: &TextBody, lvl: usize, own: Option<&ParaProps>) -> ParaProps {
        let lvl = lvl.min(8);
        // Most specific first.
        let mut chain: Vec<&ParaProps> = Vec::new();
        if let Some(p) = own {
            chain.push(p);
        }
        push_list(&mut chain, &body.list_style, lvl);
        for s in self.ph_chain(shape) {
            if let Some(t) = s.text() {
                push_list(&mut chain, &t.list_style, lvl);
            }
        }
        // The shape style's font reference (`p:style/fontRef`) outranks the
        // master and presentation defaults but not explicit formatting.
        let mut style_pp = ParaProps::default();
        if let Some(st) = shape.sp().and_then(|s| s.style.as_ref()) {
            if let Some(c) = &st.font.1 {
                style_pp.def_run.fill = Some(Fill::Solid(c.clone()));
            }
            match st.font.0.as_str() {
                "major" => style_pp.def_run.font = Some("+mj-lt".into()),
                "minor" => style_pp.def_run.font = Some("+mn-lt".into()),
                _ => {}
            }
        }
        chain.push(&style_pp);
        if let Some(ls) = self.master_style(shape) {
            push_list(&mut chain, ls, lvl);
        }
        if shape.ph.is_none() {
            push_list(&mut chain, &self.pres.default_text, lvl);
        }
        let mut out = ParaProps::default();
        for p in chain {
            out.inherit(p);
        }
        out
    }

    /// Fully resolved run properties.
    pub fn run_props(&self, para_props: &ParaProps, run: &RunProps) -> RunProps {
        let mut r = run.clone();
        r.inherit(&para_props.def_run);
        r
    }

    // ------------------------------------------------- fills and lines --

    /// Effective fill of a shape: explicit, then placeholder chain, then the
    /// theme style matrix. `None` means "no fill".
    pub fn shape_fill(&self, shape: &Shape) -> Option<(Fill, Option<Color>)> {
        let (own_fill, style) = match &shape.kind {
            ShapeKind::Shape(s) | ShapeKind::Connector(s) => (s.fill.clone(), s.style.as_ref()),
            ShapeKind::Picture(_) => (None, None),
            ShapeKind::Group(g) => (g.fill.clone(), None),
            _ => (None, None),
        };
        if let Some(f) = own_fill {
            return Some((f, None));
        }
        for s in self.ph_chain(shape) {
            if let Some(f) = s.sp().and_then(|sp| sp.fill.clone()) {
                return Some((f, None));
            }
        }
        let style = style?;
        let (idx, color) = &style.fill;
        let theme = self.theme?;
        let f = match *idx {
            0 => return None,
            i if i >= 1000 => theme.bg_fill_styles.get((i - 1001) as usize).cloned(),
            i => theme.fill_styles.get((i - 1) as usize).cloned(),
        }?;
        Some((f, color.clone()))
    }

    /// Effective outline of a shape (explicit values merged over the style
    /// matrix), with the `phClr` substitute colour.
    pub fn shape_line(&self, shape: &Shape) -> Option<(Line, Option<Color>)> {
        let (own, style) = match &shape.kind {
            ShapeKind::Shape(s) | ShapeKind::Connector(s) => (s.line.clone(), s.style.as_ref()),
            ShapeKind::Picture(p) => (p.line.clone(), p.style.as_ref()),
            _ => (None, None),
        };
        let mut line = own.unwrap_or_default();
        for s in self.ph_chain(shape) {
            if let Some(l) = s.sp().and_then(|sp| sp.line.as_ref()) {
                merge_line(&mut line, l);
            }
        }
        let mut ph = None;
        if let (Some(st), Some(theme)) = (style, self.theme) {
            let (idx, color) = &st.line;
            if *idx > 0 {
                if let Some(base) = theme.line_styles.get((*idx - 1) as usize) {
                    merge_line(&mut line, base);
                    ph = color.clone();
                }
            }
        }
        match &line.fill {
            None | Some(Fill::None) => None,
            Some(_) => Some((line, ph)),
        }
    }

    /// Effective shadow from explicit effects or the style matrix.
    pub fn shape_shadow(&self, shape: &Shape) -> Option<(Shadow, Option<Color>)> {
        let (own, style) = match &shape.kind {
            ShapeKind::Shape(s) | ShapeKind::Connector(s) => (s.shadow.clone(), s.style.as_ref()),
            ShapeKind::Picture(p) => (p.shadow.clone(), p.style.as_ref()),
            _ => (None, None),
        };
        if let Some(s) = own {
            return Some((s, None));
        }
        let st = style?;
        let (idx, color) = &st.effect;
        if *idx == 0 {
            return None;
        }
        let sh = self.theme?.effect_shadows.get((*idx - 1) as usize)?.clone()?;
        Some((sh, color.clone()))
    }

    /// Effective background of the current slide/layout/master.
    pub fn background(&self) -> Option<(Fill, Option<Color>)> {
        let bgs = [
            self.slide.and_then(|s| s.background.as_ref()),
            self.layout.and_then(|l| l.background.as_ref()),
            self.master.and_then(|m| m.background.as_ref()),
        ];
        let bg = bgs.into_iter().flatten().next()?;
        match bg {
            Background::Fill(f) => Some((f.clone(), None)),
            Background::Ref { idx, color } => {
                let theme = self.theme?;
                let f = if *idx >= 1001 {
                    theme.bg_fill_styles.get((*idx - 1001) as usize)
                } else if *idx >= 1 {
                    theme.fill_styles.get((*idx - 1) as usize)
                } else {
                    None
                }?;
                Some((f.clone(), color.clone()))
            }
        }
    }
}

fn push_list<'b>(chain: &mut Vec<&'b ParaProps>, ls: &'b ListStyle, lvl: usize) {
    if let Some(p) = ls.level(lvl) {
        chain.push(p);
    }
    if let Some(d) = &ls.default {
        chain.push(d);
    }
}

fn merge_line(dst: &mut Line, src: &Line) {
    if dst.width.is_none() {
        dst.width = src.width;
    }
    if dst.fill.is_none() {
        dst.fill = src.fill.clone();
    }
    if dst.dash.is_none() {
        dst.dash = src.dash.clone();
    }
    if dst.cap.is_none() {
        dst.cap = src.cap.clone();
    }
    if dst.join.is_none() {
        dst.join = src.join.clone();
    }
    if dst.head.is_none() {
        dst.head = src.head.clone();
    }
    if dst.tail.is_none() {
        dst.tail = src.tail.clone();
    }
}

/// Normalise a placeholder type for master lookup.
fn master_kind(kind: &str) -> &str {
    match kind {
        "ctrTitle" | "title" => "title",
        "dt" => "dt",
        "ftr" => "ftr",
        "sldNum" => "sldNum",
        _ => "body",
    }
}

fn find_ph<'s>(shapes: &'s [Shape], ph: &Placeholder, master: bool) -> Option<&'s Shape> {
    let phs = || shapes.iter().filter_map(|s| s.ph.as_ref().map(|p| (s, p)));
    if master {
        let k = master_kind(&ph.kind);
        return phs()
            .find(|(_, p)| master_kind(&p.kind) == k && p.kind == k)
            .or_else(|| phs().find(|(_, p)| master_kind(&p.kind) == k))
            .map(|(s, _)| s);
    }
    // Title-like placeholders match by type; others by index first.
    if ph.is_title() {
        if let Some((s, _)) = phs().find(|(_, p)| p.is_title()) {
            return Some(s);
        }
    }
    if let Some(idx) = ph.idx {
        if let Some((s, _)) = phs().find(|(_, p)| p.idx == Some(idx)) {
            return Some(s);
        }
    }
    phs().find(|(_, p)| p.kind == ph.kind).map(|(s, _)| s)
}
