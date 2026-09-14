//! includecost ranks C/C++ headers by the preprocessed code that would
//! disappear if each one were emptied, computed exactly from dominator trees
//! of per-translation-unit include graphs.
//!
//! Pipeline: [`compdb`] reads `compile_commands.json`, [`project::Project`]
//! scans and resolves every reachable file, [`project::Project::tu_graph`]
//! builds one include graph per TU, [`dominators`] computes its dominator tree,
//! and [`analyze`] aggregates exclusive and inclusive bytes across TUs.

pub mod analyze;
pub mod compdb;
pub mod dominators;
pub mod fs;
pub mod generate;
pub mod json;
pub mod project;
pub mod report;
pub mod resolve;
pub mod scan;

pub use analyze::{analyze, Analysis, HeaderCost, Options};
pub use compdb::{parse_compile_commands, CompileCommand, SearchPaths};
pub use fs::{DiskFs, MemFs, SourceProvider};
pub use project::Project;
