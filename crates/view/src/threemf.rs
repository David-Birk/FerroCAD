//! 3MF export (3D Manufacturing Format, what slicers prefer over STL): a zip
//! package holding an XML model with one mesh object per body, in mm, with
//! the body colours. Vertices are welded so every mesh is watertight.

use ferrocad_model::TriMesh;
use std::collections::HashMap;
use std::fmt::Write as _;

/// One body to export.
pub struct Object<'a> {
    pub name: &'a str,
    pub mesh: &'a TriMesh,
    pub color: [u8; 3],
}

/// The complete .3mf file.
pub fn write(objects: &[Object]) -> Vec<u8> {
    let content_types = r#"<?xml version="1.0" encoding="UTF-8"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="model" ContentType="application/vnd.ms-package.3dmanufacturing-3dmodel+xml"/></Types>"#;
    let rels = r#"<?xml version="1.0" encoding="UTF-8"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Target="/3D/3dmodel.model" Id="rel0" Type="http://schemas.microsoft.com/3dmanufacturing/2013/01/3dmodel"/></Relationships>"#;
    let mut zip = Zip::default();
    zip.add("[Content_Types].xml", content_types.as_bytes());
    zip.add("_rels/.rels", rels.as_bytes());
    zip.add("3D/3dmodel.model", model_xml(objects).as_bytes());
    zip.finish()
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Shared vertices and triangles of a mesh: positions within 1e-5 mm are
/// one vertex, triangles that collapse are dropped.
pub fn weld(mesh: &TriMesh) -> (Vec<[f64; 3]>, Vec<[u32; 3]>) {
    let key = |p: [f64; 3]| p.map(|v| (v * 1e5).round() as i64);
    let mut index: HashMap<[i64; 3], u32> = HashMap::new();
    let mut verts = Vec::new();
    let remap: Vec<u32> = mesh
        .positions
        .iter()
        .map(|&p| {
            *index.entry(key(p)).or_insert_with(|| {
                verts.push(p);
                (verts.len() - 1) as u32
            })
        })
        .collect();
    let tris = mesh
        .indices
        .chunks_exact(3)
        .map(|t| [remap[t[0] as usize], remap[t[1] as usize], remap[t[2] as usize]])
        .filter(|t| t[0] != t[1] && t[1] != t[2] && t[0] != t[2])
        .collect();
    (verts, tris)
}

fn model_xml(objects: &[Object]) -> String {
    let mut x = String::new();
    x.push_str(r#"<?xml version="1.0" encoding="UTF-8"?>"#);
    x.push_str("\n<model unit=\"millimeter\" xml:lang=\"en-US\" xmlns=\"http://schemas.microsoft.com/3dmanufacturing/core/2015/02\">\n");
    x.push_str("<metadata name=\"Application\">ferrocad</metadata>\n<resources>\n<basematerials id=\"1\">\n");
    for o in objects {
        let [r, g, b] = o.color;
        let _ = writeln!(x, "<base name=\"{}\" displaycolor=\"#{r:02X}{g:02X}{b:02X}\"/>", escape(o.name));
    }
    x.push_str("</basematerials>\n");
    for (i, o) in objects.iter().enumerate() {
        let (verts, tris) = weld(o.mesh);
        let _ = writeln!(x, "<object id=\"{}\" type=\"model\" name=\"{}\" pid=\"1\" pindex=\"{i}\">", i + 2, escape(o.name));
        x.push_str("<mesh>\n<vertices>\n");
        for v in &verts {
            let _ = writeln!(x, "<vertex x=\"{:.6}\" y=\"{:.6}\" z=\"{:.6}\"/>", v[0], v[1], v[2]);
        }
        x.push_str("</vertices>\n<triangles>\n");
        for t in &tris {
            let _ = writeln!(x, "<triangle v1=\"{}\" v2=\"{}\" v3=\"{}\"/>", t[0], t[1], t[2]);
        }
        x.push_str("</triangles>\n</mesh>\n</object>\n");
    }
    x.push_str("</resources>\n<build>\n");
    for i in 0..objects.len() {
        let _ = writeln!(x, "<item objectid=\"{}\"/>", i + 2);
    }
    x.push_str("</build>\n</model>\n");
    x
}

fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, t) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *t = c;
    }
    !data.iter().fold(!0u32, |c, &b| table[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8))
}

/// A minimal zip writer: stored (uncompressed) entries, which every 3MF
/// reader accepts.
#[derive(Default)]
struct Zip {
    data: Vec<u8>,
    central: Vec<u8>,
    count: u16,
}

impl Zip {
    fn add(&mut self, name: &str, content: &[u8]) {
        let crc = crc32(content);
        let offset = self.data.len() as u32;
        let size = content.len() as u32;
        let header = |sig: u32, central: bool| {
            let mut h = Vec::new();
            h.extend(sig.to_le_bytes());
            if central {
                h.extend(20u16.to_le_bytes()); // made by
            }
            h.extend(20u16.to_le_bytes()); // version needed
            h.extend(0u16.to_le_bytes()); // flags
            h.extend(0u16.to_le_bytes()); // stored
            h.extend(0u16.to_le_bytes()); // time
            h.extend(0x21u16.to_le_bytes()); // date: 1980-01-01
            h.extend(crc.to_le_bytes());
            h.extend(size.to_le_bytes());
            h.extend(size.to_le_bytes());
            h.extend((name.len() as u16).to_le_bytes());
            h.extend(0u16.to_le_bytes()); // extra
            if central {
                h.extend(0u16.to_le_bytes()); // comment
                h.extend(0u16.to_le_bytes()); // disk
                h.extend(0u16.to_le_bytes()); // internal attributes
                h.extend(0u32.to_le_bytes()); // external attributes
                h.extend(offset.to_le_bytes());
            }
            h.extend(name.as_bytes());
            h
        };
        self.data.extend(header(0x0403_4b50, false));
        self.data.extend(content);
        self.central.extend(header(0x0201_4b50, true));
        self.count += 1;
    }

    fn finish(mut self) -> Vec<u8> {
        let start = self.data.len() as u32;
        let len = self.central.len() as u32;
        self.data.extend(&self.central);
        self.data.extend(0x0605_4b50u32.to_le_bytes());
        self.data.extend([0u8; 4]); // disk numbers
        self.data.extend(self.count.to_le_bytes());
        self.data.extend(self.count.to_le_bytes());
        self.data.extend(len.to_le_bytes());
        self.data.extend(start.to_le_bytes());
        self.data.extend(0u16.to_le_bytes());
        self.data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A unit cube with per-face vertices, like a CAD tessellation.
    fn cube() -> TriMesh {
        let c = |i: usize| [(i & 1) as f64, ((i >> 1) & 1) as f64, ((i >> 2) & 1) as f64];
        let faces = [[0, 2, 3, 1], [4, 5, 7, 6], [0, 1, 5, 4], [2, 6, 7, 3], [0, 4, 6, 2], [1, 3, 7, 5]];
        let mut m = TriMesh { positions: vec![], normals: vec![], indices: vec![] };
        for f in faces {
            let base = m.positions.len() as u32;
            m.positions.extend(f.map(c));
            m.indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        m
    }

    #[test]
    fn welded_watertight_mesh_in_a_valid_zip() {
        let mesh = cube();
        let (v, t) = weld(&mesh);
        assert_eq!((v.len(), t.len()), (8, 12));
        // Watertight: every edge is used by exactly two triangles.
        let mut edges: HashMap<(u32, u32), usize> = HashMap::new();
        for tri in &t {
            for k in 0..3 {
                let (a, b) = (tri[k], tri[(k + 1) % 3]);
                *edges.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
        assert!(edges.values().all(|n| *n == 2));

        let bytes = write(&[Object { name: "Body <1>", mesh: &mesh, color: [200, 100, 50] }]);
        assert_eq!(&bytes[..4], b"PK\x03\x04");
        assert_eq!(&bytes[bytes.len() - 22..bytes.len() - 18], b"PK\x05\x06");
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.contains("unit=\"millimeter\""));
        assert!(text.contains("name=\"Body &lt;1&gt;\""));
        assert!(text.contains("displaycolor=\"#C86432\""));
        assert_eq!(text.matches("<triangle ").count(), 12);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }
}
