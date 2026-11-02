//! Commands: every change to a document goes through [`Command::apply`],
//! and [`History`] records snapshots for unlimited undo/redo. Snapshots are
//! cheap because slides are shared (`Arc`) and only the touched slide is
//! copied.

use crate::types::*;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditError {
    NoSlide(usize),
    NoShape(u32),
    Invalid(&'static str),
}

impl std::fmt::Display for EditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EditError::NoSlide(i) => write!(f, "slide {i} does not exist"),
            EditError::NoShape(i) => write!(f, "shape {i} does not exist"),
            EditError::Invalid(s) => write!(f, "invalid edit: {s}"),
        }
    }
}

impl std::error::Error for EditError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZOrder {
    Front,
    Back,
    Forward,
    Backward,
}

/// Partial change of character properties applied to whole shapes.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RunPatch {
    pub size: Option<i32>,
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub underline: Option<bool>,
    pub color: Option<crate::color::Color>,
    pub font: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    /// Set transforms (move, resize, rotate) of top-level shapes.
    SetXfrm {
        slide: usize,
        items: Vec<(u32, Xfrm)>,
    },
    SetFill {
        slide: usize,
        ids: Vec<u32>,
        fill: Option<Fill>,
    },
    SetLine {
        slide: usize,
        ids: Vec<u32>,
        line: Option<Line>,
    },
    /// Replace the text of a shape, keeping the formatting of the first run
    /// of each existing paragraph.
    SetText {
        slide: usize,
        id: u32,
        text: String,
    },
    SetRunProps {
        slide: usize,
        ids: Vec<u32>,
        patch: RunPatch,
    },
    SetAlign {
        slide: usize,
        ids: Vec<u32>,
        align: Align,
    },
    SetAnchor {
        slide: usize,
        ids: Vec<u32>,
        anchor: Anchor,
    },
    InsertShapes {
        slide: usize,
        shapes: Vec<Shape>,
    },
    DeleteShapes {
        slide: usize,
        ids: Vec<u32>,
    },
    Reorder {
        slide: usize,
        ids: Vec<u32>,
        how: ZOrder,
    },
    Group {
        slide: usize,
        ids: Vec<u32>,
    },
    Ungroup {
        slide: usize,
        id: u32,
    },
    InsertSlide {
        at: usize,
        slide: Box<Slide>,
    },
    DuplicateSlide {
        index: usize,
    },
    DeleteSlide {
        index: usize,
    },
    MoveSlide {
        from: usize,
        to: usize,
    },
    SetTransition {
        slides: Vec<usize>,
        transition: Option<Transition>,
    },
    SetHidden {
        slide: usize,
        hidden: bool,
    },
    SetBackground {
        slide: usize,
        background: Option<Background>,
    },
    AddMedia {
        part: String,
        bytes: Arc<Vec<u8>>,
    },
    SetSlideSize {
        w: Emu,
        h: Emu,
    },
}

impl Command {
    /// Human-readable label (for the Edit menu).
    pub fn label(&self) -> &'static str {
        match self {
            Command::SetXfrm { .. } => "Déplacer",
            Command::SetFill { .. } => "Remplissage",
            Command::SetLine { .. } => "Contour",
            Command::SetText { .. } => "Texte",
            Command::SetRunProps { .. } => "Format du texte",
            Command::SetAlign { .. } => "Alignement",
            Command::SetAnchor { .. } => "Alignement vertical",
            Command::InsertShapes { .. } => "Insérer",
            Command::DeleteShapes { .. } => "Supprimer",
            Command::Reorder { .. } => "Ordre",
            Command::Group { .. } => "Grouper",
            Command::Ungroup { .. } => "Dissocier",
            Command::InsertSlide { .. } => "Nouvelle diapositive",
            Command::DuplicateSlide { .. } => "Dupliquer la diapositive",
            Command::DeleteSlide { .. } => "Supprimer la diapositive",
            Command::MoveSlide { .. } => "Déplacer la diapositive",
            Command::SetTransition { .. } => "Transition",
            Command::SetHidden { .. } => "Masquer la diapositive",
            Command::SetBackground { .. } => "Arrière-plan",
            Command::AddMedia { .. } => "Ajouter un média",
            Command::SetSlideSize { .. } => "Taille des diapositives",
        }
    }

    pub fn apply(&self, p: &mut Presentation) -> Result<(), EditError> {
        match self {
            Command::SetXfrm { slide, items } => {
                let s = slide_mut(p, *slide)?;
                for (id, x) in items {
                    find_mut(&mut s.shapes, *id).ok_or(EditError::NoShape(*id))?.xfrm = Some(*x);
                }
            }
            Command::SetFill { slide, ids, fill } => {
                let s = slide_mut(p, *slide)?;
                for id in ids {
                    let sh = find_mut(&mut s.shapes, *id).ok_or(EditError::NoShape(*id))?;
                    match &mut sh.kind {
                        ShapeKind::Shape(sp) | ShapeKind::Connector(sp) => sp.fill = fill.clone(),
                        ShapeKind::Group(g) => g.fill = fill.clone(),
                        _ => {}
                    }
                }
            }
            Command::SetLine { slide, ids, line } => {
                let s = slide_mut(p, *slide)?;
                for id in ids {
                    let sh = find_mut(&mut s.shapes, *id).ok_or(EditError::NoShape(*id))?;
                    match &mut sh.kind {
                        ShapeKind::Shape(sp) | ShapeKind::Connector(sp) => sp.line = line.clone(),
                        ShapeKind::Picture(pic) => pic.line = line.clone(),
                        _ => {}
                    }
                }
            }
            Command::SetText { slide, id, text } => {
                let s = slide_mut(p, *slide)?;
                let sh = find_mut(&mut s.shapes, *id).ok_or(EditError::NoShape(*id))?;
                let sp = sh.sp_mut().ok_or(EditError::Invalid("not a text shape"))?;
                let body = sp.text.get_or_insert_with(TextBody::default);
                set_plain_text(body, text);
            }
            Command::SetRunProps { slide, ids, patch } => {
                let s = slide_mut(p, *slide)?;
                for id in ids {
                    let sh = find_mut(&mut s.shapes, *id).ok_or(EditError::NoShape(*id))?;
                    for_each_text(sh, &mut |body| {
                        for para in &mut body.paragraphs {
                            for run in &mut para.runs {
                                patch_run(&mut run.props, patch);
                            }
                            if let Some(e) = &mut para.end_props {
                                patch_run(e, patch);
                            }
                        }
                    });
                }
            }
            Command::SetAlign { slide, ids, align } => {
                let s = slide_mut(p, *slide)?;
                for id in ids {
                    let sh = find_mut(&mut s.shapes, *id).ok_or(EditError::NoShape(*id))?;
                    for_each_text(sh, &mut |body| {
                        for para in &mut body.paragraphs {
                            para.props.align = Some(*align);
                        }
                    });
                }
            }
            Command::SetAnchor { slide, ids, anchor } => {
                let s = slide_mut(p, *slide)?;
                for id in ids {
                    let sh = find_mut(&mut s.shapes, *id).ok_or(EditError::NoShape(*id))?;
                    if let Some(sp) = sh.sp_mut() {
                        sp.text.get_or_insert_with(TextBody::default).body.anchor = Some(*anchor);
                    }
                }
            }
            Command::InsertShapes { slide, shapes } => {
                let s = slide_mut(p, *slide)?;
                for sh in shapes {
                    let mut sh = sh.clone();
                    if find(&s.shapes, sh.id).is_some() || sh.id == 0 {
                        let old = format!(" {}", sh.id);
                        sh.id = next_id(&s.shapes);
                        if let Some(stem) = sh.name.strip_suffix(&old) {
                            sh.name = format!("{stem} {}", sh.id);
                        }
                    }
                    s.shapes.push(sh);
                }
            }
            Command::DeleteShapes { slide, ids } => {
                let s = slide_mut(p, *slide)?;
                remove_ids(&mut s.shapes, ids);
            }
            Command::Reorder { slide, ids, how } => {
                let s = slide_mut(p, *slide)?;
                reorder(&mut s.shapes, ids, *how);
            }
            Command::Group { slide, ids } => {
                let s = slide_mut(p, *slide)?;
                group(&mut s.shapes, ids)?;
            }
            Command::Ungroup { slide, id } => {
                let s = slide_mut(p, *slide)?;
                ungroup(&mut s.shapes, *id)?;
            }
            Command::InsertSlide { at, slide } => {
                let at = (*at).min(p.slides.len());
                p.slides.insert(at, Arc::new((**slide).clone()));
            }
            Command::DuplicateSlide { index } => {
                let s = p.slides.get(*index).ok_or(EditError::NoSlide(*index))?;
                let mut copy = (**s).clone();
                copy.part = None;
                copy.notes = None;
                p.slides.insert(index + 1, Arc::new(copy));
            }
            Command::DeleteSlide { index } => {
                if *index >= p.slides.len() {
                    return Err(EditError::NoSlide(*index));
                }
                p.slides.remove(*index);
            }
            Command::MoveSlide { from, to } => {
                if *from >= p.slides.len() {
                    return Err(EditError::NoSlide(*from));
                }
                let s = p.slides.remove(*from);
                let to = (*to).min(p.slides.len());
                p.slides.insert(to, s);
            }
            Command::SetTransition { slides, transition } => {
                for i in slides {
                    slide_mut(p, *i)?.transition = transition.clone();
                }
            }
            Command::SetHidden { slide, hidden } => slide_mut(p, *slide)?.hidden = *hidden,
            Command::SetBackground { slide, background } => slide_mut(p, *slide)?.background = background.clone(),
            Command::AddMedia { part, bytes } => {
                Arc::make_mut(&mut p.media).insert(part.clone(), bytes.clone());
            }
            Command::SetSlideSize { w, h } => {
                if *w <= 0 || *h <= 0 {
                    return Err(EditError::Invalid("slide size must be positive"));
                }
                p.slide_w = *w;
                p.slide_h = *h;
            }
        }
        Ok(())
    }
}

fn slide_mut(p: &mut Presentation, i: usize) -> Result<&mut Slide, EditError> {
    p.slides.get_mut(i).map(Arc::make_mut).ok_or(EditError::NoSlide(i))
}

fn patch_run(r: &mut RunProps, patch: &RunPatch) {
    if let Some(v) = patch.size {
        r.size = Some(v);
    }
    if let Some(v) = patch.bold {
        r.bold = Some(v);
    }
    if let Some(v) = patch.italic {
        r.italic = Some(v);
    }
    if let Some(v) = patch.underline {
        r.underline = Some(if v { "sng".into() } else { "none".into() });
    }
    if let Some(c) = &patch.color {
        r.fill = Some(Fill::Solid(c.clone()));
    }
    if let Some(f) = &patch.font {
        r.font = Some(f.clone());
    }
}

fn for_each_text(sh: &mut Shape, f: &mut dyn FnMut(&mut TextBody)) {
    match &mut sh.kind {
        ShapeKind::Shape(sp) | ShapeKind::Connector(sp) => {
            if let Some(t) = &mut sp.text {
                f(t);
            }
        }
        ShapeKind::Group(g) => {
            for c in &mut g.shapes {
                for_each_text(c, f);
            }
        }
        ShapeKind::Frame(fr) => {
            if let FrameContent::Table(t) = &mut fr.content {
                for row in &mut t.rows {
                    for cell in &mut row.cells {
                        f(&mut cell.text);
                    }
                }
            }
        }
        _ => {}
    }
}

/// Replace the text of a body with `text` (`\n` separates paragraphs,
/// `\u{2028}` or `\u{b}` are line breaks), reusing existing formatting.
pub fn set_plain_text(body: &mut TextBody, text: &str) {
    if body.plain_text() == text {
        return;
    }
    let old = std::mem::take(&mut body.paragraphs);
    for (i, line) in text.split('\n').enumerate() {
        let tpl = old.get(i).or_else(|| old.last());
        let mut para = Paragraph {
            level: tpl.map(|p| p.level).unwrap_or(0),
            props: tpl.map(|p| p.props.clone()).unwrap_or_default(),
            runs: Vec::new(),
            end_props: tpl.and_then(|p| p.end_props.clone()),
            source: None,
        };
        // Keep the paragraph's source when its text is unchanged.
        if let Some(t) = old.get(i) {
            if t.plain_text() == line {
                body.paragraphs.push(t.clone());
                continue;
            }
        }
        let run_tpl = tpl.and_then(|p| p.runs.iter().find(|r| matches!(r.kind, RunKind::Text(_))));
        let run_props =
            run_tpl.map(|r| r.props.clone()).or_else(|| tpl.and_then(|p| p.end_props.clone())).unwrap_or_default();
        for (j, seg) in line.split(['\u{2028}', '\u{b}']).enumerate() {
            if j > 0 {
                para.runs.push(Run { kind: RunKind::Break, props: run_props.clone(), source: None });
            }
            if !seg.is_empty() {
                para.runs.push(Run {
                    kind: RunKind::Text(seg.to_string()),
                    props: run_props.clone(),
                    source: run_tpl.and_then(|r| r.source.clone()),
                });
            }
        }
        if para.end_props.is_none() && !run_props.is_default() {
            para.end_props = Some(run_props);
        }
        body.paragraphs.push(para);
    }
}

impl RunProps {
    pub fn is_default(&self) -> bool {
        *self == RunProps::default()
    }
}

/// Find a shape anywhere in a tree by id.
pub fn find(shapes: &[Shape], id: u32) -> Option<&Shape> {
    for s in shapes {
        if s.id == id {
            return Some(s);
        }
        if let ShapeKind::Group(g) = &s.kind {
            if let Some(f) = find(&g.shapes, id) {
                return Some(f);
            }
        }
    }
    None
}

pub fn find_mut(shapes: &mut [Shape], id: u32) -> Option<&mut Shape> {
    for s in shapes.iter_mut() {
        if s.id == id {
            return Some(s);
        }
        if let ShapeKind::Group(g) = &mut s.kind {
            if let Some(f) = find_mut(&mut g.shapes, id) {
                return Some(f);
            }
        }
    }
    None
}

fn max_id(shapes: &[Shape]) -> u32 {
    shapes
        .iter()
        .map(|s| match &s.kind {
            ShapeKind::Group(g) => s.id.max(max_id(&g.shapes)),
            _ => s.id,
        })
        .max()
        .unwrap_or(1)
}

/// A fresh shape id for a slide.
pub fn next_id(shapes: &[Shape]) -> u32 {
    max_id(shapes).saturating_add(1).max(2)
}

fn remove_ids(shapes: &mut Vec<Shape>, ids: &[u32]) {
    shapes.retain(|s| !ids.contains(&s.id));
    for s in shapes.iter_mut() {
        if let ShapeKind::Group(g) = &mut s.kind {
            remove_ids(&mut g.shapes, ids);
        }
    }
}

fn reorder(shapes: &mut Vec<Shape>, ids: &[u32], how: ZOrder) {
    if ids.is_empty() {
        return;
    }
    match how {
        ZOrder::Front | ZOrder::Back => {
            let (sel, rest): (Vec<Shape>, Vec<Shape>) =
                std::mem::take(shapes).into_iter().partition(|s| ids.contains(&s.id));
            *shapes = if how == ZOrder::Front {
                rest.into_iter().chain(sel).collect()
            } else {
                sel.into_iter().chain(rest).collect()
            };
        }
        ZOrder::Forward => {
            for i in (0..shapes.len().saturating_sub(1)).rev() {
                if ids.contains(&shapes[i].id) && !ids.contains(&shapes[i + 1].id) {
                    shapes.swap(i, i + 1);
                }
            }
        }
        ZOrder::Backward => {
            for i in 1..shapes.len() {
                if ids.contains(&shapes[i].id) && !ids.contains(&shapes[i - 1].id) {
                    shapes.swap(i, i - 1);
                }
            }
        }
    }
}

/// Axis-aligned bounds of a transform, taking rotation into account.
pub fn bounds(x: &Xfrm) -> (f64, f64, f64, f64) {
    let (cx, cy) = x.center();
    let a = x.rot_deg().to_radians();
    let (s, c) = (a.sin().abs(), a.cos().abs());
    let w = x.w as f64 * c + x.h as f64 * s;
    let h = x.w as f64 * s + x.h as f64 * c;
    (cx - w / 2.0, cy - h / 2.0, cx + w / 2.0, cy + h / 2.0)
}

fn group(shapes: &mut Vec<Shape>, ids: &[u32]) -> Result<(), EditError> {
    if ids.len() < 2 {
        return Err(EditError::Invalid("group needs at least two shapes"));
    }
    let new_id = next_id(shapes);
    let first = shapes.iter().position(|s| ids.contains(&s.id)).ok_or(EditError::Invalid("nothing to group"))?;
    let (sel, rest): (Vec<Shape>, Vec<Shape>) = std::mem::take(shapes).into_iter().partition(|s| ids.contains(&s.id));
    let mut b = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for s in &sel {
        if let Some(x) = &s.xfrm {
            let (l, t, r, bt) = bounds(x);
            b = (b.0.min(l), b.1.min(t), b.2.max(r), b.3.max(bt));
        }
    }
    if b.0 > b.2 {
        *shapes = rest;
        shapes.extend(sel);
        return Err(EditError::Invalid("shapes have no position"));
    }
    let x = Xfrm::new(b.0 as Emu, b.1 as Emu, (b.2 - b.0) as Emu, (b.3 - b.1) as Emu);
    let g = Shape {
        id: new_id,
        name: format!("Groupe {new_id}"),
        descr: None,
        hidden: false,
        ph: None,
        xfrm: Some(x),
        kind: ShapeKind::Group(Box::new(GroupProps { child: x, fill: None, shapes: sel })),
        source: None,
    };
    let mut rest = rest;
    rest.insert(first.min(rest.len()), g);
    *shapes = rest;
    Ok(())
}

fn ungroup(shapes: &mut Vec<Shape>, id: u32) -> Result<(), EditError> {
    let pos = shapes.iter().position(|s| s.id == id).ok_or(EditError::NoShape(id))?;
    let g = shapes.remove(pos);
    let (gx, gp) = match (g.xfrm, g.kind) {
        (Some(x), ShapeKind::Group(gp)) => (x, gp),
        (_, kind) => {
            shapes.insert(pos, Shape { kind, ..g });
            return Err(EditError::Invalid("not a group"));
        }
    };
    let ch = gp.child;
    let sx = if ch.w != 0 { gx.w as f64 / ch.w as f64 } else { 1.0 };
    let sy = if ch.h != 0 { gx.h as f64 / ch.h as f64 } else { 1.0 };
    let (gcx, gcy) = gx.center();
    let ga = gx.rot_deg().to_radians();
    let children: Vec<Shape> = gp
        .shapes
        .into_iter()
        .map(|mut c| {
            if let Some(cx) = c.xfrm {
                let nx = gx.x as f64 + (cx.x - ch.x) as f64 * sx;
                let ny = gx.y as f64 + (cx.y - ch.y) as f64 * sy;
                let nw = cx.w as f64 * sx;
                let nh = cx.h as f64 * sy;
                // Rotate the child's centre around the group centre.
                let (ccx, ccy) = (nx + nw / 2.0 - gcx, ny + nh / 2.0 - gcy);
                let (rx, ry) = (ccx * ga.cos() - ccy * ga.sin() + gcx, ccx * ga.sin() + ccy * ga.cos() + gcy);
                c.xfrm = Some(Xfrm {
                    x: (rx - nw / 2.0) as Emu,
                    y: (ry - nh / 2.0) as Emu,
                    w: nw as Emu,
                    h: nh as Emu,
                    rot: (cx.rot + gx.rot).rem_euclid(21_600_000),
                    flip_h: cx.flip_h ^ gx.flip_h,
                    flip_v: cx.flip_v ^ gx.flip_v,
                });
            }
            c
        })
        .collect();
    for (i, c) in children.into_iter().enumerate() {
        shapes.insert(pos + i, c);
    }
    Ok(())
}

/// Undo/redo history of whole-document snapshots.
#[derive(Debug, Clone)]
pub struct History {
    current: Presentation,
    undo: Vec<(&'static str, Presentation)>,
    redo: Vec<(&'static str, Presentation)>,
    /// Key of the last coalescable command (e.g. an ongoing drag).
    coalesce: Option<u64>,
    /// Number of edits since the last save (negative after undo past it).
    generation: u64,
    saved_generation: u64,
    /// Bumped on every change of the current document (edit, undo or redo).
    revision: u64,
}

/// Cheap document equality: shared (`Arc`) parts are compared by pointer
/// first, so a one-slide edit never walks the whole presentation.
fn same(a: &Presentation, b: &Presentation) -> bool {
    fn arcs<T: PartialEq>(x: &[Arc<T>], y: &[Arc<T>]) -> bool {
        x.len() == y.len() && x.iter().zip(y).all(|(p, q)| Arc::ptr_eq(p, q) || **p == **q)
    }
    a.slide_w == b.slide_w
        && a.slide_h == b.slide_h
        && arcs(&a.slides, &b.slides)
        && arcs(&a.layouts, &b.layouts)
        && arcs(&a.masters, &b.masters)
        && arcs(&a.themes, &b.themes)
        && Arc::ptr_eq(&a.default_text, &b.default_text)
        && (Arc::ptr_eq(&a.media, &b.media) || a.media.len() == b.media.len())
}

impl History {
    pub fn new(p: Presentation) -> Self {
        History {
            current: p,
            undo: Vec::new(),
            redo: Vec::new(),
            coalesce: None,
            generation: 0,
            saved_generation: 0,
            revision: 0,
        }
    }

    pub fn current(&self) -> &Presentation {
        &self.current
    }

    /// Apply a command and record it.
    pub fn apply(&mut self, cmd: Command) -> Result<(), EditError> {
        self.apply_coalesced(cmd, None)
    }

    /// Apply a command; consecutive commands with the same `key` merge into a
    /// single undo step (used for drags and typing).
    pub fn apply_coalesced(&mut self, cmd: Command, key: Option<u64>) -> Result<(), EditError> {
        let mut next = self.current.clone();
        cmd.apply(&mut next)?;
        if same(&next, &self.current) {
            return Ok(());
        }
        self.revision += 1;
        let merge = key.is_some() && key == self.coalesce && !self.undo.is_empty();
        let prev = std::mem::replace(&mut self.current, next);
        if !merge {
            self.undo.push((cmd.label(), prev));
            self.generation = self.generation.wrapping_add(1);
        }
        self.redo.clear();
        self.coalesce = key;
        Ok(())
    }

    /// End any coalescing in progress.
    pub fn seal(&mut self) {
        self.coalesce = None;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
    pub fn undo_label(&self) -> Option<&'static str> {
        self.undo.last().map(|(l, _)| *l)
    }
    pub fn redo_label(&self) -> Option<&'static str> {
        self.redo.last().map(|(l, _)| *l)
    }

    pub fn undo(&mut self) -> bool {
        self.coalesce = None;
        match self.undo.pop() {
            Some((label, prev)) => {
                let cur = std::mem::replace(&mut self.current, prev);
                self.redo.push((label, cur));
                self.generation = self.generation.wrapping_sub(1);
                self.revision += 1;
                true
            }
            None => false,
        }
    }

    pub fn redo(&mut self) -> bool {
        self.coalesce = None;
        match self.redo.pop() {
            Some((label, next)) => {
                let cur = std::mem::replace(&mut self.current, next);
                self.undo.push((label, cur));
                self.generation = self.generation.wrapping_add(1);
                self.revision += 1;
                true
            }
            None => false,
        }
    }

    /// Changes whenever the current document changes.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn mark_saved(&mut self) {
        self.saved_generation = self.generation;
    }

    pub fn is_dirty(&self) -> bool {
        self.generation != self.saved_generation
    }
}
