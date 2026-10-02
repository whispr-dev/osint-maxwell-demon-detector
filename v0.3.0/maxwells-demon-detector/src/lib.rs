//! Maxwell's Demon Detector: watch structure flow through a data stream as a
//! scrolling ASCII landscape.
//!
//! Pipeline: [`source`] (demo, files, stdin, pcap, live) -> [`event`] symbolisation
//! -> [`analysis`] per-lane statistics and detectors -> [`app`] history ->
//! [`render`] frame -> [`screen`] flicker-free terminal output.

pub mod analysis;
pub mod app;
pub mod cli;
pub mod decode;
pub mod event;
pub mod pcapfile;
pub mod render;
pub mod rng;
pub mod run;
pub mod screen;
pub mod source;

/// Crate-wide error type: any error, sendable across threads.
pub type Error = Box<dyn std::error::Error + Send + Sync + 'static>;
/// Crate-wide result type.
pub type Result<T> = std::result::Result<T, Error>;

pub use cli::{Cli, Mode};
pub use run::{run, run_headless};
