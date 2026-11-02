//! Lossless `.pptx` reader/writer.
//!
//! `open` parses a file into a [`Presentation`] while keeping the original
//! package; `save` writes the (possibly edited) model back, re-emitting
//! unchanged objects from their source XML and copying every unmodelled part
//! byte for byte. Nothing Kinaute does not understand is dropped.

mod package;
pub mod read;
pub mod write;

pub use package::{Package, PptxError};
pub use read::Opened;

/// A document on disk: its model, original package and import report.
pub struct Doc {
    pub history: kinaute_model::History,
    pub package: Package,
    pub report: Vec<String>,
}

/// Open a `.pptx` from bytes.
pub fn open(bytes: &[u8]) -> Result<Doc, PptxError> {
    let o = read::open(bytes)?;
    Ok(Doc { history: kinaute_model::History::new(o.pres), package: o.package, report: o.report })
}

impl Doc {
    pub fn pres(&self) -> &kinaute_model::Presentation {
        self.history.current()
    }

    /// Serialise the current state.
    pub fn save_to_bytes(&self) -> Result<Vec<u8>, PptxError> {
        write::save(self.history.current(), &self.package)
    }
}

/// A new blank 16:9 presentation using Kinaute's own original theme.
pub fn blank() -> Result<Doc, PptxError> {
    use std::sync::Arc;
    let mut pkg = Package::default();
    let files: [(&str, &str); 13] = [
        ("[Content_Types].xml", include_str!("../template/content_types.xml")),
        ("_rels/.rels", include_str!("../template/root_rels.xml")),
        ("ppt/presentation.xml", include_str!("../template/presentation.xml")),
        ("ppt/_rels/presentation.xml.rels", include_str!("../template/presentation_rels.xml")),
        ("ppt/slideMasters/slideMaster1.xml", include_str!("../template/master.xml")),
        ("ppt/slideMasters/_rels/slideMaster1.xml.rels", include_str!("../template/master_rels.xml")),
        ("ppt/slideLayouts/slideLayout1.xml", include_str!("../template/layout1.xml")),
        ("ppt/slideLayouts/slideLayout2.xml", include_str!("../template/layout2.xml")),
        ("ppt/slideLayouts/slideLayout3.xml", include_str!("../template/layout3.xml")),
        ("ppt/theme/theme1.xml", include_str!("../template/theme.xml")),
        ("ppt/slides/slide1.xml", include_str!("../template/slide1.xml")),
        ("ppt/slides/_rels/slide1.xml.rels", include_str!("../template/slide_rels.xml")),
        ("ppt/slideLayouts/_rels/slideLayout1.xml.rels", include_str!("../template/layout_rels.xml")),
    ];
    for (name, text) in files {
        pkg.set(name, Arc::new(text.as_bytes().to_vec()));
    }
    for n in 2..=3 {
        pkg.set(&format!("ppt/slideLayouts/_rels/slideLayout{n}.xml.rels"), Arc::new(include_bytes!("../template/layout_rels.xml").to_vec()));
    }
    let bytes = pkg.write()?;
    open(&bytes)
}
