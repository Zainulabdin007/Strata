//! Automatic compression optimization for Strata.
//!
//! Profiles data, benches candidate encoding pipelines on sample blocks, and
//! selects a plan per column according to a declared goal and constraints.

#![deny(missing_docs)]

pub mod bench;
pub mod candidates;
pub mod emit;
pub mod error;
pub mod format;
pub mod goal;
pub mod reason;
pub mod report;
pub mod select;

pub use bench::{bench_plan, BenchOptions, BenchResult};
pub use candidates::candidate_plans;
pub use emit::emit_profile_toml;
pub use error::{Error, Result};
pub use format::{format_human, format_json, format_plan_display};
pub use goal::{OptimizationConstraints, OptimizationGoal, OptimizeOptions};
pub use report::{optimize_dataset, ColumnDecision, OptimizeReport, PipelineTrial, SelectedPlan};
pub use select::select_plan;

pub mod experiment;
