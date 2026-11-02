//! Kinaute document model.
//!
//! Pure data and logic, no I/O, no UI, no rendering. The model mirrors
//! DrawingML/PresentationML closely because `.pptx` is Kinaute's only file
//! format: every object keeps its original XML (`source`) so that the writer
//! can patch what changed and carry the rest through untouched.

pub mod color;
pub mod edit;
pub mod new;
pub mod resolve;
pub mod types;

pub use color::{Color, ColorBase, Rgba};
pub use edit::{Command, EditError, History};
pub use types::*;
