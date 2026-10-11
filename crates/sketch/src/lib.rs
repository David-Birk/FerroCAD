//! 2D sketch model and geometric constraint solver.
//!
//! A [`Sketch`] stores every coordinate and radius as a scalar *parameter*.
//! Constraints are turned into residual equations `f(params) = 0` which the
//! [`solver`] drives to zero with a damped minimum-norm Gauss–Newton
//! iteration. Because the update is minimum-norm, under-constrained sketches
//! move as little as possible – the same "rubber band" feel Fusion has.
//!
//! After solving, [`Sketch::analyze`] reports the remaining degrees of
//! freedom and which points are fully determined (Fusion draws those black),
//! and [`profile::find_profiles`] turns the closed loops into extrudable
//! regions.

mod constraint;
mod data;
mod linalg;
pub mod profile;
mod region;
mod sketch;
pub mod solver;

pub use constraint::Constraint;
pub use data::{EntityData, SketchData};
pub use profile::{find_profiles, Loop, Profile, Segment};
pub use sketch::{ConstraintId, Entity, EntityId, ParamId, Point, PointId, Sketch};
pub use solver::{Analysis, SolveError, SolveReport};
