//! Dictation pipeline modules (PRD §11).
//!
//! The session state machine and its runtime owner live in [`session`].  This
//! module keeps the public pipeline surface stable for existing IPC and test
//! callers while making the normative ownership explicit.

pub mod session;
pub use session::*;

pub mod command_layer;
pub mod microphone;
pub mod session_pump;
pub mod session_task;

pub use session_task::SessionTaskEvent;
