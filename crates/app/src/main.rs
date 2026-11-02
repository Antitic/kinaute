//! Kinaute — lightweight presentation editor working directly on `.pptx`.

mod app;
mod present;

#[allow(unsafe_code, clippy::unwrap_used, clippy::expect_used, clippy::all, clippy::pedantic)]
mod ui {
    slint::include_modules!();
}

use app::{App, EMU_CM, EMU_PT};
use kinaute_model::edit::RunPatch;
use kinaute_model::Slide;
use kinaute_render::Renderer;
use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};
use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use ui::{MainWindow, SlideItem};

const THUMB_W: u32 = 196;

struct Shell {
    app: RefCell<App>,
    ui: slint::Weak<MainWindow>,
    slides: Rc<VecModel<SlideItem>>,
    thumbs: RefCell<Vec<Option<Arc<Slide>>>>,
    renderer: Rc<RefCell<Renderer>>,
    present: RefCell<Option<Rc<RefCell<present::Present>>>>,
    last_autosave_rev: Cell<u64>,
    transition_kinds: Vec<&'static str>,
}

fn color_of(c: [u8; 4]) -> slint::Color {
    slint::Color::from_argb_u8(c[3], c[0], c[1], c[2])
}

fn rgb_of(c: slint::Color) -> [u8; 3] {
    [c.red(), c.green(), c.blue()]
}

fn cm(v: i64) -> String {
    format!("{:.2}", v as f64 / EMU_CM)
}

fn parse_f(s: &str) -> Option<f64> {
    s.trim().replace(',', ".").parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Ask the desktop for a file path with zenity or kdialog when available.
fn native_dialog(save: bool, start: &str) -> Option<Option<PathBuf>> {
    use std::process::Command;
    let try_cmd = |mut c: Command| -> Option<Option<PathBuf>> {
        match c.output() {
            Ok(o) if o.status.success() => {
                let p = String::from_utf8_lossy(&o.stdout).trim().to_string();
                Some(if p.is_empty() { None } else { Some(PathBuf::from(p)) })
            }
            Ok(_) => Some(None), // cancelled
            Err(_) => None,      // not installed
        }
    };
    let mut z = Command::new("zenity");
    z.arg("--file-selection").arg(format!("--filename={start}")).arg("--file-filter=Présentations | *.pptx");
    if save {
        z.arg("--save").arg("--confirm-overwrite");
    }
    if let Some(r) = try_cmd(z) {
        return Some(r);
    }
    let mut k = Command::new("kdialog");
    k.arg(if save { "--getsavefilename" } else { "--getopenfilename" }).arg(start).arg("*.pptx");
    try_cmd(k)
}

#[derive(Clone, Copy, PartialEq)]
enum Pending {
    Open,
    SaveAs,
    Image,
    Pdf,
}

impl Shell {
    fn ui(&self) -> Option<MainWindow> {
        self.ui.upgrade()
    }

    fn scale(&self) -> f32 {
        self.ui().map(|u| u.window().scale_factor()).unwrap_or(1.0)
    }

    // --------------------------------------------------------- refresh --

    fn refresh_canvas(&self) {
        let Some(ui) = self.ui() else { return };
        let mut a = self.app.borrow_mut();
        if let Some(pm) = a.render_canvas() {
            ui.set_canvas(present::to_image(&pm));
        }
        ui.set_can_undo(a.doc.history.can_undo());
        ui.set_can_redo(a.doc.history.can_redo());
        ui.set_zoom_percent(a.zoom_percent());
        ui.set_status(SharedString::from(a.status.as_str()));
    }

    fn refresh_title(&self) {
        let Some(ui) = self.ui() else { return };
        let a = self.app.borrow();
        let name = a
            .path
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "Sans titre".into());
        ui.set_doc_title(SharedString::from(format!("{}{} — Kinaute", name, if a.dirty() { " •" } else { "" })));
    }

    fn refresh_inspector(&self) {
        let Some(ui) = self.ui() else { return };
        let a = self.app.borrow();
        let info = a.sel_info();
        ui.set_has_sel(info.count > 0);
        ui.set_sel_count(info.count as i32);
        ui.set_has_text(info.has_text);
        if ui.get_sel_text().as_str() != info.text {
            ui.set_sel_text(SharedString::from(info.text.as_str()));
        }
        ui.set_font_size(SharedString::from(format!("{}", (info.size_pt * 10.0).round() / 10.0)));
        ui.set_is_bold(info.bold);
        ui.set_is_italic(info.italic);
        ui.set_is_underline(info.underline);
        ui.set_h_align(info.h_align);
        ui.set_v_align(info.v_align);
        ui.set_has_fill(info.fill.is_some());
        if let Some(f) = info.fill {
            ui.set_fill_color(color_of(f));
        }
        ui.set_has_line(info.line.is_some());
        if let Some((c, w)) = info.line {
            ui.set_line_color(color_of(c));
            ui.set_line_width(SharedString::from(format!("{:.1}", w)));
        }
        if let Some(x) = info.xfrm {
            ui.set_px(SharedString::from(cm(x.x)));
            ui.set_py(SharedString::from(cm(x.y)));
            ui.set_pw(SharedString::from(cm(x.w)));
            ui.set_ph(SharedString::from(cm(x.h)));
            ui.set_prot(SharedString::from(format!("{:.0}", x.rot_deg())));
        }
        ui.set_is_group(info.is_group);
        let t = a.current_transition();
        let kind = t.as_ref().map(|t| t.kind.as_str()).unwrap_or("none");
        ui.set_transition_index(self.transition_kinds.iter().position(|k| *k == kind).unwrap_or(0) as i32);
        ui.set_transition_duration(SharedString::from(format!(
            "{:.1}",
            t.as_ref().map(|t| t.duration_ms).unwrap_or(700) as f32 / 1000.0
        )));
        ui.set_advance_on_click(t.as_ref().map(|t| t.advance_on_click).unwrap_or(true));
        ui.set_slide_hidden(a.pres().slides.get(a.cur).map(|s| s.hidden).unwrap_or(false));
        ui.set_tool(SharedString::from(a.tool.as_str()));
        ui.set_current_slide(a.cur as i32);
    }

    /// Reconcile the thumbnail list with the document.
    fn sync_slides(&self) {
        let a = self.app.borrow();
        let n = a.slide_count();
        let mut thumbs = self.thumbs.borrow_mut();
        while self.slides.row_count() > n {
            self.slides.remove(self.slides.row_count() - 1);
        }
        thumbs.truncate(n);
        for i in 0..n {
            let slide = &a.pres().slides[i];
            let stale = !matches!(thumbs.get(i), Some(Some(s)) if Arc::ptr_eq(s, slide));
            let item = SlideItem {
                thumb: self.slides.row_data(i).map(|r| r.thumb).unwrap_or_default(),
                number: i as i32 + 1,
                selected: i == a.cur,
                hidden: slide.hidden,
            };
            if i < self.slides.row_count() {
                self.slides.set_row_data(i, item);
            } else {
                self.slides.push(item);
            }
            if i >= thumbs.len() {
                thumbs.push(None);
            } else if stale {
                thumbs[i] = None;
            }
        }
        if let Some(ui) = self.ui() {
            ui.set_slide_count(n as i32);
        }
    }

    /// Render one missing thumbnail; returns true while work remains.
    fn thumb_step(&self) -> bool {
        let (target, pres) = {
            let a = self.app.borrow();
            let thumbs = self.thumbs.borrow();
            // Current slide first, then the others in order.
            let order = std::iter::once(a.cur).chain((0..a.slide_count()).filter(|i| *i != a.cur));
            let t = order.into_iter().find(|i| matches!(thumbs.get(*i), Some(None)));
            (t, a.pres().clone())
        };
        let Some(i) = target else { return false };
        let pm = {
            let mut r = self.renderer.borrow_mut();
            r.show_prompts = true;
            let pm = r.render_slide(&pres, i, THUMB_W);
            r.show_prompts = false;
            pm
        };
        if let (Some(pm), Some(slide)) = (pm, pres.slides.get(i)) {
            if i < self.slides.row_count() {
                if let Some(mut row) = self.slides.row_data(i) {
                    row.thumb = present::to_image(&pm);
                    self.slides.set_row_data(i, row);
                }
            }
            if let Some(t) = self.thumbs.borrow_mut().get_mut(i) {
                *t = Some(slide.clone());
            }
        } else if let Some(t) = self.thumbs.borrow_mut().get_mut(i) {
            *t = Some(pres.slides[i].clone());
        }
        true
    }

    fn refresh_all(&self) {
        self.sync_slides();
        self.refresh_inspector();
        self.refresh_canvas();
        self.refresh_title();
    }

    // --------------------------------------------------------- actions --

    fn dialog(&self, what: Pending, title: &str, initial: String) {
        // Native dialog when available, otherwise the in-app path prompt.
        let save = matches!(what, Pending::SaveAs | Pending::Pdf);
        let native = if what == Pending::Image { None } else { native_dialog(save, &initial) };
        match native {
            Some(Some(p)) => self.complete(what, p),
            Some(None) => {}
            None => {
                if let Some(ui) = self.ui() {
                    ui.set_dialog_title(SharedString::from(title));
                    ui.set_dialog_text(SharedString::from(initial));
                    ui.set_dialog_open(true);
                    PENDING.with(|p| p.set(Some(what)));
                }
            }
        }
    }

    fn complete(&self, what: Pending, path: PathBuf) {
        match what {
            Pending::Open => self.open_path(path),
            Pending::SaveAs => {
                let mut path = path;
                if path.extension().is_none() {
                    path.set_extension("pptx");
                }
                self.save_path(path);
            }
            Pending::Image => {
                let r = self.app.borrow_mut().add_image(&path);
                if let Err(e) = r {
                    self.app.borrow_mut().status = e;
                }
                self.refresh_all();
            }
            Pending::Pdf => self.export_pdf(path),
        }
    }

    fn open_path(&self, path: PathBuf) {
        let result = std::fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|b| kinaute_pptx::open(&b).map_err(|e| e.to_string()));
        match result {
            Ok(doc) => {
                let report = doc.report.len();
                for r in &doc.report {
                    tracing::info!("import: {r}");
                }
                let mut app = App::new(doc, Some(path), self.renderer.clone());
                if let Some(ui) = self.ui() {
                    let sf = ui.window().scale_factor();
                    app.set_canvas((ui.get_canvas().size().width).max(64), (ui.get_canvas().size().height).max(64));
                    let _ = sf;
                }
                if report > 0 {
                    app.status = format!("Ouvert — {report} remarque(s) d'import (voir la console)");
                }
                self.thumbs.borrow_mut().clear();
                while self.slides.row_count() > 0 {
                    self.slides.remove(0);
                }
                let canvas = self.app.borrow().canvas;
                app.set_canvas(canvas.0, canvas.1);
                *self.app.borrow_mut() = app;
                self.refresh_all();
            }
            Err(e) => {
                self.app.borrow_mut().status = format!("Impossible d'ouvrir : {e}");
                self.refresh_canvas();
            }
        }
    }

    fn new_doc(&self) {
        match kinaute_pptx::blank() {
            Ok(doc) => {
                let canvas = self.app.borrow().canvas;
                let mut app = App::new(doc, None, self.renderer.clone());
                app.set_canvas(canvas.0, canvas.1);
                self.thumbs.borrow_mut().clear();
                while self.slides.row_count() > 0 {
                    self.slides.remove(0);
                }
                *self.app.borrow_mut() = app;
                self.refresh_all();
            }
            Err(e) => {
                self.app.borrow_mut().status = format!("Erreur : {e}");
            }
        }
    }

    fn save_path(&self, path: PathBuf) {
        let r = self.app.borrow_mut().save_to(path.clone());
        self.app.borrow_mut().status = match r {
            Ok(()) => format!("Enregistré : {}", path.display()),
            Err(e) => format!("Échec de l'enregistrement : {e}"),
        };
        self.refresh_title();
        self.refresh_canvas();
    }

    fn save(&self) {
        let p = self.app.borrow().path.clone();
        match p {
            Some(p) => self.save_path(p),
            None => self.save_as(),
        }
    }

    fn save_as(&self) {
        let start =
            self.app.borrow().path.as_ref().map(|p| p.display().to_string()).unwrap_or_else(|| {
                format!("{}/presentation.pptx", std::env::var("HOME").unwrap_or_else(|_| ".".into()))
            });
        self.dialog(Pending::SaveAs, "Enregistrer sous…", start);
    }

    fn export_pdf(&self, path: PathBuf) {
        let pres = self.app.borrow().pres().clone();
        let mut pages = Vec::new();
        {
            let mut r = self.renderer.borrow_mut();
            r.show_prompts = false;
            for i in 0..pres.slides.len() {
                if pres.slides[i].hidden {
                    continue;
                }
                if let Some(pm) = r.render_slide(&pres, i, 1600) {
                    if let Some(jpeg) = kinaute_render::pdf::pixmap_to_jpeg(&pm, 88) {
                        pages.push(kinaute_render::pdf::Page { jpeg, width_px: pm.width(), height_px: pm.height() });
                    }
                }
            }
        }
        let title = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let pdf = kinaute_render::pdf::write_pdf(
            &pages,
            pres.slide_w as f32 / EMU_PT as f32,
            pres.slide_h as f32 / EMU_PT as f32,
            &title,
        );
        let mut path = path;
        if path.extension().is_none() {
            path.set_extension("pdf");
        }
        self.app.borrow_mut().status = match std::fs::write(&path, pdf) {
            Ok(()) => format!("PDF exporté : {}", path.display()),
            Err(e) => format!("Échec de l'export PDF : {e}"),
        };
        self.refresh_canvas();
    }

    fn play(self: &Rc<Self>, from_current: bool) {
        if self.present.borrow().is_some() {
            return;
        }
        let (pres, start) = {
            let a = self.app.borrow();
            (a.pres().clone(), if from_current { a.cur } else { 0 })
        };
        let me = Rc::downgrade(self);
        let on_exit: Box<dyn Fn()> = Box::new(move || {
            // Called from inside the presentation window; drop it afterwards.
            let me = me.clone();
            Timer::single_shot(Duration::from_millis(1), move || {
                if let Some(s) = me.upgrade() {
                    let cur = s.present.borrow().as_ref().map(|p| p.borrow().current_slide());
                    s.present.borrow_mut().take();
                    if let Some(c) = cur {
                        s.app.borrow_mut().select_slide(c);
                    }
                    s.refresh_all();
                    if let Some(ui) = s.ui() {
                        ui.window().set_fullscreen(false);
                    }
                }
            });
        });
        match present::Present::start(pres, start, self.renderer.clone(), on_exit) {
            Ok(p) => *self.present.borrow_mut() = Some(p),
            Err(e) => self.app.borrow_mut().status = format!("Impossible de démarrer le diaporama : {e}"),
        }
    }

    fn apply_geometry(&self) {
        let Some(ui) = self.ui() else { return };
        let (x, y, w, h, r) = (
            parse_f(&ui.get_px()),
            parse_f(&ui.get_py()),
            parse_f(&ui.get_pw()),
            parse_f(&ui.get_ph()),
            parse_f(&ui.get_prot()),
        );
        self.app.borrow_mut().set_xfrm_of_selection(x, y, w, h, r);
        self.refresh_all();
    }

    fn apply_transition(&self, all: bool) {
        let Some(ui) = self.ui() else { return };
        let kind = self.transition_kinds.get(ui.get_transition_index().max(0) as usize).copied().unwrap_or("none");
        let secs = parse_f(&ui.get_transition_duration()).unwrap_or(0.7);
        self.app.borrow_mut().set_transition(kind, secs, ui.get_advance_on_click(), all);
        self.refresh_all();
    }

    /// Keyboard shortcuts. Returns true when handled.
    fn key(&self, text: &str, ctrl: bool, shift: bool, _alt: bool) -> bool {
        use slint::platform::Key;
        let is = |k: Key| text == String::from(char::from(k));
        let step = if shift { 10.0 * EMU_PT } else { EMU_PT };
        let mut handled = true;
        if ctrl {
            match text.to_lowercase().as_str() {
                "z" if shift => self.app.borrow_mut().redo(),
                "z" => self.app.borrow_mut().undo(),
                "y" => self.app.borrow_mut().redo(),
                "c" => self.app.borrow_mut().copy(),
                "x" => {
                    let mut a = self.app.borrow_mut();
                    a.copy();
                    a.delete_selection();
                }
                "v" => self.app.borrow_mut().paste(),
                "d" => self.app.borrow_mut().duplicate_selection(),
                "a" => self.app.borrow_mut().select_all(),
                "g" if shift => self.app.borrow_mut().arrange("ungroup"),
                "g" => self.app.borrow_mut().arrange("group"),
                "m" => self.app.borrow_mut().add_slide(),
                "s" if shift => {
                    self.save_as();
                    return true;
                }
                "s" => {
                    self.save();
                    return true;
                }
                "o" => {
                    self.dialog(
                        Pending::Open,
                        "Ouvrir…",
                        format!("{}/", std::env::var("HOME").unwrap_or_else(|_| ".".into())),
                    );
                    return true;
                }
                "n" => {
                    self.new_doc();
                    return true;
                }
                "b" => self.app.borrow_mut().toggle_bold(),
                "i" => self.app.borrow_mut().toggle_italic(),
                "u" => self.app.borrow_mut().toggle_underline(),
                "+" | "=" => self.app.borrow_mut().zoom_by(1.25, None),
                "-" => self.app.borrow_mut().zoom_by(0.8, None),
                "0" => self.app.borrow_mut().zoom_fit(),
                _ => handled = false,
            }
        } else if is(Key::Delete) || is(Key::Backspace) {
            self.app.borrow_mut().delete_selection();
        } else if is(Key::Escape) {
            let mut a = self.app.borrow_mut();
            a.tool = "select".into();
            a.sel.clear();
        } else if is(Key::LeftArrow) {
            self.app.borrow_mut().nudge(-step, 0.0);
        } else if is(Key::RightArrow) {
            self.app.borrow_mut().nudge(step, 0.0);
        } else if is(Key::UpArrow) {
            self.app.borrow_mut().nudge(0.0, -step);
        } else if is(Key::DownArrow) {
            self.app.borrow_mut().nudge(0.0, step);
        } else if is(Key::PageUp) {
            let c = self.app.borrow().cur;
            self.app.borrow_mut().select_slide(c.saturating_sub(1));
        } else if is(Key::PageDown) {
            let c = self.app.borrow().cur;
            self.app.borrow_mut().select_slide(c + 1);
        } else if is(Key::F5) {
            return false; // handled by the caller (needs Rc<Self>)
        } else {
            handled = false;
        }
        if handled {
            self.refresh_all();
        }
        handled
    }

    fn autosave(&self) {
        let (rev, dirty, path) = {
            let a = self.app.borrow();
            (a.doc.history.revision(), a.dirty(), a.path.clone())
        };
        if !dirty || rev == self.last_autosave_rev.get() {
            return;
        }
        let dir = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
            .map(|d| d.join("kinaute").join("recovery"));
        let Some(dir) = dir else { return };
        let name = path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "sans-titre".into());
        if std::fs::create_dir_all(&dir).is_ok() {
            if let Ok(bytes) = self.app.borrow().doc.save_to_bytes() {
                if std::fs::write(dir.join(format!("{name}.recovery.pptx")), bytes).is_ok() {
                    self.last_autosave_rev.set(rev);
                }
            }
        }
    }
}

thread_local! {
    static PENDING: Cell<Option<Pending>> = const { Cell::new(None) };
}

fn run() -> Result<(), slint::PlatformError> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();
    let ui = MainWindow::new()?;
    let renderer = Rc::new(RefCell::new(Renderer::new()));

    let arg_path = std::env::args().nth(1).map(PathBuf::from);
    let (doc, path) = match &arg_path {
        Some(p) => match std::fs::read(p)
            .map_err(|e| e.to_string())
            .and_then(|b| kinaute_pptx::open(&b).map_err(|e| e.to_string()))
        {
            Ok(d) => (d, Some(p.clone())),
            Err(e) => {
                eprintln!("kinaute: cannot open {}: {e}", p.display());
                (kinaute_pptx::blank().map_err(|e| slint::PlatformError::Other(e.to_string()))?, None)
            }
        },
        None => (kinaute_pptx::blank().map_err(|e| slint::PlatformError::Other(e.to_string()))?, None),
    };
    let kinds: Vec<&'static str> = kinaute_render::transition::KINDS.iter().map(|(k, _)| *k).collect();
    let names: Vec<SharedString> =
        kinaute_render::transition::KINDS.iter().map(|(_, n)| SharedString::from(*n)).collect();
    ui.set_transition_names(ModelRc::new(VecModel::from(names)));

    let slides = Rc::new(VecModel::<SlideItem>::default());
    ui.set_slides(ModelRc::from(slides.clone()));
    let shell = Rc::new(Shell {
        app: RefCell::new(App::new(doc, path, renderer.clone())),
        ui: ui.as_weak(),
        slides,
        thumbs: RefCell::new(Vec::new()),
        renderer,
        present: RefCell::new(None),
        last_autosave_rev: Cell::new(0),
        transition_kinds: kinds,
    });

    macro_rules! on {
        ($setter:ident, |$s:ident $(, $arg:ident)*| $body:expr) => {{
            let $s = shell.clone();
            ui.$setter(move |$($arg),*| {
                let $s = &$s;
                $body
            });
        }};
    }

    // Canvas.
    on!(on_canvas_resized, |s, w, h| {
        let sf = s.scale();
        s.app.borrow_mut().set_canvas((w * sf) as u32, (h * sf) as u32);
        s.refresh_canvas();
    });
    on!(on_canvas_pressed, |s, x, y, shift, ctrl| {
        let sf = s.scale();
        s.app.borrow_mut().press(x * sf, y * sf, shift, ctrl);
        s.refresh_inspector();
        s.refresh_canvas();
    });
    on!(on_canvas_moved, |s, x, y| {
        let sf = s.scale();
        let dragging = s.app.borrow().is_dragging();
        if dragging {
            s.app.borrow_mut().drag_to(x * sf, y * sf, false);
            s.sync_slides();
            s.refresh_canvas();
        } else {
            s.app.borrow_mut().hover(x * sf, y * sf);
            if let Some(ui) = s.ui() {
                use app::Handle;
                let kind = match s.app.borrow().cursor_handle {
                    Some(Handle::Rotate) => "rotate",
                    Some(Handle::Resize(0, _)) => "ns",
                    Some(Handle::Resize(_, 0)) => "ew",
                    Some(Handle::Resize(1, 1)) | Some(Handle::Resize(-1, -1)) => "nwse",
                    Some(Handle::Resize(_, _)) => "nesw",
                    None => "default",
                };
                ui.set_cursor_kind(SharedString::from(kind));
            }
        }
    });
    on!(on_canvas_released, |s, x, y| {
        let sf = s.scale();
        s.app.borrow_mut().release(x * sf, y * sf);
        s.refresh_all();
    });
    on!(on_canvas_double_clicked, |s, _x, _y| {
        let has_text = s.app.borrow().sel_info().has_text;
        if has_text {
            if let Some(ui) = s.ui() {
                ui.set_tab(1);
            }
        }
    });
    on!(on_canvas_scrolled, |s, x, y, dy, ctrl| {
        let sf = s.scale();
        if ctrl {
            s.app.borrow_mut().zoom_by(if dy > 0.0 { 1.1 } else { 0.9 }, Some((x * sf, y * sf)));
            s.refresh_canvas();
        }
    });
    {
        let s = shell.clone();
        ui.on_key(move |t, ctrl, shift, alt| {
            use slint::platform::Key;
            if t.as_str() == String::from(char::from(Key::F5)) {
                s.play(shift);
                return true;
            }
            s.key(t.as_str(), ctrl, shift, alt)
        });
    }

    // Files.
    on!(on_new_doc, |s| s.new_doc());
    on!(on_open_doc, |s| s.dialog(
        Pending::Open,
        "Ouvrir…",
        format!("{}/", std::env::var("HOME").unwrap_or_else(|_| ".".into()))
    ));
    on!(on_save_doc, |s| s.save());
    on!(on_save_as, |s| s.save_as());
    on!(on_export_pdf, |s| {
        let start =
            s.app.borrow().path.as_ref().map(|p| p.with_extension("pdf").display().to_string()).unwrap_or_else(|| {
                format!("{}/presentation.pdf", std::env::var("HOME").unwrap_or_else(|_| ".".into()))
            });
        s.dialog(Pending::Pdf, "Exporter en PDF…", start);
    });
    on!(on_dialog_accepted, |s, t| {
        let what = PENDING.with(|p| p.take());
        if let Some(ui) = s.ui() {
            ui.set_dialog_open(false);
        }
        if let Some(w) = what {
            let p = PathBuf::from(t.as_str().trim());
            s.complete(w, p);
        }
    });
    on!(on_dialog_cancelled, |s| {
        PENDING.with(|p| p.set(None));
        if let Some(ui) = s.ui() {
            ui.set_dialog_open(false);
        }
    });

    // Edit.
    on!(on_undo, |s| {
        s.app.borrow_mut().undo();
        s.refresh_all();
    });
    on!(on_redo, |s| {
        s.app.borrow_mut().redo();
        s.refresh_all();
    });
    on!(on_select_tool, |s, t| {
        if t.as_str() == "image" {
            s.dialog(
                Pending::Image,
                "Insérer une image — chemin du fichier",
                format!("{}/", std::env::var("HOME").unwrap_or_else(|_| ".".into())),
            );
            return;
        }
        s.app.borrow_mut().tool = t.to_string();
        s.refresh_inspector();
    });
    on!(on_add_slide, |s| {
        s.app.borrow_mut().add_slide();
        s.refresh_all();
    });
    on!(on_duplicate_slide, |s| {
        s.app.borrow_mut().duplicate_slide();
        s.refresh_all();
    });
    on!(on_delete_slide, |s| {
        s.app.borrow_mut().delete_slide();
        s.refresh_all();
    });
    on!(on_move_slide, |s, d| {
        s.app.borrow_mut().move_slide(d);
        s.refresh_all();
    });
    on!(on_toggle_hidden, |s| {
        s.app.borrow_mut().toggle_hidden();
        s.refresh_all();
    });
    on!(on_slide_dragged, |s, from, rows| {
        let n = s.app.borrow().slide_count() as i32;
        let to = (from + rows).clamp(0, n - 1);
        s.app.borrow_mut().move_slide_to(from as usize, to as usize);
        s.refresh_all();
    });
    on!(on_pick_slide, |s, i| {
        s.app.borrow_mut().select_slide(i as usize);
        s.refresh_all();
    });
    {
        let s = shell.clone();
        ui.on_play(move |from_current| s.play(from_current));
    }
    on!(on_text_edited, |s, t| {
        let id = s.app.borrow().sel.first().copied();
        if let Some(id) = id {
            s.app.borrow_mut().set_text(id, t.as_str());
            s.sync_slides();
            s.refresh_canvas();
            s.refresh_title();
        }
    });
    on!(on_font_size_accepted, |s, t| {
        if let Some(v) = parse_f(t.as_str()) {
            s.app
                .borrow_mut()
                .patch_run(RunPatch { size: Some((v.clamp(1.0, 999.0) * 100.0) as i32), ..Default::default() });
            s.refresh_all();
        }
    });
    on!(on_toggle_bold, |s| {
        s.app.borrow_mut().toggle_bold();
        s.refresh_all();
    });
    on!(on_toggle_italic, |s| {
        s.app.borrow_mut().toggle_italic();
        s.refresh_all();
    });
    on!(on_toggle_underline, |s| {
        s.app.borrow_mut().toggle_underline();
        s.refresh_all();
    });
    on!(on_set_halign, |s, a| {
        s.app.borrow_mut().set_halign(a);
        s.refresh_all();
    });
    on!(on_set_valign, |s, a| {
        s.app.borrow_mut().set_valign(a);
        s.refresh_all();
    });
    on!(on_text_color, |s, c| {
        let [r, g, b] = rgb_of(c);
        s.app.borrow_mut().patch_run(RunPatch {
            color: Some(kinaute_model::Color::srgb(&kinaute_model::Rgba::from_u8(r, g, b).to_hex())),
            ..Default::default()
        });
        s.refresh_all();
    });
    on!(on_fill_picked, |s, c| {
        s.app.borrow_mut().set_fill(Some(rgb_of(c)));
        s.refresh_all();
    });
    on!(on_no_fill, |s| {
        s.app.borrow_mut().set_fill(None);
        s.refresh_all();
    });
    on!(on_line_picked, |s, c| {
        s.app.borrow_mut().set_line(Some(rgb_of(c)), None);
        s.refresh_all();
    });
    on!(on_no_line, |s| {
        s.app.borrow_mut().set_line(None, None);
        s.refresh_all();
    });
    on!(on_line_width_accepted, |s, t| {
        if let Some(w) = parse_f(t.as_str()) {
            s.app.borrow_mut().set_line(None, Some(w.clamp(0.0, 100.0)));
            s.refresh_all();
        }
    });
    on!(on_geometry_accepted, |s| s.apply_geometry());
    on!(on_arrange, |s, what| {
        s.app.borrow_mut().arrange(what.as_str());
        s.refresh_all();
    });
    on!(on_transition_changed, |s| s.apply_transition(false));
    on!(on_apply_transition_all, |s| s.apply_transition(true));
    on!(on_zoom_fit, |s| {
        s.app.borrow_mut().zoom_fit();
        s.refresh_canvas();
    });
    on!(on_zoom_step, |s, d| {
        s.app.borrow_mut().zoom_by(if d > 0 { 1.25 } else { 0.8 }, None);
        s.refresh_canvas();
    });

    // Background work: thumbnails (a few at a time, never blocking input)
    // and crash-recovery autosave.
    let thumb_timer = Timer::default();
    {
        let s = shell.clone();
        thumb_timer.start(TimerMode::Repeated, Duration::from_millis(8), move || {
            s.thumb_step();
        });
    }
    let autosave_timer = Timer::default();
    {
        let s = shell.clone();
        autosave_timer.start(TimerMode::Repeated, Duration::from_secs(30), move || s.autosave());
    }

    shell.refresh_all();
    ui.run()?;
    // Final autosave is not needed: the user saw the close; only dirty
    // documents keep a recovery copy.
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("kinaute: {e}");
        std::process::exit(1);
    }
}
