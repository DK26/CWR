//! Lexers + light structural analysis for the Poseidon resource languages.
//!
//! Grammar facts are taken from the engine source:
//!   * SQF/SQS  — `engine/Evaluator/express.cpp`, `Scripts.cpp`
//!   * ParamFile — `engine/Poseidon/IO/ParamFile/*`, `IO/PreprocC/Preproc.cpp`
//!
//! Everything here works on byte offsets; mapping to LSP line/character
//! positions is the server's job (see `poseidon-lsp`).

pub mod check;
pub mod common;
pub mod config;
pub mod oracle;
pub mod sqf;

pub use common::{Diagnostic, Severity, Span, Tok, Token};
