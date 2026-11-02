//! `kinaute-tool info|roundtrip|check FILE...`

use kinaute_model::edit::RunPatch;
use kinaute_model::*;
use std::process::ExitCode;

fn count(shapes: &[Shape]) -> (usize, usize, usize) {
    let (mut sh, mut pic, mut opaque) = (0, 0, 0);
    for s in shapes {
        match &s.kind {
            ShapeKind::Group(g) => {
                let (a, b, c) = count(&g.shapes);
                sh += a + 1;
                pic += b;
                opaque += c;
            }
            ShapeKind::Picture(_) => pic += 1,
            ShapeKind::Opaque { .. } => opaque += 1,
            _ => sh += 1,
        }
    }
    (sh, pic, opaque)
}

fn info(path: &str) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let doc = kinaute_pptx::open(&bytes).map_err(|e| e.to_string())?;
    let p = doc.pres();
    println!("{path}: {} slides, {}x{} EMU, {} layouts, {} masters, {} media", p.slides.len(), p.slide_w, p.slide_h, p.layouts.len(), p.masters.len(), p.media.len());
    for (i, s) in p.slides.iter().enumerate() {
        let (sh, pic, op) = count(&s.shapes);
        let title = s.shapes.iter().find(|x| x.ph.as_ref().map(|p| p.is_title()).unwrap_or(false)).and_then(|x| x.text()).map(|t| t.plain_text()).unwrap_or_default();
        println!("  {:>3}. {sh} shapes, {pic} pics, {op} opaque, transition={:?}  {}", i + 1, s.transition.as_ref().map(|t| t.kind.as_str()), title.replace('\n', " / "));
    }
    for r in &doc.report {
        println!("  ! {r}");
    }
    Ok(())
}

/// Open → save → reopen; the model must be identical.
fn roundtrip(path: &str, out: Option<&str>) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let doc = kinaute_pptx::open(&bytes).map_err(|e| format!("open: {e}"))?;
    let saved = doc.save_to_bytes().map_err(|e| format!("save: {e}"))?;
    let again = kinaute_pptx::open(&saved).map_err(|e| format!("reopen: {e}"))?;
    let strip = |p: &Presentation| {
        let mut p = p.clone();
        p.slides = p
            .slides
            .iter()
            .map(|s| {
                let mut s = (**s).clone();
                s.source = None; // whitespace between shapes is not content
                std::sync::Arc::new(s)
            })
            .collect();
        p
    };
    if strip(doc.pres()) != strip(again.pres()) {
        let (a, b) = (&strip(doc.pres()), &strip(again.pres()));
        let mut why = String::from("model differs");
        if a.slides.len() != b.slides.len() {
            why = format!("slide count {} -> {}", a.slides.len(), b.slides.len());
        }
        for (i, (x, y)) in a.slides.iter().zip(b.slides.iter()).enumerate() {
            if x != y {
                let (xs, ys) = (format!("{x:?}"), format!("{y:?}"));
                let at = xs.bytes().zip(ys.bytes()).position(|(p, q)| p != q).unwrap_or(0);
                let lo = at.saturating_sub(150);
                why = format!("slide {} differs at {at}:\n   A: {}\n   B: {}", i + 1, &xs[lo..(at + 150).min(xs.len())], &ys[lo..(at + 150).min(ys.len())]);
                break;
            }
        }
        if why == "model differs" && (a.layouts != b.layouts || a.masters != b.masters || a.themes != b.themes) {
            why = "layouts/masters/themes differ".into();
        }
        return Err(why);
    }
    // Every original part must still be present.
    let a = doc.package.parts.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>();
    let missing: Vec<_> = a.iter().filter(|n| !again.package.contains(n)).collect();
    if !missing.is_empty() {
        return Err(format!("parts lost: {missing:?}"));
    }
    // Parts we did not touch must be byte-identical.
    let changed: Vec<_> = doc
        .package
        .parts
        .iter()
        .filter(|(n, d)| again.package.get(n).map(|x| x.as_slice() != d.as_slice()).unwrap_or(true))
        .map(|(n, _)| n.as_str())
        .filter(|n| !(n.starts_with("ppt/slides/") || n.contains("presentation.xml") || n.contains("Content_Types")))
        .collect();
    if !changed.is_empty() {
        return Err(format!("untouched parts modified: {changed:?}"));
    }
    if let Some(o) = out {
        std::fs::write(o, saved).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Apply a fixed series of edits (for validating the writer with other readers).
fn demo_edit(path: &str, out: &str) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let mut doc = kinaute_pptx::open(&bytes).map_err(|e| e.to_string())?;
    let e = |r: Result<(), EditError>| r.map_err(|e| e.to_string());
    let h = &mut doc.history;
    let w = h.current().slide_w;
    let hh = h.current().slide_h;
    e(h.apply(Command::DuplicateSlide { index: 0 }))?;
    e(h.apply(Command::InsertShapes { slide: 0, shapes: vec![
        new::text_box(0, Xfrm::new(w / 10, hh / 10, w * 8 / 10, hh / 8), "Édité par Kinaute — été, œuvre & <test>", 3200),
        new::shape(0, Xfrm::new(w / 10, hh / 2, w / 4, hh / 4), "roundRect"),
        new::line(0, Xfrm::new(w / 2, hh / 2, w / 4, hh / 4)),
    ] }))?;
    let first = h.current().slides[0].shapes.iter().find(|s| s.text().is_some()).map(|s| s.id);
    if let Some(id) = first {
        e(h.apply(Command::SetText { slide: 0, id, text: "Titre modifié\nSeconde ligne".into() }))?;
        e(h.apply(Command::SetRunProps { slide: 0, ids: vec![id], patch: RunPatch { bold: Some(true), color: Some(Color::srgb("C0392B")), ..Default::default() } }))?;
    }
    e(h.apply(Command::SetTransition { slides: vec![0], transition: Some(Transition { kind: "push".into(), dir: Some("l".into()), ..Default::default() }) }))?;
    if h.current().slides.len() > 2 {
        e(h.apply(Command::MoveSlide { from: 1, to: 2 }))?;
        e(h.apply(Command::DeleteSlide { index: 3.min(h.current().slides.len() - 1) }))?;
    }
    let saved = doc.save_to_bytes().map_err(|e| e.to_string())?;
    std::fs::write(out, saved).map_err(|e| e.to_string())
}

/// Render every slide to PNG files `OUT/NN.png`.
fn render(path: &str, out: &str, width: u32) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let doc = kinaute_pptx::open(&bytes).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(out).map_err(|e| e.to_string())?;
    let mut r = kinaute_render::Renderer::new();
    let pres = doc.pres();
    r.set_media(pres.media.clone());
    let t0 = std::time::Instant::now();
    for i in 0..pres.slides.len() {
        if std::env::var_os("KINAUTE_TRACE").is_some() {
            eprintln!("slide {}", i + 1);
        }
        let pm = r.render_slide(pres, i, width).ok_or("render failed")?;
        let png = kinaute_render::to_png(&pm).ok_or("png failed")?;
        std::fs::write(format!("{out}/{:02}.png", i + 1), png).map_err(|e| e.to_string())?;
    }
    eprintln!("{path}: {} slides rendered in {:?}", pres.slides.len(), t0.elapsed());
    Ok(())
}

/// Export all (non-hidden) slides to a raster PDF.
fn export_pdf(path: &str, out: &str) -> Result<(), String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let doc = kinaute_pptx::open(&bytes).map_err(|e| e.to_string())?;
    let pres = doc.pres();
    let mut r = kinaute_render::Renderer::new();
    r.set_media(pres.media.clone());
    let mut pages = Vec::new();
    for i in 0..pres.slides.len() {
        if pres.slides[i].hidden {
            continue;
        }
        let pm = r.render_slide(pres, i, 1600).ok_or("render failed")?;
        let jpeg = kinaute_render::pdf::pixmap_to_jpeg(&pm, 88).ok_or("jpeg failed")?;
        pages.push(kinaute_render::pdf::Page { jpeg, width_px: pm.width(), height_px: pm.height() });
    }
    let pdf = kinaute_render::pdf::write_pdf(&pages, pres.slide_w as f32 / 12700.0, pres.slide_h as f32 / 12700.0, path);
    std::fs::write(out, pdf).map_err(|e| e.to_string())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let Some(cmd) = args.get(1) else {
        eprintln!("usage: kinaute-tool info|roundtrip|check FILE... [OUT]");
        return ExitCode::from(2);
    };
    let mut failed = 0;
    match cmd.as_str() {
        "info" => {
            for f in &args[2..] {
                if let Err(e) = info(f) {
                    eprintln!("{f}: ERROR {e}");
                    failed += 1;
                }
            }
        }
        "roundtrip" if args.len() == 4 && !args[2].is_empty() && args[3].ends_with(".pptx") && args[2] != "-" => {
            if let Err(e) = roundtrip(&args[2], Some(&args[3])) {
                eprintln!("{}: FAIL {e}", args[2]);
                failed += 1;
            }
        }
        "demo-edit" if args.len() == 4 => {
            if let Err(e) = demo_edit(&args[2], &args[3]) {
                eprintln!("{}: FAIL {e}", args[2]);
                failed += 1;
            }
        }
        "render" if args.len() >= 4 => {
            let w = args.get(4).and_then(|v| v.parse().ok()).unwrap_or(960);
            if let Err(e) = render(&args[2], &args[3], w) {
                eprintln!("{}: FAIL {e}", args[2]);
                failed += 1;
            }
        }
        "pdf" if args.len() == 4 => {
            if let Err(e) = export_pdf(&args[2], &args[3]) {
                eprintln!("{}: FAIL {e}", args[2]);
                failed += 1;
            }
        }
        "new" if args.len() == 3 => match kinaute_pptx::blank().and_then(|d| d.save_to_bytes()) {
            Ok(b) => {
                if let Err(e) = std::fs::write(&args[2], b) {
                    eprintln!("{e}");
                    failed += 1;
                }
            }
            Err(e) => {
                eprintln!("FAIL {e}");
                failed += 1;
            }
        },
        "check" => {
            let (mut ok, mut skipped) = (0, 0);
            for f in &args[2..] {
                match roundtrip(f, None) {
                    Ok(()) => ok += 1,
                    Err(e) if e.starts_with("open:") => {
                        skipped += 1;
                        eprintln!("{f}: SKIP {e}");
                    }
                    Err(e) => {
                        failed += 1;
                        eprintln!("{f}: FAIL {e}");
                    }
                }
            }
            println!("round trip: {ok} ok, {failed} failed, {skipped} not openable");
        }
        _ => {
            eprintln!("unknown command");
            return ExitCode::from(2);
        }
    }
    if failed > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
