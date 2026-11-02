//! UI-agnostic application layer.
//!
//! Front ends (GUI, CLI, MCP server) call into this crate; it owns file loading,
//! background jobs, view-models and the command list. Nothing here knows about
//! egui, terminals or JSON, so all of it is unit-testable.

pub mod commands;
pub mod compare;
pub mod crash;
pub mod folder_view;
pub mod jobs;
pub mod text_view;

pub use commands::Command;
pub use compare::{
    CompareError, Comparison, FileCompare, FileVerdict, SideContent, compare_files, compare_one_sided, compare_paths, compare_texts,
};
pub use folder_view::{Filter, FolderView};
pub use jobs::{Job, JobFailed};
pub use text_view::TextView;
