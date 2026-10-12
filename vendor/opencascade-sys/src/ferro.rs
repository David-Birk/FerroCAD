//! ferrocad additions: whole modelling operations that report OCCT failures
//! as `Err` instead of aborting (see `include/ferro.hxx`).
pub use inner::*;

#[cxx::bridge]
mod inner {
    unsafe extern "C++" {
        include!("opencascade-sys/include/ferro.hxx");

        type TopoDS_Shape = crate::topo_ds::TopoDS_Shape;

        pub fn ferro_fillet(
            shape: &TopoDS_Shape,
            edges: &[u32],
            radius: f64,
            chamfer: bool,
        ) -> Result<UniquePtr<TopoDS_Shape>>;
        pub fn ferro_thin_band(face: &TopoDS_Shape, p: &[f64]) -> Result<UniquePtr<TopoDS_Shape>>;
        pub fn ferro_cylinder(shape: &TopoDS_Shape, index: u32) -> Vec<f64>;
        pub fn ferro_bounds(shape: &TopoDS_Shape) -> Vec<f64>;
        pub fn ferro_thread_groove(p: &[f64]) -> Result<UniquePtr<TopoDS_Shape>>;
        pub fn ferro_chamfer2(shape: &TopoDS_Shape, edges: &[u32], d1: f64, d2: f64) -> Result<UniquePtr<TopoDS_Shape>>;
        pub fn ferro_draft(shape: &TopoDS_Shape, faces: &[u32], p: &[f64]) -> Result<UniquePtr<TopoDS_Shape>>;
        pub fn ferro_boolean(a: &TopoDS_Shape, b: &TopoDS_Shape, op: u32) -> Result<UniquePtr<TopoDS_Shape>>;
        pub fn ferro_shell(shape: &TopoDS_Shape, faces: &[u32], thickness: f64) -> Result<UniquePtr<TopoDS_Shape>>;
        pub fn ferro_transform(shape: &TopoDS_Shape, kind: u32, p: &[f64]) -> Result<UniquePtr<TopoDS_Shape>>;
        pub fn ferro_profile_face(data: &[f64]) -> Result<UniquePtr<TopoDS_Shape>>;
        pub fn ferro_prism(face: &TopoDS_Shape, dx: f64, dy: f64, dz: f64) -> Result<UniquePtr<TopoDS_Shape>>;
        pub fn ferro_draft_prism(
            face: &TopoDS_Shape,
            h1: f64,
            h2: f64,
            angle: f64,
            reverse: bool,
        ) -> Result<UniquePtr<TopoDS_Shape>>;
        pub fn ferro_revol(face: &TopoDS_Shape, p: &[f64]) -> Result<UniquePtr<TopoDS_Shape>>;
        pub fn ferro_loft(data: &[f64]) -> Result<UniquePtr<TopoDS_Shape>>;
        pub fn ferro_sweep(face: &TopoDS_Shape, path: &[f64]) -> Result<UniquePtr<TopoDS_Shape>>;
        pub fn ferro_sub_shape(shape: &TopoDS_Shape, kind: u32, index: u32) -> Result<UniquePtr<TopoDS_Shape>>;
        pub fn ferro_read_step(path: &str) -> Result<UniquePtr<TopoDS_Shape>>;
        pub fn ferro_find_face(shape: &TopoDS_Shape, q: &[f64]) -> Vec<f64>;
        pub fn ferro_properties(shape: &TopoDS_Shape) -> Vec<f64>;
        pub fn ferro_count(shape: &TopoDS_Shape, kind: u32) -> u32;
    }
}
