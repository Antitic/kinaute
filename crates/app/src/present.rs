//! Full-screen presentation mode with transitions.

use crate::ui::PresentWindow;
use kinaute_model::{Presentation, Transition};
use kinaute_render::{transition, Renderer};
use slint::{ComponentHandle, Timer, TimerMode};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};
use tiny_skia::Pixmap;

struct Anim {
    from: Rc<Pixmap>,
    to: Rc<Pixmap>,
    kind: Transition,
    start: Instant,
    target_pos: usize,
}

pub struct Present {
    pub win: PresentWindow,
    pres: Presentation,
    order: Vec<usize>,
    pos: usize,
    size: (u32, u32),
    cache: HashMap<usize, Rc<Pixmap>>,
    renderer: Rc<RefCell<Renderer>>,
    anim: Option<Anim>,
    ticker: Timer,
    auto: Timer,
    ended: bool,
    on_exit: Option<Box<dyn Fn()>>,
    me: Weak<RefCell<Present>>,
}

pub fn to_image(pm: &Pixmap) -> slint::Image {
    let buf = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(pm.data(), pm.width(), pm.height());
    slint::Image::from_rgba8_premultiplied(buf)
}

impl Present {
    /// Start a presentation at slide `start` (index into the document).
    pub fn start(
        pres: Presentation,
        start: usize,
        renderer: Rc<RefCell<Renderer>>,
        on_exit: Box<dyn Fn()>,
    ) -> Result<Rc<RefCell<Present>>, slint::PlatformError> {
        let win = PresentWindow::new()?;
        let order: Vec<usize> = (0..pres.slides.len()).filter(|i| !pres.slides[*i].hidden || *i == start).collect();
        let pos = order.iter().position(|i| *i == start).unwrap_or(0);
        let p = Rc::new(RefCell::new(Present {
            win,
            pres,
            order,
            pos,
            size: (1280, 720),
            cache: HashMap::new(),
            renderer,
            anim: None,
            ticker: Timer::default(),
            auto: Timer::default(),
            ended: false,
            on_exit: Some(on_exit),
            me: Weak::new(),
        }));
        p.borrow_mut().me = Rc::downgrade(&p);
        let w: Weak<RefCell<Present>> = Rc::downgrade(&p);
        {
            let me = p.borrow();
            let weak = |f: fn(&mut Present)| {
                let w = w.clone();
                move || {
                    if let Some(rc) = w.upgrade() {
                        // The callback may close the window; hold the borrow briefly.
                        let rc2 = rc.clone();
                        f(&mut rc2.borrow_mut());
                    }
                }
            };
            me.win.on_next(weak(Present::next));
            me.win.on_prev(weak(Present::prev));
            me.win.on_first(weak(Present::first));
            me.win.on_last(weak(Present::last));
            me.win.on_exit(weak(Present::exit));
            let w2 = w.clone();
            me.win.on_resized(move |_, _| {
                if let Some(rc) = w2.upgrade() {
                    if let Ok(mut me) = rc.try_borrow_mut() {
                        me.layout_changed();
                    }
                }
            });
        }
        {
            let mut me = p.borrow_mut();
            me.win.show()?;
            me.win.window().set_fullscreen(true);
            me.layout_changed();
        }
        Ok(p)
    }

    fn layout_changed(&mut self) {
        let ps = self.win.window().size();
        if ps.width == 0 || ps.height == 0 {
            return;
        }
        let aspect = self.pres.slide_w as f64 / self.pres.slide_h as f64;
        let w = (ps.width as f64).min(ps.height as f64 * aspect).round().max(64.0) as u32;
        let w = w.min(2560);
        let h = (w as f64 / aspect).round() as u32;
        if (w, h) != self.size {
            self.size = (w, h);
            self.cache.clear();
        }
        if self.anim.is_none() && !self.ended {
            self.show_current();
        }
    }

    fn slide_pixmap(&mut self, idx: usize) -> Option<Rc<Pixmap>> {
        if let Some(p) = self.cache.get(&idx) {
            return Some(p.clone());
        }
        let pm = {
            let mut r = self.renderer.borrow_mut();
            r.show_prompts = false;
            r.render_slide(&self.pres, idx, self.size.0)
        };
        let pm = Rc::new(pm?);
        // Keep only a small window of slides in memory (old machines).
        if self.cache.len() > 4 {
            let keep: Vec<usize> = self.order.iter().skip(self.pos.saturating_sub(1)).take(3).copied().collect();
            self.cache.retain(|k, _| keep.contains(k));
        }
        self.cache.insert(idx, pm.clone());
        Some(pm)
    }

    fn show_current(&mut self) {
        let Some(&idx) = self.order.get(self.pos) else { return };
        let Some(pm) = self.slide_pixmap(idx) else { return };
        self.win.set_frame(to_image(&pm));
        self.schedule_auto();
        // Pre-render the next slide while the audience reads this one.
        if let Some(&next) = self.order.get(self.pos + 1) {
            let _ = self.slide_pixmap(next);
        }
    }

    fn schedule_auto(&mut self) {
        self.auto.stop();
        let Some(&idx) = self.order.get(self.pos) else { return };
        if let Some(ms) = self.pres.slides[idx].transition.as_ref().and_then(|t| t.advance_after_ms) {
            let me = self.me.clone();
            self.auto.start(TimerMode::SingleShot, Duration::from_millis(ms as u64), move || {
                if let Some(rc) = me.upgrade() {
                    if let Ok(mut s) = rc.try_borrow_mut() {
                        s.next();
                    }
                }
            });
        }
    }

    fn finish_anim(&mut self) {
        if let Some(a) = self.anim.take() {
            self.ticker.stop();
            self.pos = a.target_pos;
            self.win.set_frame(to_image(&a.to));
            self.schedule_auto();
            if let Some(&next) = self.order.get(self.pos + 1) {
                let _ = self.slide_pixmap(next);
            }
        }
    }

    fn go(&mut self, target: usize, animate: bool) {
        self.finish_anim();
        self.ended = false;
        let Some(&from_idx) = self.order.get(self.pos) else { return };
        let Some(&to_idx) = self.order.get(target) else { return };
        let t = self.pres.slides[to_idx].transition.clone().filter(|t| t.kind != "none" && t.kind != "cut");
        match (animate, t) {
            (true, Some(kind)) if target != self.pos => {
                let (Some(from), Some(to)) = (self.slide_pixmap(from_idx), self.slide_pixmap(to_idx)) else { return };
                self.anim = Some(Anim { from, to, kind, start: Instant::now(), target_pos: target });
                self.auto.stop();
                let me = self.me.clone();
                self.ticker.start(TimerMode::Repeated, Duration::from_millis(16), move || {
                    if let Some(rc) = me.upgrade() {
                        if let Ok(mut s) = rc.try_borrow_mut() {
                            s.tick();
                        }
                    }
                });
                self.tick();
            }
            _ => {
                self.pos = target;
                self.show_current();
            }
        }
    }

    /// Advance the running transition by one frame.
    fn tick(&mut self) {
        let Some(a) = &self.anim else { return };
        let dur = a.kind.duration_ms.max(1) as f32 / 1000.0;
        let t = a.start.elapsed().as_secs_f32() / dur;
        if t >= 1.0 {
            self.finish_anim();
        } else {
            let frame = transition::compose(&a.from, &a.to, &a.kind.kind, a.kind.dir.as_deref(), t);
            self.win.set_frame(to_image(&frame));
        }
    }

    pub fn next(&mut self) {
        if self.anim.is_some() {
            self.finish_anim();
            return;
        }
        if self.pos + 1 < self.order.len() {
            self.go(self.pos + 1, true);
        } else if !self.ended {
            self.ended = true;
            self.auto.stop();
            if let Some(p) = Pixmap::new(self.size.0, self.size.1) {
                let mut p = p;
                p.fill(tiny_skia::Color::BLACK);
                self.win.set_frame(to_image(&p));
            }
        } else {
            self.exit();
        }
    }

    pub fn prev(&mut self) {
        if self.anim.is_some() {
            self.finish_anim();
        }
        if self.ended {
            self.ended = false;
            self.show_current();
        } else if self.pos > 0 {
            self.go(self.pos - 1, false);
        }
    }

    pub fn first(&mut self) {
        self.go(0, false);
    }

    pub fn last(&mut self) {
        let n = self.order.len().saturating_sub(1);
        self.go(n, false);
    }

    pub fn exit(&mut self) {
        self.ticker.stop();
        self.auto.stop();
        let _ = self.win.hide();
        if let Some(f) = self.on_exit.take() {
            f();
        }
    }

    pub fn current_slide(&self) -> usize {
        self.order.get(self.pos).copied().unwrap_or(0)
    }
}
