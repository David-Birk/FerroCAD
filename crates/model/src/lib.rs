//! The parametric document: user parameters, the feature timeline and the
//! [`Kernel`] trait that turns features into solids.
//!
//! Everything here is independent of the geometry kernel. The real kernel
//! (OpenCascade) lives in `ferrocad-kernel-occt`; tests use a mock.

pub mod document;
pub mod expr;
pub mod feature;
pub mod geom;
pub mod history;
pub mod io;
pub mod json;
pub mod kernel;
pub mod params;
pub mod project;
pub mod thread;

pub use document::{Body, Document, FeatureStatus, Model, ViewState};
pub use feature::*;
pub use geom::{Plane, Vec3};
pub use history::History;
pub use kernel::{
    BooleanOp, CylinderInfo, EdgeRef, FaceInfo, FaceRef, Kernel, KernelError, Loop3, MassProperties,
    PlacedProfile, Seg3, Transform, TriMesh,
};
pub use params::{ParamTable, Parameter};
