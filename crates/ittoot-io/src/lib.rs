//! IO builder core: reads IO tables and circuit diagrams (scanned or text), cross-checks them,
//! names the signals and writes IO lists for PLC makers. No UI and no operating-system calls: the
//! words come from an OCR engine the caller provides (the app uses Windows' own).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

pub mod bitmap;
pub mod export;
pub mod grid;
pub mod model;
pub mod naming;
pub mod ocr;
pub mod pdf;
pub mod sheet;
pub mod table;
pub mod xref;

pub use model::{apply_edits, Address, Edit, IoKind, IoPoint, IoSet, Source, Status};
