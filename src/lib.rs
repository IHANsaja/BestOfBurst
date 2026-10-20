//! BestOfBurst prototype: turns a folder of photos into a `picks` folder of the best N shots.
//!
//! The same engine is driven by the command line (`main.rs`) and the egui window (`ui`).

pub mod analyze;
pub mod decode;
pub mod engine;
pub mod gpu;
pub mod output;
pub mod rank;
pub mod scan;
pub mod state;
pub mod stats;
pub mod sysmon;
pub mod ui;

/// Bump when metrics change: every file is re-analyzed when `state.json` has another version.
pub const ANALYSIS_VERSION: u32 = 1;
