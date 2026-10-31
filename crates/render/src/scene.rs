//! A zone made ready to draw, on the CPU: every placed mesh's first LOD packed into
//! one vertex and one index buffer, its placements into one instance buffer, and one
//! instanced draw per mesh.

use frostbite::level::{Mesh, Transform, Zone};
use frostbite::mesh::{Geometry, Meshes};

/// A placement's transform as the vertex shader reads it: right, up and forward,
/// each followed by one coordinate of the translation.
pub type Instance = [f32; 12];

/// One instanced indexed draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Draw {
    pub first_index: u32,
    pub index_count: u32,
    pub vertex_offset: i32,
    pub first_instance: u32,
    pub instance_count: u32,
}

/// What a zone draws, and what it could not.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Scene {
    pub positions: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
    pub instances: Vec<Instance>,
    pub draws: Vec<Draw>,
    /// Of where the drawn meshes are placed (their origins, not their vertices: some
    /// meshes' vertices are authored far from their origin, e.g. a speedway stand's
    /// steps already in world space, which their placements then move kilometres
    /// away); `None` if nothing is drawn.
    pub bounds: Option<([f32; 3], [f32; 3])>,
    /// Meshes that could not be read, with why, and how many placements that left
    /// out.
    pub failed: Vec<(String, String)>,
    pub placements_left_out: usize,
}

impl Scene {
    pub fn triangles(&self) -> usize {
        self.draws
            .iter()
            .map(|d| d.index_count as usize / 3 * d.instance_count as usize)
            .sum()
    }
}

fn instance(t: &Transform) -> Instance {
    let [r, u, f, o] = [t.right, t.up, t.forward, t.trans];
    [
        r[0], r[1], r[2], o[0], u[0], u[1], u[2], o[1], f[0], f[1], f[2], o[2],
    ]
}

/// The draws for `zone`, its meshes' geometry from `load` (each mesh's sections). A
/// mesh that fails to load, or has no triangles, is left out and listed.
pub fn build(
    zone: &Zone,
    mut load: impl FnMut(&Mesh) -> frostbite::Result<Vec<Geometry>>,
) -> Scene {
    let mut scene = Scene::default();
    let mut by_mesh: Vec<Vec<&Transform>> = vec![Vec::new(); zone.meshes.len()];
    for p in &zone.placements {
        // Level::zone only makes valid indices; a bad one is left out, not a panic.
        if let Some(list) = by_mesh.get_mut(p.mesh) {
            list.push(&p.transform);
        }
    }
    let (mut min, mut max) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for (mesh, transforms) in zone.meshes.iter().zip(&by_mesh) {
        if transforms.is_empty() {
            continue;
        }
        let sections = match load(mesh) {
            Ok(s) => s,
            Err(e) => {
                scene.failed.push((mesh.name.clone(), e.0));
                scene.placements_left_out += transforms.len();
                continue;
            }
        };
        let (first_vertex, first_index) = (scene.positions.len(), scene.indices.len());
        for g in &sections {
            let base = (scene.positions.len() - first_vertex) as u32;
            scene.positions.extend_from_slice(&g.positions);
            scene.indices.extend(g.indices.iter().map(|i| i + base));
        }
        let index_count = scene.indices.len() - first_index;
        if index_count == 0 {
            scene.positions.truncate(first_vertex);
            scene
                .failed
                .push((mesh.name.clone(), "no triangles".into()));
            scene.placements_left_out += transforms.len();
            continue;
        }
        for t in transforms {
            for k in 0..3 {
                min[k] = min[k].min(t.trans[k]);
                max[k] = max[k].max(t.trans[k]);
            }
        }
        scene.draws.push(Draw {
            first_index: first_index as u32,
            index_count: index_count as u32,
            vertex_offset: first_vertex as i32,
            first_instance: scene.instances.len() as u32,
            instance_count: transforms.len() as u32,
        });
        scene
            .instances
            .extend(transforms.iter().map(|t| instance(t)));
    }
    if !scene.draws.is_empty() && min.iter().chain(&max).all(|v| v.is_finite()) {
        scene.bounds = Some((min, max));
    }
    scene
}

/// Every section of the first LOD of the mesh asset `mesh`, from the store.
pub fn first_lod(meshes: &mut Meshes, mesh: &Mesh) -> frostbite::Result<Vec<Geometry>> {
    let set = meshes.by_partition(&mesh.partition)?;
    let Some(lod) = set.lods.first() else {
        return Err(frostbite::Error(format!("{}: no LODs", mesh.name)));
    };
    let data = meshes.chunk(lod)?;
    lod.sections.iter().map(|s| lod.section(s, &data)).collect()
}

/// A view of the whole scene from above one corner of its bounds, looking at the
/// middle; at the origin looking along +z when there is nothing to look at.
pub fn overview(scene: &Scene, width: u32, height: u32) -> crate::Camera {
    let Some((min, max)) = scene.bounds else {
        return crate::Camera::new([0.0; 3], 0.0, 0.0, width, height);
    };
    let centre = [0, 1, 2].map(|k| (min[k] + max[k]) / 2.0);
    let size = (max[0] - min[0]).max(max[2] - min[2]).max(1.0);
    let eye = [
        centre[0] - size * 0.45,
        max[1] + size * 0.3,
        centre[2] - size * 0.45,
    ];
    crate::Camera::looking_at(eye, centre, width, height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use frostbite::level::Placement;

    fn mesh(name: &str) -> Mesh {
        Mesh {
            name: name.into(),
            kind: "RigidMeshAsset".into(),
            partition: [0; 16],
        }
    }

    fn at(mesh: usize, x: f32) -> Placement {
        let mut transform = Transform::IDENTITY;
        transform.trans = [x, 0.0, 0.0];
        Placement { mesh, transform }
    }

    fn triangle() -> Geometry {
        Geometry {
            positions: vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            uvs: None,
            indices: vec![0, 1, 2],
        }
    }

    #[test]
    fn packs_meshes_once_and_placements_as_instances() {
        let zone = Zone {
            meshes: vec![mesh("a"), mesh("b"), mesh("unplaced")],
            placements: vec![at(1, 10.0), at(0, 0.0), at(1, 20.0)],
            ..Zone::default()
        };
        let mut loaded = Vec::new();
        let scene = build(&zone, |m| {
            loaded.push(m.name.clone());
            // b has two sections: its second's indices count from its own vertices.
            Ok(match m.name.as_str() {
                "b" => vec![triangle(), triangle()],
                _ => vec![triangle()],
            })
        });
        assert_eq!(loaded, ["a", "b"]);
        assert_eq!(scene.positions.len(), 9);
        assert_eq!(scene.indices, [0, 1, 2, 0, 1, 2, 3, 4, 5]);
        assert_eq!(
            scene.draws,
            [
                Draw {
                    first_index: 0,
                    index_count: 3,
                    vertex_offset: 0,
                    first_instance: 0,
                    instance_count: 1
                },
                Draw {
                    first_index: 3,
                    index_count: 6,
                    vertex_offset: 3,
                    first_instance: 1,
                    instance_count: 2
                },
            ]
        );
        // b's instances in placement order, the translation in each row's last value.
        assert_eq!(scene.instances[1][3], 10.0);
        assert_eq!(scene.instances[2][3], 20.0);
        assert_eq!(scene.instances[2][..3], [1.0, 0.0, 0.0]);
        assert_eq!(scene.triangles(), 1 + 2 * 2);
        assert_eq!(scene.bounds, Some(([0.0; 3], [20.0, 0.0, 0.0])));
        assert!(scene.failed.is_empty());
    }

    #[test]
    fn meshes_that_fail_are_listed_not_drawn() {
        let zone = Zone {
            meshes: vec![mesh("broken"), mesh("empty"), mesh("good")],
            placements: vec![at(0, 0.0), at(0, 1.0), at(1, 0.0), at(2, 5.0), at(9, 0.0)],
            ..Zone::default()
        };
        let scene = build(&zone, |m| match m.name.as_str() {
            "broken" => Err(frostbite::Error("no chunk".into())),
            "empty" => Ok(vec![Geometry {
                positions: vec![[0.0; 3]],
                ..Geometry::default()
            }]),
            _ => Ok(vec![triangle()]),
        });
        assert_eq!(
            scene.failed,
            [
                ("broken".to_owned(), "no chunk".to_owned()),
                ("empty".to_owned(), "no triangles".to_owned())
            ]
        );
        assert_eq!(scene.placements_left_out, 3);
        assert_eq!(scene.positions.len(), 3);
        assert_eq!(scene.draws.len(), 1);
        assert_eq!(scene.draws[0].vertex_offset, 0);
    }

    #[test]
    fn nothing_to_draw_has_no_bounds() {
        let scene = build(&Zone::default(), |_| Ok(vec![triangle()]));
        assert_eq!(scene, Scene::default());
        let c = overview(&scene, 640, 480);
        assert_eq!(c.origin, [0.0; 3]);
    }

    /// Builds zone 495 (the speedway) from the store `NFSH_STORE` points at (local-ci
    /// sets it on the Windows side): every mesh it places is drawn.
    #[test]
    fn builds_the_speedway_zone() {
        let Some(dir) = std::env::var_os("NFSH_STORE") else {
            eprintln!("NFSH_STORE not set; skipped");
            return;
        };
        let store = frostbite::store::Store::open(std::path::Path::new(&dir)).unwrap();
        let zone = frostbite::level::Level::new(&store, "Win32/levels/mainlevel/mainlevel")
            .unwrap()
            .zone("levels/mainlevel/worldart_zs_495")
            .unwrap();
        let mut meshes = Meshes::new(&store).unwrap();
        let scene = build(&zone, |m| first_lod(&mut meshes, m));
        assert_eq!(scene.failed, []);
        // Counted on the owner's store, 2026-10-31.
        assert_eq!(zone.placements.len(), 2171);
        assert_eq!(scene.draws.len(), 199);
        assert_eq!(scene.instances.len(), 2171);
        assert_eq!(scene.positions.len(), 1_356_354);
        assert_eq!(scene.triangles(), 3_988_832);
    }
}
