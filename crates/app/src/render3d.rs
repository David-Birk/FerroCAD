//! GPU rendering of bodies with a real depth buffer, as an egui_wgpu paint
//! callback inside egui's render pass (eframe is started with a depth
//! buffer). Overlays (sketches, highlights, dimensions) are still drawn with
//! egui's painter on top.

use eframe::egui_wgpu::{self, wgpu};
use ferrocad_model::geom::{dot, Vec3};
use ferrocad_view::{Camera, Viewport};
use std::sync::Arc;
use wgpu::util::DeviceExt;

pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth24Plus;
pub const MSAA: u32 = 4;

const SHADER: &str = r#"
struct U {
    view_proj: mat4x4<f32>,
    eye: vec4<f32>,
    color: vec4<f32>,
    section_o: vec4<f32>,
    section_n: vec4<f32>,
};
@group(0) @binding(0) var<uniform> u: U;

struct VIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
};
struct VOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
};

@vertex
fn vs_main(v: VIn) -> VOut {
    var o: VOut;
    o.clip = u.view_proj * vec4<f32>(v.pos, 1.0);
    o.world = v.pos;
    o.normal = v.normal;
    o.color = u.color.rgb;
    return o;
}

struct BIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
};

/// Body triangles carry their material colour.
@vertex
fn vs_body(v: BIn) -> VOut {
    var o: VOut;
    o.clip = u.view_proj * vec4<f32>(v.pos, 1.0);
    o.world = v.pos;
    o.normal = v.normal;
    o.color = v.color;
    return o;
}

fn cut(world: vec3<f32>) -> bool {
    return u.section_n.w > 0.5 && dot(world - u.section_o.xyz, u.section_n.xyz) > 0.0;
}

@fragment
fn fs_main(i: VOut) -> @location(0) vec4<f32> {
    if (cut(i.world)) { discard; }
    let view = normalize(u.eye.xyz - i.world);
    var n = normalize(i.normal);
    var inside = 0.0;
    if (dot(n, view) < 0.0) { n = -n; inside = 1.0; }
    let key = normalize(view * 0.8 + vec3<f32>(0.25, 0.15, 0.55));
    let diffuse = max(dot(n, key), 0.0);
    let rim = pow(1.0 - max(dot(n, view), 0.0), 3.0) * 0.12;
    var c = i.color * (0.38 + 0.62 * diffuse) + vec3<f32>(rim);
    // Faces seen from inside (section view) are tinted.
    c = mix(c, vec3<f32>(0.85, 0.45, 0.2), inside * 0.6);
    return vec4<f32>(c, 1.0);
}

struct OIn {
    @location(0) pos: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) cuttable: f32,
};
struct OOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) cuttable: f32,
};

@vertex
fn vs_overlay(v: OIn) -> OOut {
    var o: OOut;
    o.clip = u.view_proj * vec4<f32>(v.pos, 1.0);
    o.world = v.pos;
    o.color = v.color;
    o.cuttable = v.cuttable;
    return o;
}

/// Face highlights are cut with the bodies; planes are not.
@fragment
fn fs_overlay(i: OOut) -> @location(0) vec4<f32> {
    if (i.cuttable > 0.5 && cut(i.world)) { discard; }
    return i.color;
}

@fragment
fn fs_line(i: VOut) -> @location(0) vec4<f32> {
    if (cut(i.world)) { discard; }
    return vec4<f32>(0.08, 0.09, 0.11, 1.0);
}
"#;

/// Geometry for one frame; rebuilt only when `id` changes.
pub struct SceneData {
    pub id: u64,
    /// Interleaved position + normal + colour, three vertices per triangle.
    pub tris: Vec<f32>,
    /// Interleaved position + (unused) normal, two vertices per segment.
    pub lines: Vec<f32>,
}

impl SceneData {
    pub fn build<'a>(
        id: u64,
        bodies: impl Iterator<Item = &'a crate::app::BodyDisplay>,
        edges: bool,
        color_of: &dyn Fn(&str) -> [u8; 3],
    ) -> Self {
        let mut tris = Vec::new();
        let mut lines = Vec::new();
        for b in bodies.filter(|b| b.visible) {
            let m = &b.mesh;
            let col = color_of(&b.name).map(|c| c as f32 / 255.0);
            let has_normals = m.normals.len() == m.positions.len();
            for t in m.indices.as_chunks::<3>().0 {
                let [p0, p1, p2] = [0, 1, 2].map(|k| m.positions[t[k] as usize]);
                let flat = ferrocad_model::geom::normalize(ferrocad_model::geom::cross(
                    ferrocad_model::geom::sub(p1, p0),
                    ferrocad_model::geom::sub(p2, p0),
                ));
                for &i in t {
                    let p = m.positions[i as usize];
                    let n = if has_normals && dot(m.normals[i as usize], flat) > 0.0 { m.normals[i as usize] } else { flat };
                    tris.extend([p[0], p[1], p[2], n[0], n[1], n[2]].map(|v| v as f32));
                    tris.extend(col);
                }
            }
            if edges {
                for e in &b.edges {
                    for w in e.windows(2) {
                        for p in [w[0], w[1]] {
                            lines.extend([p[0] as f32, p[1] as f32, p[2] as f32, 0.0, 0.0, 1.0]);
                        }
                    }
                }
            }
        }
        SceneData { id, tris, lines }
    }
}

/// Translucent, depth-tested overlay triangles (face highlights, planes),
/// filled during the frame: position (3) + premultiplied colour (4).
pub type Overlay = std::sync::Arc<std::sync::Mutex<Vec<f32>>>;

/// Floats per overlay vertex: position, colour, "cut by the section view".
const OVERLAY_FLOATS: usize = 8;

/// A translucent triangle drawn depth-tested with the bodies. `cuttable`
/// triangles (face highlights) disappear with the section-cut part of the
/// bodies; planes stay whole, like in Fusion.
pub fn push_triangle(overlay: &Overlay, pts: [Vec3; 3], color: egui::Color32, cuttable: bool) {
    let c = color.to_normalized_gamma_f32();
    let cut = if cuttable { 1.0 } else { 0.0 };
    if let Ok(mut v) = overlay.lock() {
        for p in pts {
            v.extend([p[0] as f32, p[1] as f32, p[2] as f32, c[0], c[1], c[2], c[3], cut]);
        }
    }
}

struct Gpu {
    tri_pipeline: wgpu::RenderPipeline,
    line_pipeline: wgpu::RenderPipeline,
    overlay_pipeline: wgpu::RenderPipeline,
    overlay: Option<(wgpu::Buffer, u32)>,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    scene_id: u64,
    tris: Option<(wgpu::Buffer, u32)>,
    lines: Option<(wgpu::Buffer, u32)>,
}

/// Create the pipelines. Returns false if the app runs without wgpu.
pub fn init(cc: &eframe::CreationContext<'_>) -> bool {
    let Some(rs) = cc.wgpu_render_state.as_ref() else { return false };
    let device = &rs.device;
    let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("ferrocad bodies"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });
    let layout_entry = wgpu::BindGroupLayoutEntry {
        binding: 0,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    };
    let bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: None, entries: &[layout_entry] });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: None,
        bind_group_layouts: &[Some(&bgl)],
        immediate_size: 0,
    });
    let attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
    let buffers = [wgpu::VertexBufferLayout {
        array_stride: 24,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &attributes,
    }];
    let body_attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x3];
    let body_buffers = [wgpu::VertexBufferLayout {
        array_stride: 36,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &body_attributes,
    }];
    let pipeline = |topology, vs: &str, layouts: &[wgpu::VertexBufferLayout], fs: &str, bias: wgpu::DepthBiasState| {
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("ferrocad"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some(vs),
                compilation_options: Default::default(),
                buffers: &[Some(layouts[0].clone())],
            },
            primitive: wgpu::PrimitiveState { topology, ..Default::default() },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: Default::default(),
                bias,
            }),
            multisample: wgpu::MultisampleState { count: MSAA, ..Default::default() },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some(fs),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: rs.target_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
    };
    // Faces are pushed back a little so their edges win the depth test.
    let face_bias = wgpu::DepthBiasState { constant: 4, slope_scale: 1.5, clamp: 0.0 };
    let tri_pipeline = pipeline(wgpu::PrimitiveTopology::TriangleList, "vs_body", &body_buffers, "fs_main", face_bias);
    let line_pipeline =
        pipeline(wgpu::PrimitiveTopology::LineList, "vs_main", &buffers, "fs_line", Default::default());
    let overlay_attributes = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x4, 2 => Float32];
    let overlay_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("ferrocad overlay"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &module,
            entry_point: Some("vs_overlay"),
            compilation_options: Default::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: (OVERLAY_FLOATS * 4) as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &overlay_attributes,
            })],
        },
        primitive: wgpu::PrimitiveState::default(),
        // Tested against the bodies but not written, so overlays don't hide each other.
        depth_stencil: Some(wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::LessEqual),
            stencil: Default::default(),
            bias: Default::default(),
        }),
        multisample: wgpu::MultisampleState { count: MSAA, ..Default::default() },
        fragment: Some(wgpu::FragmentState {
            module: &module,
            entry_point: Some("fs_overlay"),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: rs.target_format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        multiview_mask: None,
        cache: None,
    });
    let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("ferrocad uniforms"),
        size: 32 * 4,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: None,
        layout: &bgl,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniforms.as_entire_binding() }],
    });
    rs.renderer.write().callback_resources.insert(Gpu {
        tri_pipeline,
        line_pipeline,
        overlay_pipeline,
        overlay: None,
        uniforms,
        bind_group,
        scene_id: u64::MAX,
        tris: None,
        lines: None,
    });
    true
}

/// View-projection matrix (column-major) matching `Camera::project` for the
/// viewport, with depth 0 at `near` and 1 at `far`.
pub fn view_proj(cam: &Camera, vp: &Viewport, near: f64, far: f64) -> [f32; 16] {
    let (right, up, forward) = cam.basis();
    let eye = cam.eye();
    let f = 1.0 / (cam.fov_y * 0.5).tan();
    let aspect = vp.width / vp.height.max(1.0);
    let a = far / (far - near);
    let b = -far * near / (far - near);
    let row = |v: Vec3, s: f64| [v[0] * s, v[1] * s, v[2] * s, -dot(v, eye) * s];
    let r0 = row(right, f / aspect);
    let r1 = row(up, f);
    let w = row(forward, 1.0);
    let r2 = [w[0] * a, w[1] * a, w[2] * a, w[3] * a + b];
    let rows = [r0, r1, r2, w];
    let mut m = [0.0f32; 16];
    for c in 0..4 {
        for r in 0..4 {
            m[c * 4 + r] = rows[r][c] as f32;
        }
    }
    m
}

pub struct SceneCallback {
    pub data: Arc<SceneData>,
    pub camera: Camera,
    pub viewport: Viewport,
    pub color: [f32; 3],
    pub section: Option<(Vec3, Vec3)>,
    /// Radius around the camera target that holds every body.
    pub radius: f64,
    pub overlay: Overlay,
}

/// Distance from `target` to the farthest overlay vertex (planes, face
/// highlights), so the depth range covers them as well as the bodies.
fn overlay_radius(overlay: &[f32], target: Vec3) -> f64 {
    overlay
        .as_chunks::<OVERLAY_FLOATS>()
        .0
        .iter()
        .map(|v| {
            let d = [v[0] as f64 - target[0], v[1] as f64 - target[1], v[2] as f64 - target[2]];
            (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
        })
        .fold(0.0, f64::max)
}

impl egui_wgpu::CallbackTrait for SceneCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &egui_wgpu::ScreenDescriptor,
        _encoder: &mut wgpu::CommandEncoder,
        resources: &mut egui_wgpu::CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        let Some(gpu) = resources.get_mut::<Gpu>() else { return Vec::new() };
        // The overlay is complete only now (it is filled after the callback is
        // queued), so the near/far range is decided here.
        let extra = self.overlay.lock().map(|v| overlay_radius(&v, self.camera.target)).unwrap_or(0.0);
        let u = uniforms(&self.camera, &self.viewport, self.color, self.section, self.radius.max(extra));
        queue.write_buffer(&gpu.uniforms, 0, bytemuck::cast_slice(&u));
        gpu.overlay = self.overlay.lock().ok().filter(|v| !v.is_empty()).map(|v| {
            let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("ferrocad overlay"),
                contents: bytemuck::cast_slice(&v),
                usage: wgpu::BufferUsages::VERTEX,
            });
            (buf, (v.len() / OVERLAY_FLOATS) as u32)
        });
        if gpu.scene_id != self.data.id {
            let upload = |data: &[f32], label, floats_per_vertex: usize| {
                (!data.is_empty()).then(|| {
                    let buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some(label),
                        contents: bytemuck::cast_slice(data),
                        usage: wgpu::BufferUsages::VERTEX,
                    });
                    (buf, (data.len() / floats_per_vertex) as u32)
                })
            };
            gpu.tris = upload(&self.data.tris, "ferrocad triangles", 9);
            gpu.lines = upload(&self.data.lines, "ferrocad edges", 6);
            gpu.scene_id = self.data.id;
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &egui_wgpu::CallbackResources,
    ) {
        let Some(gpu) = resources.get::<Gpu>() else { return };
        pass.set_bind_group(0, &gpu.bind_group, &[]);
        if let Some((buf, n)) = &gpu.tris {
            pass.set_pipeline(&gpu.tri_pipeline);
            pass.set_vertex_buffer(0, buf.slice(..));
            pass.draw(0..*n, 0..1);
        }
        if let Some((buf, n)) = &gpu.lines {
            pass.set_pipeline(&gpu.line_pipeline);
            pass.set_vertex_buffer(0, buf.slice(..));
            pass.draw(0..*n, 0..1);
        }
        if let Some((buf, n)) = &gpu.overlay {
            pass.set_pipeline(&gpu.overlay_pipeline);
            pass.set_vertex_buffer(0, buf.slice(..));
            pass.draw(0..*n, 0..1);
        }
    }
}

/// Near and far clip distances for everything within `radius` of the target.
fn depth_range(cam: &Camera, radius: f64) -> (f64, f64) {
    let near = (cam.distance - radius).max(cam.distance * 0.001).max(1e-3) * 0.5;
    let far = cam.distance + radius * 2.0 + 10.0;
    (near, far)
}

/// Uniforms for one frame.
pub fn uniforms(cam: &Camera, vp: &Viewport, color: [f32; 3], section: Option<(Vec3, Vec3)>, radius: f64) -> [f32; 32] {
    let (near, far) = depth_range(cam, radius);
    let mut u = [0.0f32; 32];
    u[..16].copy_from_slice(&view_proj(cam, vp, near, far));
    let eye = cam.eye();
    u[16..20].copy_from_slice(&[eye[0] as f32, eye[1] as f32, eye[2] as f32, 1.0]);
    u[20..24].copy_from_slice(&[color[0], color[1], color[2], 1.0]);
    if let Some((o, n)) = section {
        u[24..28].copy_from_slice(&[o[0] as f32, o[1] as f32, o[2] as f32, 0.0]);
        u[28..32].copy_from_slice(&[n[0] as f32, n[1] as f32, n[2] as f32, 1.0]);
    }
    u
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Origin planes reach far beyond a small part; their fills must not be
    /// clipped by a depth range sized for the part alone.
    #[test]
    fn depth_range_covers_overlay_planes() {
        let mut cam = Camera::default();
        cam.set_view(ferrocad_view::ViewPreset::Home);
        cam.target = [40.0, 30.0, 2.0];
        cam.distance = 25.0;
        let vp = Viewport { x: 0.0, y: 0.0, width: 800.0, height: 600.0 };
        let s = cam.distance * 4.0;
        let mut overlay = Vec::new();
        for c in [[0.0, 0.0, 0.0], [s, 0.0, 0.0], [s, s, 0.0], [0.0, s, 0.0], [0.0, 0.0, s], [0.0, s, s]] {
            overlay.extend([c[0] as f32, c[1] as f32, c[2] as f32, 1.0, 0.5, 0.0, 0.2, 0.0]);
        }
        let part_radius: f64 = 3.0;
        let radius = part_radius.max(overlay_radius(&overlay, cam.target));
        let (near, far) = depth_range(&cam, radius);
        for v in overlay.as_chunks::<OVERLAY_FLOATS>().0 {
            if let Some(p) = cam.project([v[0] as f64, v[1] as f64, v[2] as f64], &vp) {
                assert!(p[2] > near && p[2] < far, "depth {} outside {near}..{far}", p[2]);
            }
        }
        // The part alone would have clipped them.
        let (_, far_part) = depth_range(&cam, part_radius);
        assert!(overlay.as_chunks::<OVERLAY_FLOATS>().0.iter().any(|v| cam
            .project([v[0] as f64, v[1] as f64, v[2] as f64], &vp)
            .is_some_and(|p| p[2] > far_part)));
    }

    #[test]
    fn matrix_matches_camera_projection() {
        let mut cam = Camera::default();
        cam.set_view(ferrocad_view::ViewPreset::Home);
        cam.distance = 120.0;
        let vp = Viewport { x: 10.0, y: 20.0, width: 800.0, height: 600.0 };
        let m = view_proj(&cam, &vp, 1.0, 1000.0);
        for p in [[0.0, 0.0, 0.0], [30.0, -10.0, 5.0], [-20.0, 40.0, 25.0]] {
            let s = cam.project(p, &vp).unwrap();
            let c: Vec<f64> = (0..4)
                .map(|r| (0..3).map(|k| m[k * 4 + r] as f64 * p[k]).sum::<f64>() + m[12 + r] as f64)
                .collect();
            let (nx, ny) = (c[0] / c[3], c[1] / c[3]);
            let sx = vp.x + (nx + 1.0) * 0.5 * vp.width;
            let sy = vp.y + (1.0 - ny) * 0.5 * vp.height;
            assert!((sx - s[0]).abs() < 1e-2 && (sy - s[1]).abs() < 1e-2, "{p:?}: {sx},{sy} vs {s:?}");
            let z = c[2] / c[3];
            assert!((0.0..=1.0).contains(&z));
        }
    }
}
