//! `nfsh mesh`: one mesh's LODs and sections from a store, as text; its first LOD
//! decoded, and optionally written as a Wavefront .obj for a look in any viewer.

use std::fmt::Write;
use std::path::Path;

use frostbite::Result;
use frostbite::mesh::{Bounds, Geometry, Meshes};
use frostbite::store::Store;

fn bounds(b: &Bounds) -> String {
    let v = |p: [f32; 3]| format!("{:.3} {:.3} {:.3}", p[0], p[1], p[2]);
    format!("{} .. {}", v(b.min), v(b.max))
}

/// Reads the mesh asset `name` from the store in `dir` and describes it; writes its
/// first LOD to `obj` when given.
pub fn run(dir: &Path, name: &str, obj: Option<&Path>) -> Result<String> {
    let store = Store::open(dir)?;
    let mut meshes = Meshes::new(&store)?;
    let set = meshes.by_name(name)?;
    let mut out = String::new();
    let _ = writeln!(out, "mesh set {} (kind {})", set.name, set.kind);
    let _ = writeln!(out, "bounds {}", bounds(&set.bounds));
    for lod in &set.lods {
        let chunk = match lod.chunk {
            Some(id) => id.iter().map(|b| format!("{b:02x}")).collect(),
            None => "inline".to_owned(),
        };
        let _ = writeln!(
            out,
            "\n{}: chunk {chunk}, {} vertex bytes, {} index bytes ({}-byte indices), flags {:#x}",
            lod.name, lod.vertex_bytes, lod.index_bytes, lod.index_width, lod.flags
        );
        for s in &lod.sections {
            let elements: Vec<String> = s
                .elements
                .iter()
                .map(|e| format!("{:02x}/{:02x}", e.usage, e.format))
                .collect();
            let _ = writeln!(
                out,
                "  {:?} material {}: {} triangles, {} vertices, elements {} | {}",
                s.name,
                s.material,
                s.triangles,
                s.vertices,
                elements.join(" "),
                bounds(&s.bounds)
            );
        }
    }

    let Some(lod) = set.lods.first() else {
        return Ok(out);
    };
    let data = meshes.chunk(lod)?;
    let sections: Vec<Geometry> = lod
        .sections
        .iter()
        .map(|s| lod.section(s, &data))
        .collect::<Result<_>>()?;
    let vertices: usize = sections.iter().map(|g| g.positions.len()).sum();
    let triangles: usize = sections.iter().map(|g| g.indices.len() / 3).sum();
    let _ = writeln!(
        out,
        "\n{} decoded: {vertices} vertices, {triangles} triangles",
        lod.name
    );
    if let Some(path) = obj {
        std::fs::write(path, wavefront(&lod.name, &lod.sections, &sections))
            .map_err(|e| frostbite::Error(format!("{}: {e}", path.display())))?;
        let _ = writeln!(out, "written to {}", path.display());
    }
    Ok(out)
}

/// The sections as one .obj, a group each; texture coordinates where a section has
/// them (v flipped, as .obj counts it from the bottom).
fn wavefront(name: &str, sections: &[frostbite::mesh::Section], geometry: &[Geometry]) -> String {
    let mut out = format!("# {name}, written by nfsh\n");
    let (mut base, mut uv_base) = (1, 1);
    for (s, g) in sections.iter().zip(geometry) {
        let _ = writeln!(
            out,
            "g {}",
            if s.name.is_empty() {
                "unnamed"
            } else {
                &s.name
            }
        );
        for p in &g.positions {
            let _ = writeln!(out, "v {} {} {}", p[0], p[1], p[2]);
        }
        if let Some(uvs) = &g.uvs {
            for t in uvs {
                let _ = writeln!(out, "vt {} {}", t[0], 1.0 - t[1]);
            }
        }
        for t in g.indices.as_chunks::<3>().0 {
            let _ = match &g.uvs {
                Some(_) => writeln!(
                    out,
                    "f {}/{} {}/{} {}/{}",
                    t[0] as usize + base,
                    t[0] as usize + uv_base,
                    t[1] as usize + base,
                    t[1] as usize + uv_base,
                    t[2] as usize + base,
                    t[2] as usize + uv_base
                ),
                None => writeln!(
                    out,
                    "f {} {} {}",
                    t[0] as usize + base,
                    t[1] as usize + base,
                    t[2] as usize + base
                ),
            };
        }
        base += g.positions.len();
        uv_base += g.uvs.as_ref().map_or(0, Vec::len);
    }
    out
}
