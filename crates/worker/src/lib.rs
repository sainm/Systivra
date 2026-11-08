#![forbid(unsafe_code)]

pub mod docx;
pub mod job;
mod office;
pub mod pdf;
pub mod pptx;
pub mod project;
pub mod xlsx;

pub use engine_code::{jsp, text};
pub use job::run;
