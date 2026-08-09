//! Library behind the `kladde` binary.
//!
//! The binary stays thin: behavior lives here so it can be tested without
//! spawning a process. The library reads no environment and touches no global
//! state; the binary gathers inputs and passes them in.

pub mod config;
pub mod day;
pub mod editor;
pub mod frontmatter;
pub mod notebook;
pub mod structure;
pub mod write;
