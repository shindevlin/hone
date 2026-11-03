//! hone-decide: a small Rust-native decision engine for HONE.
//!
//! Given a context, a question and candidate answers it returns calibrated
//! probabilities per candidate. The design is a reimplementation of Open-Jev
//! (<https://github.com/Zefan-Cai/Open-Jev>, MIT): each candidate is scored by an
//! independent Yes/No prompt, a scalar head reads the final-normed hidden state of
//! the prompt's last token, and one calibration temperature turns the candidate
//! scores into a softmax. See docs/OPEN_JEV_NOTES.md.

pub mod engine;
pub mod models;
pub mod prompt;
pub mod pyjson;
pub mod server;
pub mod tasks;
pub mod weights;
