//! A level's graph, read from nfsh's store: the bundles a level loads (sub-levels and
//! streaming zones, found by name) and what a zone places, each mesh with where it
//! goes in the world. See `docs/FORMATS.md`, "Palm City".
//!
//! Two kinds of link hold a level together. A sub-level is a *bundle*, named by a
//! `BundleName` string; its root partition has the bundle's name. Everything else is a
//! class reference, often into another partition (an import), found by partition GUID
//! with [`Store::partitions`]; a partition copied into several bundles is taken from
//! the level's own superbundle when it has a copy.
//!
//! What is followed is listed by type name; every other object is counted in
//! [`Zone::skipped`], so the report shows what the walk does not yet draw.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::rc::Rc;

use crate::ebx::{Object, Partition, Ref, Value};
use crate::store::{EbxRef, Entries, Store};
use crate::{Result, bail};

/// How deep blueprints may nest inside each other before the walk gives up on a
/// branch (real ones nest a few levels; this only stops a cycle the walk missed).
const MAX_DEPTH: usize = 32;

/// The bundles a sub-level link names are listed with this prefix.
const PLATFORM: &str = "win32/";

/// Objects that link to a sub-level bundle by its `BundleName`.
const SUB_LEVEL_LINKS: &[&str] = &["SubWorldReferenceObjectData", "ZoneStreamerSubWorldRod"];

/// Objects that place a blueprint (`Blueprint`) at `BlueprintTransform`.
const BLUEPRINT_REFERENCES: &[&str] = &[
    "ObjectReferenceObjectData",
    "SpatialPrefabReferenceObjectData",
    "LayerReferenceObjectData",
];

/// Blueprints whose content is a list of objects (`Objects`).
const OBJECT_LISTS: &[&str] = &[
    "SubWorldData",
    "LayerData",
    "SpatialPrefabBlueprint",
    "PrefabBlueprint",
];

/// A Frostbite `LinearTransform`: the local axes and the origin, in the parent's
/// space. A point (x, y, z) goes to x·right + y·up + z·forward + trans.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transform {
    pub right: [f32; 3],
    pub up: [f32; 3],
    pub forward: [f32; 3],
    pub trans: [f32; 3],
}

impl Transform {
    pub const IDENTITY: Self = Self {
        right: [1.0, 0.0, 0.0],
        up: [0.0, 1.0, 0.0],
        forward: [0.0, 0.0, 1.0],
        trans: [0.0, 0.0, 0.0],
    };

    /// A direction in the parent's space.
    pub fn rotate(&self, v: [f32; 3]) -> [f32; 3] {
        std::array::from_fn(|i| v[0] * self.right[i] + v[1] * self.up[i] + v[2] * self.forward[i])
    }

    /// A point in the parent's space.
    pub fn apply(&self, p: [f32; 3]) -> [f32; 3] {
        let r = self.rotate(p);
        std::array::from_fn(|i| r[i] + self.trans[i])
    }

    /// This transform, given in `parent`'s space, in the space `parent` is given in.
    pub fn then(&self, parent: &Self) -> Self {
        Self {
            right: parent.rotate(self.right),
            up: parent.rotate(self.up),
            forward: parent.rotate(self.forward),
            trans: parent.apply(self.trans),
        }
    }
}

/// A mesh a zone places.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mesh {
    /// The ebx it is in.
    pub name: String,
    /// Its type, e.g. `RigidMeshAsset`.
    pub kind: String,
    pub partition: [u8; 16],
}

/// One mesh in the world.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    /// Index into [`Zone::meshes`].
    pub mesh: usize,
    pub transform: Transform,
}

/// What one zone places.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Zone {
    /// Every mesh placed, each once.
    pub meshes: Vec<Mesh>,
    pub placements: Vec<Placement>,
    /// Objects the walk met but does not follow, by type name.
    pub skipped: BTreeMap<String, usize>,
    /// References the store can't resolve: the partition or the instance is missing.
    pub missing: usize,
}

/// A bundle a level loads, and the link that names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubLevel {
    /// As the link names it, lowercased (`levels/mainlevel/worldart_zs_495`).
    pub name: String,
    /// 0 for the level itself.
    pub depth: usize,
    /// The ebx whose link names it; empty for the level itself.
    pub from: String,
    /// Whether the store has the bundle.
    pub found: bool,
}

/// A partition read from the store, with the bytes it borrows.
struct Loaded {
    data: Vec<u8>,
    name: String,
}

/// Reads a level's graph from a store.
pub struct Level<'s> {
    store: &'s Store,
    entries: Entries<'s>,
    /// The superbundle whose copies of a partition are preferred.
    home: Option<u32>,
    /// Lowercased bundle name without the platform prefix → where it is.
    bundles: HashMap<String, (u32, u32)>,
    /// A blueprint's walk in its own space, by (partition, instance) GUID.
    blueprints: HashMap<([u8; 16], [u8; 16]), Rc<Walk>>,
    meshes: HashMap<([u8; 16], [u8; 16]), Option<Mesh>>,
    /// Blueprints being expanded, to stop a cycle.
    open: HashSet<([u8; 16], [u8; 16])>,
}

/// What one walk collects; a blueprint's walk starts its own.
#[derive(Default)]
struct Walk {
    placed: Vec<(Mesh, Transform)>,
    skipped: BTreeMap<String, usize>,
    missing: usize,
}

impl Walk {
    fn skip(&mut self, what: impl Into<String>) {
        *self.skipped.entry(what.into()).or_default() += 1;
    }
}

/// The bundle name a link names, as the store lists it: lowercased, without the
/// platform prefix.
pub fn bundle_key(name: &str) -> String {
    let name = name.to_ascii_lowercase();
    match name.strip_prefix(PLATFORM) {
        Some(rest) => rest.to_owned(),
        None => name,
    }
}

/// The value of the field `name` of `o`, if its type has one.
fn field<'a>(p: &Partition<'a>, o: Object, name: &str) -> Result<Option<Value<'a>>> {
    for f in p.fields_of(o.type_index)? {
        if p.name(f.name_hash) == Some(name) {
            return Ok(Some(p.value(f, o.base)?));
        }
    }
    Ok(None)
}

fn class_ref(p: &Partition, o: Object, name: &str) -> Result<Ref> {
    match field(p, o, name)? {
        Some(Value::Class(r)) => Ok(r),
        Some(other) => bail!("{}: {name} is {other:?}, not a reference", p.name),
        None => bail!("{}: {} has no {name}", p.name, p.type_name(o.type_index)),
    }
}

/// The elements of the array field `name` of `o`.
fn elements<'a>(p: &Partition<'a>, o: Object, name: &str) -> Result<Vec<Value<'a>>> {
    let Some(Value::Array(i, _)) = field(p, o, name)? else {
        bail!(
            "{}: {} has no array {name}",
            p.name,
            p.type_name(o.type_index)
        );
    };
    let Some(&array) = p.arrays.get(i) else {
        bail!("{}: array {i} is out of range", p.name);
    };
    (0..array.count).map(|e| p.element(&array, e)).collect()
}

fn float(p: &Partition, o: Object, name: &str) -> Result<f32> {
    match field(p, o, name)? {
        Some(Value::Float32(x)) => Ok(x),
        other => bail!("{}: {name} is {other:?}, not a float", p.name),
    }
}

fn vec3(p: &Partition, v: Option<Value>) -> Result<[f32; 3]> {
    let Some(Value::Struct(o)) = v else {
        bail!("{}: {v:?} is not a Vec3", p.name);
    };
    Ok([float(p, o, "x")?, float(p, o, "y")?, float(p, o, "z")?])
}

/// A `LinearTransform` value.
fn transform(p: &Partition, v: Option<Value>) -> Result<Transform> {
    let Some(Value::Struct(o)) = v else {
        bail!("{}: {v:?} is not a LinearTransform", p.name);
    };
    Ok(Transform {
        right: vec3(p, field(p, o, "right")?)?,
        up: vec3(p, field(p, o, "up")?)?,
        forward: vec3(p, field(p, o, "forward")?)?,
        trans: vec3(p, field(p, o, "trans")?)?,
    })
}

fn instance_object(p: &Partition, index: usize) -> Result<Object> {
    match p.instances.get(index) {
        Some(i) => Ok(p.object(i)),
        None => bail!("{}: instance {index} is out of range", p.name),
    }
}

impl<'s> Level<'s> {
    /// A reader for the levels in `store`, preferring copies of partitions in the
    /// superbundle `home` (e.g. `Win32/levels/mainlevel/mainlevel`).
    pub fn new(store: &'s Store, home: &str) -> Result<Self> {
        let mut bundles = HashMap::new();
        for (sb, s) in store.superbundles.iter().zip(0..) {
            for (b, n) in sb.bundles.iter().zip(0..) {
                bundles.entry(bundle_key(&b.name)).or_insert((s, n));
            }
        }
        let home = store
            .superbundles
            .iter()
            .position(|sb| sb.name.eq_ignore_ascii_case(home))
            .map(|i| i as u32);
        Ok(Self {
            store,
            entries: store.entries()?,
            home,
            bundles,
            blueprints: HashMap::new(),
            meshes: HashMap::new(),
            open: HashSet::new(),
        })
    }

    fn bundle(&self, name: &str) -> Option<&'s crate::install::BundleEntry> {
        let &(s, n) = self.bundles.get(&bundle_key(name))?;
        Some(&self.store.superbundles[s as usize].bundles[n as usize])
    }

    /// Every bundle the level `root` (e.g. `levels/mainlevel/mainlevel`) loads, found
    /// by following the sub-level links in every ebx of each bundle, breadth first;
    /// each listed once, at the depth it is first reached.
    pub fn sub_levels(&mut self, root: &str) -> Result<Vec<SubLevel>> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        let mut queue = VecDeque::from([(bundle_key(root), 0, String::new())]);
        while let Some((name, depth, from)) = queue.pop_front() {
            if !seen.insert(name.clone()) {
                continue;
            }
            let bundle = self.bundle(&name);
            out.push(SubLevel {
                name,
                depth,
                from,
                found: bundle.is_some(),
            });
            let Some(bundle) = bundle else { continue };
            for asset in &bundle.bundle.ebx {
                let data = self.entries.read(asset.location)?;
                let p = Partition::parse(&data)
                    .map_err(|e| crate::Error(format!("{}: {e}", asset.name)))?;
                for (i, instance) in p.instances.iter().enumerate() {
                    if !SUB_LEVEL_LINKS.contains(&p.type_name(instance.type_index).as_str()) {
                        continue;
                    }
                    match field(&p, instance_object(&p, i)?, "BundleName")? {
                        Some(Value::String(Some(link))) => {
                            queue.push_back((bundle_key(link), depth + 1, asset.name.clone()))
                        }
                        other => bail!("{}: a sub-level link to {other:?}", p.name),
                    }
                }
            }
        }
        Ok(out)
    }

    /// What the zone bundle `name` places: the objects of its root partition's
    /// `SubWorldData`, followed through blueprints, in world space.
    pub fn zone(&mut self, name: &str) -> Result<Zone> {
        let Some(bundle) = self.bundle(name) else {
            bail!("no bundle {name} in the store");
        };
        let key = bundle_key(name);
        let Some(asset) = bundle
            .bundle
            .ebx
            .iter()
            .find(|a| bundle_key(&a.name) == key)
        else {
            bail!("bundle {name} has no root partition {key}");
        };
        let data = self.entries.read(asset.location)?;
        let p =
            Partition::parse(&data).map_err(|e| crate::Error(format!("{}: {e}", asset.name)))?;
        let Some(root) = p
            .instances
            .iter()
            .position(|i| p.type_name(i.type_index) == "SubWorldData")
        else {
            bail!("{}: no SubWorldData", asset.name);
        };
        let mut walk = Walk::default();
        self.blueprint(&p, root, Transform::IDENTITY, &mut walk, 0)?;

        let mut zone = Zone {
            skipped: walk.skipped,
            missing: walk.missing,
            ..Zone::default()
        };
        let mut index: HashMap<[u8; 16], usize> = HashMap::new();
        for (mesh, transform) in walk.placed {
            let mesh = *index.entry(mesh.partition).or_insert_with(|| {
                zone.meshes.push(mesh);
                zone.meshes.len() - 1
            });
            zone.placements.push(Placement { mesh, transform });
        }
        Ok(zone)
    }

    /// Reads the partition `guid`, preferring the home superbundle's copy; `None` if
    /// the store has none.
    fn load(&mut self, guid: &[u8; 16]) -> Result<Option<Loaded>> {
        let copies: Vec<EbxRef> = self.store.partitions(guid)?.collect();
        let pick = copies
            .iter()
            .find(|c| Some(c.superbundle) == self.home)
            .or(copies.first());
        let Some(asset) = pick.and_then(|&c| self.store.ebx_at(c)).map(|(_, a)| a) else {
            return Ok(None);
        };
        Ok(Some(Loaded {
            data: self.entries.read(asset.location)?,
            name: asset.name.clone(),
        }))
    }

    /// Places the objects of the instance `index` of `p` (an object list or an
    /// `ObjectBlueprint`) at `at`.
    fn blueprint(
        &mut self,
        p: &Partition,
        index: usize,
        at: Transform,
        walk: &mut Walk,
        depth: usize,
    ) -> Result<()> {
        if depth > MAX_DEPTH {
            walk.skip("(nested too deep)");
            return Ok(());
        }
        let o = instance_object(p, index)?;
        let ty = p.type_name(o.type_index);
        if ty == "ObjectBlueprint" {
            return self.reference(p, class_ref(p, o, "Object")?, at, walk, depth);
        }
        if !OBJECT_LISTS.contains(&ty.as_str()) {
            walk.skip(ty);
            return Ok(());
        }
        for v in elements(p, o, "Objects")? {
            match v {
                Value::Class(r) => self.reference(p, r, at, walk, depth)?,
                other => bail!("{}: an object list holds {other:?}", p.name),
            }
        }
        Ok(())
    }

    /// Places the object `r` refers to, from `p`.
    fn reference(
        &mut self,
        p: &Partition,
        r: Ref,
        at: Transform,
        walk: &mut Walk,
        depth: usize,
    ) -> Result<()> {
        match r {
            Ref::Null => Ok(()),
            Ref::Internal(i) => self.object(p, i, at, walk, depth),
            Ref::Import(i) => {
                let import = p.imports[i];
                let Some(loaded) = self.load(&import.partition)? else {
                    walk.missing += 1;
                    return Ok(());
                };
                let q = Partition::parse(&loaded.data)
                    .map_err(|e| crate::Error(format!("{}: {e}", loaded.name)))?;
                match q
                    .instances
                    .iter()
                    .position(|x| x.guid == Some(import.instance))
                {
                    Some(i) => self.object(&q, i, at, walk, depth),
                    None => {
                        walk.missing += 1;
                        Ok(())
                    }
                }
            }
        }
    }

    /// Places the instance `index` of `p` at `at`, if it is something drawn.
    fn object(
        &mut self,
        p: &Partition,
        index: usize,
        at: Transform,
        walk: &mut Walk,
        depth: usize,
    ) -> Result<()> {
        let o = instance_object(p, index)?;
        let ty = p.type_name(o.type_index);
        match ty.as_str() {
            "StaticModelGroupEntityData" => {
                let group = transform(p, field(p, o, "Transform")?)?.then(&at);
                for member in elements(p, o, "MemberDatas")? {
                    let Value::Struct(m) = member else {
                        bail!("{}: a group member is {member:?}", p.name);
                    };
                    let Some(mesh) = self.mesh(p, class_ref(p, m, "MeshAsset")?, walk)? else {
                        continue;
                    };
                    let transforms = elements(p, m, "InstanceTransforms")?;
                    let count = field(p, m, "InstanceCount")?;
                    if count != Some(Value::UInt(transforms.len() as u64)) {
                        bail!(
                            "{}: a group member of {} instances has {} transforms",
                            p.name,
                            count.map_or("?".into(), |c| format!("{c:?}")),
                            transforms.len()
                        );
                    }
                    for t in transforms {
                        let t = transform(p, Some(t))?.then(&group);
                        walk.placed.push((mesh.clone(), t));
                    }
                }
            }
            "StaticModelEntityData" => {
                if let Some(mesh) = self.mesh(p, class_ref(p, o, "Mesh")?, walk)? {
                    let t = transform(p, field(p, o, "Transform")?)?.then(&at);
                    walk.placed.push((mesh, t));
                }
            }
            _ if BLUEPRINT_REFERENCES.contains(&ty.as_str()) => {
                let t = transform(p, field(p, o, "BlueprintTransform")?)?.then(&at);
                let r = class_ref(p, o, "Blueprint")?;
                self.placed_blueprint(p, r, t, walk, depth + 1)?;
            }
            _ if OBJECT_LISTS.contains(&ty.as_str()) || ty == "ObjectBlueprint" => {
                self.blueprint(p, index, at, walk, depth + 1)?;
            }
            // Bundles, which `sub_levels` follows.
            _ if SUB_LEVEL_LINKS.contains(&ty.as_str()) => {}
            _ => walk.skip(ty),
        }
        Ok(())
    }

    /// Places the blueprint `r` (from `p`) at `at`. An imported blueprint is walked
    /// once, in its own space, and its placements reused wherever it is placed.
    fn placed_blueprint(
        &mut self,
        p: &Partition,
        r: Ref,
        at: Transform,
        walk: &mut Walk,
        depth: usize,
    ) -> Result<()> {
        let import = match r {
            Ref::Null => return Ok(()),
            Ref::Internal(i) => return self.blueprint(p, i, at, walk, depth),
            Ref::Import(i) => p.imports[i],
        };
        let key = (import.partition, import.instance);
        if !self.blueprints.contains_key(&key) {
            if !self.open.insert(key) {
                walk.skip("(blueprint cycle)");
                return Ok(());
            }
            let mut inner = Walk::default();
            let result = self.walk_import(key, &mut inner, depth);
            self.open.remove(&key);
            result?;
            self.blueprints.insert(key, Rc::new(inner));
        }
        // Skips and misses count once per placement of the blueprint, like its meshes.
        let inner = Rc::clone(&self.blueprints[&key]);
        for (mesh, t) in &inner.placed {
            walk.placed.push((mesh.clone(), t.then(&at)));
        }
        for (what, n) in &inner.skipped {
            *walk.skipped.entry(what.clone()).or_default() += n;
        }
        walk.missing += inner.missing;
        Ok(())
    }

    fn walk_import(
        &mut self,
        (partition, instance): ([u8; 16], [u8; 16]),
        walk: &mut Walk,
        depth: usize,
    ) -> Result<()> {
        let Some(loaded) = self.load(&partition)? else {
            walk.missing += 1;
            return Ok(());
        };
        let q = Partition::parse(&loaded.data)
            .map_err(|e| crate::Error(format!("{}: {e}", loaded.name)))?;
        match q.instances.iter().position(|x| x.guid == Some(instance)) {
            Some(i) => self.blueprint(&q, i, Transform::IDENTITY, walk, depth),
            None => {
                walk.missing += 1;
                Ok(())
            }
        }
    }

    /// The mesh `r` (from `p`) refers to; `None`, counted as missing, if the store
    /// lacks it.
    fn mesh(&mut self, p: &Partition, r: Ref, walk: &mut Walk) -> Result<Option<Mesh>> {
        let (partition, instance) = match r {
            Ref::Null => return Ok(None),
            Ref::Internal(i) => {
                let ty = p.type_name(p.instances[i].type_index);
                return Ok(Some(Mesh {
                    name: p.name.to_owned(),
                    kind: ty,
                    partition: p.header.guid,
                }));
            }
            Ref::Import(i) => (p.imports[i].partition, p.imports[i].instance),
        };
        if !self.meshes.contains_key(&(partition, instance)) {
            let mesh = match self.load(&partition)? {
                Some(loaded) => {
                    let q = Partition::parse(&loaded.data)
                        .map_err(|e| crate::Error(format!("{}: {e}", loaded.name)))?;
                    q.instances
                        .iter()
                        .find(|x| x.guid == Some(instance))
                        .map(|x| Mesh {
                            name: loaded.name.clone(),
                            kind: q.type_name(x.type_index),
                            partition,
                        })
                }
                None => None,
            };
            self.meshes.insert((partition, instance), mesh);
        }
        let mesh = self.meshes[&(partition, instance)].clone();
        if mesh.is_none() {
            walk.missing += 1;
        }
        Ok(mesh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    use crate::ebx::tests::{Builder, F, V};
    use crate::frames::FrameEncoder;
    use crate::install::{BundleEntry, Layer, Superbundle};
    use crate::sb::{Asset, Bundle};
    use crate::store::{INDEX, Index, encode, pack_name, reindex};
    use crate::{CasRef, Location};

    const ROOT: &str = "levels/mainlevel/mainlevel";
    const HOME: &str = "Win32/levels/mainlevel/mainlevel";

    /// Every type the walk reads, declared the way Heat's partitions do.
    fn types() -> Builder {
        let lt = F::Struct("LinearTransform");
        let placed = [("BlueprintTransform", lt), ("Blueprint", F::Ref)];
        let objects = [("Objects", F::Array("DataContainer[]"))];
        Builder::default()
            .ty(
                "Vec3",
                false,
                &[("x", F::F32), ("y", F::F32), ("z", F::F32)],
            )
            .ty(
                "LinearTransform",
                false,
                &[
                    ("right", F::Struct("Vec3")),
                    ("up", F::Struct("Vec3")),
                    ("forward", F::Struct("Vec3")),
                    ("trans", F::Struct("Vec3")),
                ],
            )
            .array("DataContainer[]", F::Ref)
            .array("LinearTransform[]", lt)
            .ty(
                "StaticModelGroupMemberData",
                false,
                &[
                    ("MeshAsset", F::Ref),
                    ("InstanceTransforms", F::Array("LinearTransform[]")),
                    ("InstanceCount", F::U32),
                ],
            )
            .array(
                "StaticModelGroupMemberData[]",
                F::Struct("StaticModelGroupMemberData"),
            )
            .ty("SubWorldData", true, &objects)
            .ty("LayerData", true, &objects)
            .ty("SpatialPrefabBlueprint", true, &objects)
            .ty("LayerReferenceObjectData", true, &placed)
            .ty("ObjectReferenceObjectData", true, &placed)
            .ty("ObjectBlueprint", true, &[("Object", F::Ref)])
            .ty(
                "StaticModelEntityData",
                true,
                &[("Transform", lt), ("Mesh", F::Ref)],
            )
            .ty(
                "StaticModelGroupEntityData",
                true,
                &[
                    ("Transform", lt),
                    ("MemberDatas", F::Array("StaticModelGroupMemberData[]")),
                ],
            )
            .ty("RigidMeshAsset", true, &[("Name", F::Str)])
            .ty("PbrSpotLightEntityData", true, &[])
            .ty("ZoneStreamerSubWorldRod", true, &[("BundleName", F::Str)])
    }

    fn v3(v: [f32; 3]) -> V {
        V::Struct(v.map(V::F32).to_vec())
    }

    fn lt(t: Transform) -> V {
        V::Struct(vec![v3(t.right), v3(t.up), v3(t.forward), v3(t.trans)])
    }

    fn at(x: f32, y: f32, z: f32) -> Transform {
        Transform {
            trans: [x, y, z],
            ..Transform::IDENTITY
        }
    }

    /// A quarter turn about up (+y), then a move to (x, y, z).
    fn turned(x: f32, y: f32, z: f32) -> Transform {
        Transform {
            right: [0.0, 0.0, -1.0],
            up: [0.0, 1.0, 0.0],
            forward: [1.0, 0.0, 0.0],
            trans: [x, y, z],
        }
    }

    fn import(i: u32) -> V {
        V::Ref(0x8000_0000 | i)
    }

    fn objects(refs: &[u32]) -> Vec<V> {
        vec![V::Array(refs.iter().map(|&r| V::Ref(r)).collect())]
    }

    /// A bundle's partitions: name and bytes.
    type Partitions<'a> = Vec<(&'a str, Vec<u8>)>;

    /// A store in a fresh temporary folder (`name` keeps tests apart) with one
    /// superbundle, the level's, holding `bundles` of (name, partitions), GUIDs listed.
    fn store(name: &str, bundles: &[(&str, Partitions)]) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nfsh-level-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut encoder = FrameEncoder::new(None).unwrap();
        let mut pack = Vec::new();
        let mut entries = Vec::new();
        for (bundle, partitions) in bundles {
            let mut ebx = Vec::new();
            for (name, data) in partitions {
                let frame = encoder.encode(data, true).unwrap();
                let location = Location {
                    cas: CasRef {
                        patch: false,
                        package: 0,
                        cas: 0,
                    },
                    offset: pack.len() as u32,
                    size: frame.len() as u32,
                };
                pack.extend(frame);
                ebx.push(Asset {
                    name: name.to_string(),
                    sha1: [0; 20],
                    original_size: data.len() as u32,
                    location,
                });
            }
            entries.push(BundleEntry {
                name: bundle.to_string(),
                layer: Layer::Data,
                bundle: Bundle {
                    ebx,
                    res: Vec::new(),
                    chunks: Vec::new(),
                    meta: Vec::new(),
                },
            });
        }
        std::fs::write(dir.join(pack_name(0)), &pack).unwrap();
        let index = Index {
            packs: vec![pack.len() as u64],
            dictionary: Vec::new(),
            superbundles: vec![Superbundle {
                name: HOME.into(),
                bundles: entries,
                chunks: Vec::new(),
            }],
            partitions: Vec::new(),
        };
        std::fs::write(dir.join(INDEX), encode(&index)).unwrap();
        reindex(&dir).unwrap();
        dir
    }

    fn mesh(name: &str, guid: u8) -> (&str, Vec<u8>) {
        let data = types()
            .instance("RigidMeshAsset", [guid + 1; 16], vec![V::Str("mesh")])
            .build(name, [guid; 16]);
        (name, data)
    }

    #[test]
    fn transforms_compose_child_into_parent() {
        let child = turned(1.0, 2.0, 3.0);
        let parent = turned(10.0, 0.0, 0.0);
        let both = child.then(&parent);
        // Two quarter turns: right goes to -x, forward to -z.
        assert_eq!(both.right, [-1.0, 0.0, 0.0]);
        assert_eq!(both.forward, [0.0, 0.0, -1.0]);
        // (1, 2, 3) turned: x·right + y·up + z·forward = (3, 2, -1), then moved.
        assert_eq!(both.trans, [13.0, 2.0, -1.0]);
        assert_eq!(child.then(&Transform::IDENTITY), child);
        assert_eq!(Transform::IDENTITY.then(&parent), parent);
    }

    #[test]
    fn bundle_names_compare_without_case_or_platform() {
        assert_eq!(
            bundle_key("Win32/Levels/MainLevel/WorldArt"),
            "levels/mainlevel/worldart"
        );
        assert_eq!(
            bundle_key("levels/mainlevel/worldart"),
            "levels/mainlevel/worldart"
        );
    }

    /// A zone with a layer placing one blueprint twice (turned, and moved), a mesh
    /// group of two instances, a light, a rod, and two references the store can't
    /// resolve.
    #[test]
    fn a_zone_places_its_groups_and_blueprints_in_world_space() {
        let zone = types()
            .import([0x20; 16], [0x21; 16])
            .import([0x40; 16], [0x41; 16])
            .instance("SubWorldData", [0x11; 16], objects(&[2, 3, 4, 5]))
            .instance(
                "LayerReferenceObjectData",
                [0x12; 16],
                vec![lt(at(0.0, 10.0, 0.0)), import(0)],
            )
            .instance(
                "StaticModelGroupEntityData",
                [0x13; 16],
                vec![
                    lt(at(1000.0, 0.0, 0.0)),
                    V::Array(vec![V::Struct(vec![
                        import(1),
                        V::Array(vec![lt(at(1.0, 0.0, 0.0)), lt(at(2.0, 0.0, 0.0))]),
                        V::U32(2),
                    ])]),
                ],
            )
            .instance("PbrSpotLightEntityData", [0x14; 16], vec![])
            .instance(
                "ZoneStreamerSubWorldRod",
                [0x15; 16],
                vec![V::Str("levels/test/zone_1_midday")],
            )
            .build("levels/test/zone_1", [0x10; 16]);
        let layer = types()
            .import([0x30; 16], [0x31; 16])
            .import([0x99; 16], [0x99; 16]) // no such partition
            .import([0x30; 16], [0x77; 16]) // no such instance in it
            .instance("LayerData", [0x21; 16], objects(&[2, 3, 4, 5]))
            .instance(
                "ObjectReferenceObjectData",
                [0x22; 16],
                vec![lt(turned(5.0, 0.0, 0.0)), import(0)],
            )
            .instance(
                "ObjectReferenceObjectData",
                [0x23; 16],
                vec![lt(at(0.0, 0.0, 7.0)), import(0)],
            )
            .instance(
                "ObjectReferenceObjectData",
                [0x24; 16],
                vec![lt(Transform::IDENTITY), import(1)],
            )
            .instance(
                "ObjectReferenceObjectData",
                [0x25; 16],
                vec![lt(Transform::IDENTITY), import(2)],
            )
            .build("levels/test/zone_1_layer", [0x20; 16]);
        let barrel = types()
            .import([0x50; 16], [0x51; 16])
            .instance("ObjectBlueprint", [0x31; 16], vec![V::Ref(2)])
            .instance(
                "StaticModelEntityData",
                [0x32; 16],
                vec![lt(at(0.0, 1.0, 0.0)), import(0)],
            )
            .build("props/barrel", [0x30; 16]);
        let dir = store(
            "zone",
            &[
                (
                    "win32/levels/test/zone_1",
                    vec![
                        ("levels/test/zone_1", zone),
                        ("levels/test/zone_1_layer", layer),
                    ],
                ),
                (
                    "win32/props",
                    vec![
                        ("props/barrel", barrel),
                        mesh("meshes/a", 0x40),
                        mesh("meshes/b", 0x50),
                    ],
                ),
            ],
        );
        let store = Store::open(&dir).unwrap();
        let mut level = Level::new(&store, HOME).unwrap();
        let zone = level.zone("Levels/Test/Zone_1").unwrap();

        let names: Vec<&str> = zone.meshes.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["meshes/b", "meshes/a"]);
        assert!(zone.meshes.iter().all(|m| m.kind == "RigidMeshAsset"));
        let placed: Vec<(usize, Transform)> = zone
            .placements
            .iter()
            .map(|p| (p.mesh, p.transform))
            .collect();
        assert_eq!(
            placed,
            [
                // The barrel's mesh, 1 up in the blueprint, under each reference
                // and the layer's 10 up.
                (0, turned(5.0, 11.0, 0.0)),
                (0, at(0.0, 11.0, 7.0)),
                (1, at(1001.0, 0.0, 0.0)),
                (1, at(1002.0, 0.0, 0.0)),
            ]
        );
        assert_eq!(zone.missing, 2);
        // The rod is a bundle, followed by sub_levels, so not listed.
        assert_eq!(
            zone.skipped,
            BTreeMap::from([("PbrSpotLightEntityData".to_owned(), 1)])
        );
        assert!(level.zone("levels/test/zone_2").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A prefab that places itself, and an object list that lists itself: each is cut
    /// off and reported, never followed forever.
    #[test]
    fn cycles_are_cut_off() {
        let zone = types()
            .import([0x70; 16], [0x71; 16])
            .instance("SubWorldData", [0x61; 16], objects(&[2]))
            .instance(
                "ObjectReferenceObjectData",
                [0x62; 16],
                vec![lt(Transform::IDENTITY), import(0)],
            )
            .build("levels/test/loop", [0x60; 16]);
        let prefab = types()
            .import([0x70; 16], [0x71; 16])
            .instance("SpatialPrefabBlueprint", [0x71; 16], objects(&[2]))
            .instance(
                "ObjectReferenceObjectData",
                [0x72; 16],
                vec![lt(Transform::IDENTITY), import(0)],
            )
            .build("props/loop", [0x70; 16]);
        let itself = types()
            .instance("SubWorldData", [0x91; 16], objects(&[1]))
            .build("levels/test/itself", [0x90; 16]);
        let dir = store(
            "cycle",
            &[
                (
                    "win32/levels/test/loop",
                    vec![("levels/test/loop", zone), ("props/loop", prefab)],
                ),
                (
                    "win32/levels/test/itself",
                    vec![("levels/test/itself", itself)],
                ),
            ],
        );
        let store = Store::open(&dir).unwrap();
        let mut level = Level::new(&store, HOME).unwrap();
        let mut skipped = |name| {
            let zone = level.zone(name).unwrap();
            assert!(zone.placements.is_empty());
            zone.skipped.into_iter().collect::<Vec<_>>()
        };
        assert_eq!(
            skipped("levels/test/loop"),
            [("(blueprint cycle)".to_owned(), 1)]
        );
        assert_eq!(
            skipped("levels/test/itself"),
            [("(nested too deep)".to_owned(), 1)]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn sub_levels_follow_bundle_names_breadth_first() {
        let rod = |name| V::Str(name);
        let root = types()
            .instance("SubWorldData", [1; 16], objects(&[]))
            .instance(
                "ZoneStreamerSubWorldRod",
                [2; 16],
                vec![rod("Levels/Test/Zone_1")],
            )
            .instance(
                "ZoneStreamerSubWorldRod",
                [3; 16],
                vec![rod("levels/test/gone")],
            )
            .build("levels/test/root", [0x80; 16]);
        let zone = types()
            .instance(
                "ZoneStreamerSubWorldRod",
                [4; 16],
                vec![rod("levels/test/zone_1_midday")],
            )
            .instance(
                "ZoneStreamerSubWorldRod",
                [5; 16],
                vec![rod("levels/test/root")],
            )
            .build("levels/test/zone_1", [0x81; 16]);
        let dir = store(
            "sub",
            &[
                ("Win32/Levels/Test/Root", vec![("levels/test/root", root)]),
                (
                    "win32/levels/test/zone_1",
                    vec![("levels/test/zone_1", zone)],
                ),
            ],
        );
        let store = Store::open(&dir).unwrap();
        let mut level = Level::new(&store, HOME).unwrap();
        let all = level.sub_levels("levels/test/root").unwrap();
        let got: Vec<(&str, usize, &str, bool)> = all
            .iter()
            .map(|s| (s.name.as_str(), s.depth, s.from.as_str(), s.found))
            .collect();
        assert_eq!(
            got,
            [
                ("levels/test/root", 0, "", true),
                ("levels/test/zone_1", 1, "levels/test/root", true),
                ("levels/test/gone", 1, "levels/test/root", false),
                ("levels/test/zone_1_midday", 2, "levels/test/zone_1", false),
            ]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Walks Palm City in the store `NFSH_STORE` points at (local-ci sets it on the
    /// Windows side): every zone the level links to, and everything each one places.
    #[test]
    fn walks_every_zone_of_palm_city() {
        let Some(dir) = std::env::var_os("NFSH_STORE") else {
            eprintln!("NFSH_STORE not set; skipped");
            return;
        };
        let store = Store::open(Path::new(&dir)).unwrap();
        let mut level = Level::new(&store, HOME).unwrap();
        let all = level.sub_levels(ROOT).unwrap();
        let zones: Vec<&SubLevel> = all
            .iter()
            .filter(|s| {
                s.name
                    .strip_prefix("levels/mainlevel/worldart_zs_")
                    .is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()))
            })
            .collect();
        let (mut placements, mut meshes, mut missing) = (0, 0, 0);
        let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
        let mut skipped: BTreeMap<String, usize> = BTreeMap::new();
        for z in &zones {
            let zone = level.zone(&z.name).unwrap();
            placements += zone.placements.len();
            meshes += zone.meshes.len();
            missing += zone.missing;
            for m in &zone.meshes {
                *kinds.entry(m.kind.clone()).or_default() += 1;
            }
            for (what, n) in zone.skipped {
                *skipped.entry(what).or_default() += n;
            }
        }
        eprintln!("{placements} placements of {meshes} meshes, {missing} missing");
        eprintln!("meshes {kinds:?}");
        eprintln!("not followed {skipped:?}");
        assert_eq!(all.iter().filter(|s| s.found).count(), 2034);
        assert!(all.iter().all(|s| s.found));
        assert_eq!(zones.len(), 232);
        assert_eq!(missing, 0);
        // Counted on the whole install, 2026-10-31 (meshes once per zone).
        assert_eq!(placements, 259_999);
        assert_eq!(meshes, 27_753);
        let kinds: Vec<(&str, usize)> = kinds.iter().map(|(k, &n)| (k.as_str(), n)).collect();
        assert_eq!(
            kinds,
            [("CompositeMeshAsset", 6738), ("RigidMeshAsset", 21_015)]
        );
    }
}
