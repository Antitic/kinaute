//! Core document types. Units follow DrawingML so nothing is lost when
//! reading or writing `.pptx`: lengths in EMU (914400 per inch, 12700 per
//! point), angles in 60000ths of a degree, percentages in 1000ths of a percent
//! (100000 = 100 %), font sizes in 100ths of a point.

use crate::color::{ClrMap, Color, ColorScheme};
use kinaute_xml::Element;
use std::collections::BTreeMap;
use std::sync::Arc;

pub type Emu = i64;
pub const EMU_PER_PT: f64 = 12700.0;
pub const EMU_PER_INCH: f64 = 914_400.0;

/// Position, size, rotation and flips of an object (`a:xfrm`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Xfrm {
    pub x: Emu,
    pub y: Emu,
    pub w: Emu,
    pub h: Emu,
    /// Clockwise rotation, 60000ths of a degree.
    pub rot: i32,
    pub flip_h: bool,
    pub flip_v: bool,
}

impl Xfrm {
    pub fn new(x: Emu, y: Emu, w: Emu, h: Emu) -> Self {
        Xfrm { x, y, w, h, ..Default::default() }
    }
    pub fn rot_deg(&self) -> f64 {
        self.rot as f64 / 60000.0
    }
    pub fn center(&self) -> (f64, f64) {
        (self.x as f64 + self.w as f64 / 2.0, self.y as f64 + self.h as f64 / 2.0)
    }
}

/// Gradient (`a:gradFill`).
#[derive(Debug, Clone, PartialEq)]
pub struct Gradient {
    /// Stops: position 0..=100000 and colour.
    pub stops: Vec<(i32, Color)>,
    pub kind: GradientKind,
    pub rot_with_shape: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum GradientKind {
    /// Linear gradient; angle in 60000ths of a degree (0 = left to right).
    Linear { angle: i32, scaled: bool },
    /// Radial-like gradients (`circle`, `rect`, `shape`) with the
    /// `fillToRect` focus (l, t, r, b in 1000ths of a percent).
    Path { path: String, focus: (i32, i32, i32, i32) },
}

/// Picture fill (`a:blipFill`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ImageFill {
    /// Package part of the image (`ppt/media/image1.png`), `None` when the
    /// relationship is missing or external.
    pub media: Option<String>,
    /// Source cropping (l, t, r, b) in 1000ths of a percent.
    pub crop: (i32, i32, i32, i32),
    /// `true` for `a:stretch`, `false` for `a:tile`.
    pub stretch: bool,
    /// Opacity from `a:alphaModFix` (100000 = opaque).
    pub alpha: i32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Fill {
    None,
    Solid(Color),
    Gradient(Gradient),
    Image(ImageFill),
    /// `a:pattFill`: preset name, foreground, background.
    Pattern {
        preset: String,
        fg: Option<Color>,
        bg: Option<Color>,
    },
    /// `a:grpFill`: use the parent group's fill.
    Group,
}

/// Outline (`a:ln`). `None` fields inherit from the style.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Line {
    pub width: Option<Emu>,
    pub fill: Option<Fill>,
    /// Preset dash (`solid`, `dash`, `sysDot`, ...).
    pub dash: Option<String>,
    pub cap: Option<String>,
    pub join: Option<String>,
    pub head: Option<String>,
    pub tail: Option<String>,
}

/// Shape geometry.
#[derive(Debug, Clone, PartialEq)]
pub enum Geometry {
    /// Preset geometry with optional adjust values (`adj`, `adj1`...).
    Preset {
        name: String,
        adj: Vec<(String, i64)>,
    },
    Custom(CustomGeometry),
}

impl Geometry {
    pub fn preset(name: &str) -> Self {
        Geometry::Preset { name: name.to_string(), adj: Vec::new() }
    }
}

/// `a:custGeom`, with guide formulas kept as text.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CustomGeometry {
    pub av: Vec<(String, String)>,
    pub gd: Vec<(String, String)>,
    pub paths: Vec<GeomPath>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GeomPath {
    pub w: Option<i64>,
    pub h: Option<i64>,
    /// `norm`, `none`, `lighten`, `darken`...
    pub fill: String,
    pub stroke: bool,
    pub cmds: Vec<PathCmd>,
}

/// Path commands; coordinates are guide names or literal numbers.
#[derive(Debug, Clone, PartialEq)]
pub enum PathCmd {
    MoveTo(String, String),
    LineTo(String, String),
    ArcTo { wr: String, hr: String, st: String, sw: String },
    QuadTo([(String, String); 2]),
    CubicTo([(String, String); 3]),
    Close,
}

/// References into the theme's style matrix (`p:style`).
#[derive(Debug, Clone, PartialEq)]
pub struct StyleRefs {
    pub line: (u32, Option<Color>),
    pub fill: (u32, Option<Color>),
    pub effect: (u32, Option<Color>),
    /// `major`, `minor` or `none`, with an optional colour.
    pub font: (String, Option<Color>),
}

/// Outer shadow (`a:outerShdw`).
#[derive(Debug, Clone, PartialEq)]
pub struct Shadow {
    pub blur: Emu,
    pub dist: Emu,
    /// Direction, 60000ths of a degree.
    pub dir: i32,
    pub color: Color,
}

// ---------------------------------------------------------------- text ----

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
    Justify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Anchor {
    #[default]
    Top,
    Middle,
    Bottom,
}

/// Line or paragraph spacing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Spacing {
    /// 1000ths of a percent of the line height (100000 = single).
    Percent(i32),
    /// 100ths of a point.
    Points(i32),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Bullet {
    None,
    Char { ch: String, font: Option<String>, color: Option<Color>, size_pct: Option<i32> },
    AutoNum { scheme: String, start: i32, color: Option<Color>, size_pct: Option<i32> },
}

/// Character properties; `None` means "inherit".
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RunProps {
    /// 100ths of a point.
    pub size: Option<i32>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    /// `none`, `sng`, `dbl`, ...
    pub underline: Option<String>,
    /// `noStrike`, `sngStrike`, `dblStrike`.
    pub strike: Option<String>,
    pub fill: Option<Fill>,
    /// Latin typeface (may be a theme reference such as `+mj-lt`).
    pub font: Option<String>,
    /// Superscript/subscript offset in 1000ths of a percent.
    pub baseline: Option<i32>,
    /// Character spacing in 100ths of a point.
    pub spacing: Option<i32>,
    /// `none`, `small`, `all`.
    pub caps: Option<String>,
    pub lang: Option<String>,
    pub highlight: Option<Color>,
    pub outline: Option<Line>,
    pub link: Option<String>,
}

impl RunProps {
    /// Fill `None` fields from `parent`.
    pub fn inherit(&mut self, parent: &RunProps) {
        macro_rules! inh {
            ($($f:ident),*) => { $( if self.$f.is_none() { self.$f = parent.$f.clone(); } )* };
        }
        inh!(size, bold, italic, underline, strike, fill, font, baseline, spacing, caps, lang, highlight, outline);
    }
}

/// Paragraph properties; `None` means "inherit".
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ParaProps {
    pub align: Option<Align>,
    pub mar_l: Option<Emu>,
    pub indent: Option<Emu>,
    pub line_spacing: Option<Spacing>,
    pub space_before: Option<Spacing>,
    pub space_after: Option<Spacing>,
    pub bullet: Option<Bullet>,
    pub rtl: Option<bool>,
    /// Default run properties (`a:defRPr`).
    pub def_run: RunProps,
}

impl ParaProps {
    pub fn inherit(&mut self, parent: &ParaProps) {
        macro_rules! inh {
            ($($f:ident),*) => { $( if self.$f.is_none() { self.$f = parent.$f.clone(); } )* };
        }
        inh!(align, mar_l, indent, line_spacing, space_before, space_after, bullet, rtl);
        self.def_run.inherit(&parent.def_run);
    }
}

/// List style: paragraph properties per level 1..=9 (index 0..=8).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ListStyle {
    pub default: Option<ParaProps>,
    pub levels: [Option<ParaProps>; 9],
}

impl ListStyle {
    pub fn level(&self, lvl: usize) -> Option<&ParaProps> {
        self.levels.get(lvl).and_then(Option::as_ref)
    }
    pub fn is_empty(&self) -> bool {
        self.default.is_none() && self.levels.iter().all(Option::is_none)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum RunKind {
    Text(String),
    /// `a:br`
    Break,
    /// `a:fld`: field type (`slidenum`, `datetime1`...) and cached text.
    Field {
        kind: String,
        text: String,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Run {
    pub kind: RunKind,
    pub props: RunProps,
    pub source: Option<Arc<Element>>,
}

impl Run {
    pub fn text(s: impl Into<String>) -> Self {
        Run { kind: RunKind::Text(s.into()), props: RunProps::default(), source: None }
    }
    pub fn as_str(&self) -> &str {
        match &self.kind {
            RunKind::Text(t) => t,
            RunKind::Break => "\u{2028}",
            RunKind::Field { text, .. } => text,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Paragraph {
    /// Outline level 0..=8.
    pub level: u8,
    pub props: ParaProps,
    pub runs: Vec<Run>,
    /// `a:endParaRPr`.
    pub end_props: Option<RunProps>,
    pub source: Option<Arc<Element>>,
}

impl Paragraph {
    pub fn plain_text(&self) -> String {
        self.runs.iter().map(Run::as_str).collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AutoFit {
    #[default]
    None,
    /// `normAutofit`: shrink text (font scale and line spacing reduction,
    /// 1000ths of a percent).
    Normal { font_scale: i32, line_reduction: i32 },
    /// `spAutoFit`: the shape grows with its text.
    Shape,
}

/// `a:bodyPr`; `None` means "inherit".
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BodyProps {
    pub anchor: Option<Anchor>,
    pub anchor_ctr: Option<bool>,
    /// Insets l, t, r, b.
    pub insets: [Option<Emu>; 4],
    pub wrap: Option<bool>,
    pub autofit: Option<AutoFit>,
    /// Text rotation (60000ths of a degree).
    pub rot: Option<i32>,
    /// `horz`, `vert`, `vert270`, ...
    pub vert: Option<String>,
    pub columns: Option<u32>,
}

impl BodyProps {
    pub fn inherit(&mut self, p: &BodyProps) {
        macro_rules! inh {
            ($($f:ident),*) => { $( if self.$f.is_none() { self.$f = p.$f.clone(); } )* };
        }
        inh!(anchor, anchor_ctr, wrap, autofit, rot, vert, columns);
        for i in 0..4 {
            if self.insets[i].is_none() {
                self.insets[i] = p.insets[i];
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextBody {
    pub body: BodyProps,
    pub list_style: ListStyle,
    pub paragraphs: Vec<Paragraph>,
    pub source_body_pr: Option<Arc<Element>>,
    pub source_lst_style: Option<Arc<Element>>,
}

impl TextBody {
    pub fn plain_text(&self) -> String {
        self.paragraphs.iter().map(Paragraph::plain_text).collect::<Vec<_>>().join("\n")
    }
    pub fn is_empty(&self) -> bool {
        self.paragraphs.iter().all(|p| p.runs.iter().all(|r| r.as_str().is_empty()))
    }
}

// --------------------------------------------------------------- shapes ---

/// Placeholder reference (`p:ph`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placeholder {
    /// `title`, `ctrTitle`, `subTitle`, `body`, `obj`, `dt`, `ftr`, `sldNum`,
    /// `pic`, `tbl`, `chart`, ... (`obj` when absent).
    pub kind: String,
    pub idx: Option<u32>,
}

impl Placeholder {
    pub fn is_title(&self) -> bool {
        matches!(self.kind.as_str(), "title" | "ctrTitle")
    }
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct SpProps {
    pub geom: Option<Geometry>,
    pub fill: Option<Fill>,
    pub line: Option<Line>,
    pub style: Option<StyleRefs>,
    pub shadow: Option<Shadow>,
    pub text: Option<TextBody>,
    /// `txBox="1"`: created as a text box.
    pub text_box: bool,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct PicProps {
    pub image: ImageFill,
    pub geom: Option<Geometry>,
    pub line: Option<Line>,
    pub style: Option<StyleRefs>,
    pub shadow: Option<Shadow>,
    /// Linked audio/video part, when the picture is a media poster.
    pub media: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct GroupProps {
    /// Child coordinate space (`a:chOff`, `a:chExt`).
    pub child: Xfrm,
    pub fill: Option<Fill>,
    pub shapes: Vec<Shape>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableCell {
    pub text: TextBody,
    pub fill: Option<Fill>,
    /// Borders l, r, t, b.
    pub borders: [Option<Line>; 4],
    pub grid_span: u32,
    pub row_span: u32,
    pub h_merge: bool,
    pub v_merge: bool,
    pub margins: [Option<Emu>; 4],
    pub anchor: Option<Anchor>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TableRow {
    pub height: Emu,
    pub cells: Vec<TableCell>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Table {
    pub cols: Vec<Emu>,
    pub rows: Vec<TableRow>,
    pub first_row: bool,
    pub band_row: bool,
    pub style_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FrameContent {
    Table(Table),
    Chart { part: Option<String> },
    Diagram,
    Ole,
    Other(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct FrameProps {
    pub content: FrameContent,
    /// Pre-rendered shapes (SmartArt drawing part) in the frame's local
    /// coordinates, used for display only.
    pub drawing: Vec<Shape>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ShapeKind {
    Shape(Box<SpProps>),
    Connector(Box<SpProps>),
    Picture(Box<PicProps>),
    Group(Box<GroupProps>),
    Frame(Box<FrameProps>),
    /// Element Kinaute does not model (kept verbatim); `fallback` holds
    /// shapes parsed from an `mc:Fallback` branch, used for display only.
    Opaque {
        fallback: Vec<Shape>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    pub id: u32,
    pub name: String,
    pub descr: Option<String>,
    pub hidden: bool,
    pub ph: Option<Placeholder>,
    pub xfrm: Option<Xfrm>,
    pub kind: ShapeKind,
    /// Original XML element, used to write back everything Kinaute does not
    /// model.
    pub source: Option<Arc<Element>>,
}

impl Shape {
    pub fn sp(&self) -> Option<&SpProps> {
        match &self.kind {
            ShapeKind::Shape(s) | ShapeKind::Connector(s) => Some(s),
            _ => None,
        }
    }
    pub fn sp_mut(&mut self) -> Option<&mut SpProps> {
        match &mut self.kind {
            ShapeKind::Shape(s) | ShapeKind::Connector(s) => Some(s),
            _ => None,
        }
    }
    pub fn text(&self) -> Option<&TextBody> {
        self.sp().and_then(|s| s.text.as_ref())
    }
    pub fn is_group(&self) -> bool {
        matches!(self.kind, ShapeKind::Group(_))
    }
}

// --------------------------------------------------------------- slides ---

#[derive(Debug, Clone, PartialEq)]
pub enum Background {
    Fill(Fill),
    /// `p:bgRef` into the theme's background fill styles.
    Ref {
        idx: u32,
        color: Option<Color>,
    },
}

/// Slide transition (`p:transition`).
#[derive(Debug, Clone, PartialEq)]
pub struct Transition {
    /// `fade`, `push`, `wipe`, `cover`, `pull`, `split`, `cut`, `dissolve`,
    /// `zoom`, `morph`, ... (`none` when only timing is set).
    pub kind: String,
    /// Direction (`l`, `r`, `u`, `d`, ...) when relevant.
    pub dir: Option<String>,
    pub duration_ms: u32,
    pub advance_on_click: bool,
    pub advance_after_ms: Option<u32>,
}

impl Default for Transition {
    fn default() -> Self {
        Transition { kind: "fade".into(), dir: None, duration_ms: 700, advance_on_click: true, advance_after_ms: None }
    }
}

/// A package relationship of a slide (kept so duplicated slides can be written).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rel {
    pub id: String,
    /// Full relationship type URI.
    pub ty: String,
    /// Resolved part name (`ppt/media/image1.png`), or the raw URL when external.
    pub target: String,
    pub external: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Slide {
    /// Original relationships of the slide part.
    pub rels: Vec<Rel>,
    /// Package part name (`ppt/slides/slide1.xml`), `None` for new slides.
    pub part: Option<String>,
    pub layout: usize,
    pub name: Option<String>,
    pub background: Option<Background>,
    pub shapes: Vec<Shape>,
    pub transition: Option<Transition>,
    pub hidden: bool,
    pub show_master_shapes: bool,
    /// Speaker notes (plain text, read-only for now).
    pub notes: Option<String>,
    pub source: Option<Arc<Element>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    pub part: String,
    pub name: String,
    pub kind: Option<String>,
    pub master: usize,
    pub background: Option<Background>,
    pub shapes: Vec<Shape>,
    pub show_master_shapes: bool,
    pub clr_map_override: Option<ClrMap>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct TextStyles {
    pub title: ListStyle,
    pub body: ListStyle,
    pub other: ListStyle,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Master {
    pub part: String,
    pub theme: usize,
    pub background: Option<Background>,
    pub shapes: Vec<Shape>,
    pub clr_map: ClrMap,
    pub text_styles: TextStyles,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct FontScheme {
    pub major: String,
    pub minor: String,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Theme {
    pub part: String,
    pub name: String,
    pub colors: ColorScheme,
    pub fonts: FontScheme,
    pub fill_styles: Vec<Fill>,
    pub line_styles: Vec<Line>,
    pub bg_fill_styles: Vec<Fill>,
    pub effect_shadows: Vec<Option<Shadow>>,
}

/// Binary media of the package (images, audio, video), shared by reference.
pub type MediaStore = BTreeMap<String, Arc<Vec<u8>>>;

#[derive(Debug, Clone, PartialEq)]
pub struct Presentation {
    pub slide_w: Emu,
    pub slide_h: Emu,
    pub slides: Vec<Arc<Slide>>,
    pub layouts: Vec<Arc<Layout>>,
    pub masters: Vec<Arc<Master>>,
    pub themes: Vec<Arc<Theme>>,
    /// Default text style of the presentation (`p:defaultTextStyle`).
    pub default_text: Arc<ListStyle>,
    pub media: Arc<MediaStore>,
}

impl Presentation {
    pub fn layout_of(&self, slide: &Slide) -> Option<&Layout> {
        self.layouts.get(slide.layout).map(|l| l.as_ref())
    }
    pub fn master_of_layout(&self, layout: &Layout) -> Option<&Master> {
        self.masters.get(layout.master).map(|m| m.as_ref())
    }
    pub fn theme_of_master(&self, master: &Master) -> Option<&Theme> {
        self.themes.get(master.theme).map(|t| t.as_ref())
    }
}
