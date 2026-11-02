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
