//! Adapter for authored .skate world geometry. Does not replace controllers.
use bevy::{
    asset::RenderAssetUsages,
    image::ImageSampler,
    prelude::*,
    render::render_resource::{Extent3d, TextureDimension, TextureFormat},
};
use skate_core::{
    math::Vector3,
    physics::{
        board_world::{
            BoardWorld, WorldTriangle,
            query_metadata::{Bounds, QueryMesh, QueryMetadata, QueryPool},
        },
        collision::TriangleFeature,
        contact::RetailContactMaterial,
        drive_frames::RetailAffineTransform,
    },
};
use skate_data::skate_map::SkateMap;
use std::collections::HashMap;

#[cfg(test)]
#[path = "retail_shadow_geometry.rs"]
pub(crate) mod shadow_geometry;

pub(crate) fn validate_runtime(map: &SkateMap) -> Result<(), String> {
    let _span = info_span!("validate_map").entered();
    let archive = retail_archive(map)?;
    if map.geometry.collision.is_empty() && archive.is_none() {
        return Err("SKATE map has no collision geometry".into());
    }
    for triangle in map.geometry.collision.iter().filter(|_| archive.is_none()) {
        if let Some(edges) = triangle.native_edges {
            decode_native_edges(edges)?;
        }
    }
    if !map.doors.is_empty() {
        return Err(format!(
            "Map '{}' contains {} hinged doors. This imported game has no door body/controller adapter yet; refusing to drop their geometry or turn them into static walls.",
            map.name,
            map.doors.len()
        ));
    }
    for extension in &map.extensions {
        let tag = String::from_utf8_lossy(&extension.tag);
        if extension.tag == *b"RWCM" {
            continue;
        }
        if extension.tag == *b"MOBJ" {
            skate_data::skate_map::validate_static_objects(map, extension)?;
            continue;
        }
        if extension.tag == *b"SKYB" && extension.schema == 1 {
            eprintln!(
                "SKATE LIMITATION: SKYB retail sky retained; using the map horizon until its shader adapter is available."
            );
            continue;
        }
        if extension.tag != *b"WMET" && extension.tag != *b"WCFG" && extension.tag != *b"BMAT" {
            return Err(format!(
                "SKATE extension {tag} schema {} is decoded but its runtime adapter is not implemented. Refusing to silently omit potentially required world geometry.",
                extension.schema
            ));
        }
        eprintln!(
            "SKATE LIMITATION: {tag} extension retained; its runtime behavior is not connected."
        );
    }
    if map.materials.iter().any(|m| m.retail_definition.is_some()) {
        eprintln!(
            "SKATE_RENDER: retail world material adapter enabled; unsupported families retain portable PBR rendering."
        );
    }
    if map.textures.iter().any(|t| t.width == 0) {
        return Err(
            "SKATE contains external texture placeholders; supply a package with embedded textures"
                .into(),
        );
    }
    if !map.routes.is_empty() {
        eprintln!(
            "SKATE LIMITATION: {} NPC routes parsed; supplied game has no NPC controller.",
            map.routes.len()
        );
    }
    if map.lights.iter().any(|l| l.kind == 2) {
        eprintln!(
            "SKATE LIMITATION: area-light records retained; Bevy adapter currently renders point and spot lights only."
        );
    }
    eprintln!(
        "SKATE LIMITATION: native frame-lighting/day-night controller is not connected; using package lighting and retail district sky where available."
    );
    eprintln!(
        "SKATE_MAP_LOADED name={:?} version={} render_triangles={} collision_triangles={} textures={} spawn={:?}",
        map.name,
        map.version,
        map.geometry.indices.len() / 3,
        map.geometry.collision.len(),
        map.textures.len(),
        map.spawn
    );
    Ok(())
}

/// TU3 ClusteredMesh::GetUnitVolumes (82AC8A68): fdivs then fsubs,
/// using the pi-squared word at 822F88D0. This is not acos/angle decoding.
/// Bit 7 denotes an unmatched compiler edge and is not a triangle flag.
fn decode_native_edges(edges: [u8; 3]) -> Result<(u32, [f32; 3]), String> {
    let mut flags = 1 | TriangleFeature::ONE_SIDED | TriangleFeature::USE_EDGE_COSINES;
    let mut cosines = [0.; 3];
    for (i, code) in edges.into_iter().enumerate() {
        let exponent = code & 0x1f;
        // The native signed 32-bit shift becomes negative at 28 and zero
        // above it. Reject those malformed codes instead of producing NaNs.
        if exponent >= 28 {
            return Err(format!(
                "Invalid SKATE native edge angle code {exponent} at corner {i}"
            ));
        }
        cosines[i] = 1.0 - f32::from_bits(0x411d_e9e7) / ((8_u32 << exponent) as f32);
        flags |= u32::from(code & 0x20) << i;
        flags |= u32::from(code & 0x40) << (i + 3);
    }
    Ok((flags, cosines))
}

fn retail_archive(map: &SkateMap) -> Result<Option<&[u8]>, String> {
    let mut archives = map.extensions.iter().filter(|e| e.tag == *b"RWCM");
    let Some(archive) = archives.next() else {
        return Ok(None);
    };
    if archive.schema != 1 || archives.next().is_some() {
        return Err("SKATE requires one RWCM extension with schema 1".into());
    }
    Ok(Some(&archive.payload))
}

fn retail_collision_world(
    archive: &[u8],
    material: RetailContactMaterial,
) -> Result<BoardWorld, String> {
    let mut triangles = Vec::new();
    let mut packed_surfaces = Vec::new();
    let mut meshes = Vec::new();
    let count = skate_data::retail_collision::visit_clusters(archive, |_, cluster| {
        // Preserve cluster order and partition further only for group filters.
        let mut cursor = 0;
        while cursor < cluster.len() {
            let group = cluster[cursor].group;
            let start = triangles.len();
            while cursor < cluster.len() && cluster[cursor].group == group {
                let source = cluster[cursor];
                let (flags, cosines) = match source.edges {
                    Some(edges) => {
                        let (mut flags, cosines) = decode_native_edges(edges)?;
                        flags &= !TriangleFeature::ONE_SIDED;
                        if source.one_sided {
                            flags |= TriangleFeature::ONE_SIDED;
                        }
                        (flags, cosines)
                    }
                    // TriangleVolume ctor82AC7770 retains these defaults when
                    // the unit has no edge data; the mesh sidedness is not read.
                    None => (0x1e1, [-1.; 3]),
                };
                triangles.push(
                    WorldTriangle::from_vertices(
                        source.points.map(|p| Vector3::new(p[0], p[1], p[2])),
                        material,
                        u32::from(source.surface),
                        flags,
                        cosines,
                        0.,
                    )
                    .ok_or("Invalid RWCM collision triangle")?,
                );
                packed_surfaces.push(source.surface);
                cursor += 1;
            }
            let range = start..triangles.len();
            let bounds = Bounds::from_points(
                triangles[range.clone()]
                    .iter()
                    .flat_map(|t| t.triangle.vertices),
            )
            .ok_or("Invalid RWCM cluster bounds")?;
            meshes.push(QueryMesh {
                geometry: 0, rejection_flags: 0,
                triangle_range: range,
                local_to_world: RetailAffineTransform::IDENTITY,
                world_to_local: RetailAffineTransform::IDENTITY,
                local_bounds: bounds,
                matching_group: i32::from(group),
                pool: QueryPool::Ground,
            });
        }
        Ok(())
    })?;
    eprintln!(
        "SKATE_RWCM_READY triangles={count} query_clusters={} source=embedded",
        meshes.len()
    );
    BoardWorld::with_query_metadata(
        triangles,
        QueryMetadata {
            packed_surfaces,
            meshes,
            static_edges: vec![],
            island_flags: 0,
        },
    )
    .map_err(str::to_owned)
}

pub(crate) fn collision_world(
    map: &SkateMap,
    material: RetailContactMaterial,
) -> Result<BoardWorld, String> {
    if let Some(archive) = retail_archive(map)? {
        return retail_collision_world(archive, material);
    }
    // Match the reference RW mesh compiler's 1 mm vertex welding and reversed
    // edge pairing. Triangle diagonals are adjacency, never authored ledges.
    let mut welded = HashMap::<[i64; 3], usize>::new();
    let mut positions = Vec::<Vec3>::new();
    let mut vertices = Vec::new();
    let mut normals = Vec::new();
    for tri in &map.geometry.collision {
        let ids = tri.points.map(|p| {
            let inverse = 1.0 / f64::from(0.001_f32);
            let key = p.map(|v| (f64::from(v) * inverse).round() as i64);
            *welded.entry(key).or_insert_with(|| {
                let id = positions.len();
                positions.push(Vec3::from_array(p));
                id
            })
        });
        let [a, b, c] = tri.points.map(Vec3::from_array);
        let normal = (b - a)
            .cross(c - a)
            .try_normalize()
            .ok_or("Invalid SKATE collision triangle normal")?;
        vertices.push(ids);
        normals.push(normal);
    }
    let mut cosines = vec![[1.; 3]; vertices.len()];
    let mut flags =
        vec![TriangleFeature::ONE_SIDED | TriangleFeature::USE_EDGE_COSINES | 0xe0; vertices.len()];
    // Fully native maps need no reconstructed adjacency. Mixed maps still
    // include every face when finding neighbors for their authored geometry.
    if map
        .geometry
        .collision
        .iter()
        .any(|t| t.native_edges.is_none())
    {
        let mut open = HashMap::<(usize, usize), (usize, usize)>::new();
        for (i, ids) in vertices.iter().enumerate() {
            for edge in 0..3 {
                let (a, b) = (ids[edge], ids[(edge + 1) % 3]);
                if let Some((other, oe)) = open.remove(&(b, a)) {
                    let cosine = normals[i].dot(normals[other]).clamp(-1., 1.);
                    let orientation =
                        (positions[b] - positions[a]).dot(normals[i].cross(normals[other]));
                    // ExtendedEdgeCosine / MakeEdgeCode in rw_collision_mesh.cpp:
                    // orientation >= -1e-6 is convex; flat edges have no convex bit.
                    for (ti, e) in [(i, edge), (other, oe)] {
                        cosines[ti][e] = cosine;
                        if orientation <= -1.0e-6 || cosine >= 1. {
                            flags[ti] &= !(0x20 << e);
                        }
                    }
                } else {
                    open.entry((a, b)).or_insert((i, edge));
                }
            }
        }
        let mut adjacent = vec![Vec::new(); positions.len()];
        for (i, ids) in vertices.iter().enumerate() {
            for &v in ids {
                adjacent[v].push(i);
            }
        }
        for (v, faces) in adjacent.iter().enumerate() {
            let reference = normals[faces[0]];
            if faces
                .iter()
                .all(|&i| (reference.dot(normals[i]) - 1.).abs() <= 0.01)
            {
                for &i in faces {
                    for corner in 0..3 {
                        if vertices[i][corner] == v {
                            flags[i] |= 0x200 << corner;
                        }
                    }
                }
            }
        }
    }
    let mut triangles = Vec::with_capacity(vertices.len());
    let mut packed_surfaces = Vec::with_capacity(vertices.len());
    for (i, source) in map.geometry.collision.iter().enumerate() {
        if let Some(edges) = source.native_edges {
            (flags[i], cosines[i]) = decode_native_edges(edges)?;
        }
        let m = &map.materials[source.material as usize - 1];
        // Exact EncodeRwSurfaceId mapping from the reference native adapter.
        packed_surfaces.push((m.audio | (m.physics << 7) | (m.pattern << 12)) as u16);
        let points = vertices[i].map(|id| {
            let p = positions[id];
            Vector3::new(p.x, p.y, p.z)
        });
        // Keep the supplied game's original static-world contact combine values.
        // The native map bridge supplies packed surfaces, not a guessed split of
        // the package's single friction scalar into static/dynamic coefficients.
        triangles.push(
            WorldTriangle::from_vertices(
                points,
                material,
                source.surface,
                flags[i],
                cosines[i],
                0.,
            )
            .ok_or("Invalid SKATE collision volume")?,
        );
    }
    // Portable maps have no native cluster hierarchy. Bound contiguous ranges
    // once at load time so the existing BVH can reject distant geometry. Keep
    // triangle order and mesh identity/filter values: contact tie-breaking,
    // packed surfaces and adjacency must not change with this acceleration.
    let mut meshes = Vec::new();
    for start in (0..triangles.len()).step_by(64) {
        let end = (start + 64).min(triangles.len());
        let bounds = Bounds::from_points(triangles[start..end].iter().flat_map(|t| t.triangle.vertices))
            .ok_or("SKATE collision bounds empty")?;
        meshes.push(QueryMesh {
            geometry: 0, rejection_flags: 0,
            triangle_range: start..end,
            local_to_world: RetailAffineTransform::IDENTITY,
            world_to_local: RetailAffineTransform::IDENTITY,
            local_bounds: bounds,
            matching_group: -1,
            pool: QueryPool::Ground,
        });
    }
    let metadata = QueryMetadata {
        packed_surfaces,
        meshes,
        static_edges: vec![],
        island_flags: 0,
    };
    BoardWorld::with_query_metadata(triangles, metadata).map_err(str::to_owned)
}

/// Reuse identical render resources without changing authored triangles.
fn render_texture_ids(textures: &[skate_data::skate_map::Texture]) -> Vec<u32> {
    use std::hash::{Hash, Hasher};
    let mut buckets: HashMap<(u32, u32, u32, u64), Vec<usize>> = HashMap::new();
    let mut ids = vec![0];
    for (i, t) in textures.iter().enumerate() {
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        t.rgba.hash(&mut hash);
        let bucket = buckets
            .entry((t.width, t.height, t.color_space, hash.finish()))
            .or_default();
        let same = bucket.iter().copied().find(|&j| textures[j].rgba == t.rgba);
        let canonical = same.unwrap_or_else(|| {
            bucket.push(i);
            i
        });
        ids.push(canonical as u32 + 1);
    }
    ids
}

fn render_material_ids(
    materials: &[skate_data::skate_map::Material],
    texture_ids: &[u32],
) -> Vec<usize> {
    material_ids(materials, texture_ids, true)
}

fn material_ids(
    materials: &[skate_data::skate_map::Material],
    texture_ids: &[u32],
    include_lightmap: bool,
) -> Vec<usize> {
    let mut unique = HashMap::new();
    materials
        .iter()
        .enumerate()
        .map(|(i, m)| {
            // Only fields consumed by this renderer belong in the identity. Source
            // names and collision surface metadata do not change a PBR material.
            let textures = m.textures.map(|id| texture_ids[id as usize]);
            let key = [
                textures[0],
                if include_lightmap { textures[1] } else { 0 },
                textures[2],
                textures[3],
                textures[4],
                m.color[0].to_bits(),
                m.color[1].to_bits(),
                m.color[2].to_bits(),
                m.roughness.to_bits(),
                m.emissive.to_bits(),
                m.indirect_strength.to_bits(),
                m.alpha_mode,
                m.alpha_cutoff.to_bits(),
            ];
            let retail = m.retail_definition.as_deref().and_then(crate::retail_render::Definition::parse);
            *unique.entry((key, retail)).or_insert(i)
        })
        .collect()
}

fn render_groups(
    geometry: &skate_data::skate_map::Geometry,
    material_ids: &[usize],
) -> Vec<(usize, Vec<u32>)> {
    let mut lookup = HashMap::new();
    let mut groups: Vec<(usize, Vec<u32>)> = Vec::new();
    for tri in geometry.indices.chunks_exact(3) {
        let material = material_ids[geometry.vertices[tri[0] as usize].material as usize - 1];
        let group = *lookup.entry(material).or_insert_with(|| {
            let index = groups.len();
            groups.push((material, Vec::new()));
            index
        });
        groups[group].1.extend_from_slice(tri);
    }
    groups
}

struct RenderGroup {
    material: usize,
    indices: Vec<u32>,
    retail: Option<crate::retail_render::RetailWorldMaterial>,
    /// 0: explicit tangent frames, 1: generated from the normal map, 2: none.
    tangent_mode: u8,
}

fn consolidated_groups(
    map: &SkateMap,
    texture_ids: &[u32],
    tuning: &crate::retail_render::MaterialTuning,
    texture: &mut impl FnMut(u32, u8) -> Option<Handle<Image>>,
) -> Vec<RenderGroup> {
    let ids = render_material_ids(&map.materials, texture_ids);
    let mut lookup = HashMap::new();
    let mut groups: Vec<RenderGroup> = Vec::new();
    for (material, indices) in render_groups(&map.geometry, &ids) {
        let m = &map.materials[material];
        let retail = m.retail_definition.as_deref()
            .and_then(crate::retail_render::Definition::parse)
            .filter(|d| d.supported(tuning)).map(|d| d.build(m, tuning, texture));
        // Tangent construction must not switch paths when batches are combined.
        let explicit = indices.iter().all(|&i| map.geometry.vertices[i as usize].tangent_frame.is_some());
        let tangent_mode = if explicit { 0 } else if m.textures[2] != 0 { 1 } else { 2 };
        // Tangent generation can average shared vertices across old boundaries.
        // Keep those batches separate rather than changing their shading.
        if let Some(key) = retail.as_ref().and_then(|m| m.batch_key()).filter(|_| tangent_mode != 1) {
            let destination = *lookup.entry((key, tangent_mode)).or_insert(groups.len());
            if destination < groups.len() {
                groups[destination].indices.extend(indices);
                continue;
            }
        }
        groups.push(RenderGroup { material, indices, retail, tangent_mode });
    }
    groups
}

#[cfg(test)]
mod performance_inventory {
    use super::*;
    #[test]
    #[ignore = "requires SKATE_MAP_INVENTORY, SKATE_ASSET_ROOT and SKATE_MAP_INVENTORY_OUT"]
    fn consolidated_batches() {
        let map = SkateMap::load(std::path::Path::new(&std::env::var_os("SKATE_MAP_INVENTORY").unwrap())).unwrap();
        let tuning = crate::retail_render::MaterialTuning::load(std::path::Path::new(&std::env::var_os("SKATE_ASSET_ROOT").unwrap()));
        let textures = render_texture_ids(&map.textures);
        let baseline = render_groups(&map.geometry, &render_material_ids(&map.materials, &textures));
        let mut texture = |id: u32, role: u8| {
            let id = textures[id as usize];
            if id == 0 || (role == 5 && map.textures[id as usize - 1].height != map.textures[id as usize - 1].width * 6) { return None; }
            Some(Handle::Uuid(bevy::asset::uuid::Uuid::from_u128((u128::from(id) << 8) | u128::from(role)), default()))
        };
        let groups = consolidated_groups(&map, &textures, &tuning, &mut texture);
        let mut before: Vec<[u32; 3]> = map.geometry.indices.chunks_exact(3).map(|t| t.try_into().unwrap()).collect();
        let mut after: Vec<[u32; 3]> = groups.iter().flat_map(|g| g.indices.chunks_exact(3).map(|t| t.try_into().unwrap())).collect();
        before.sort_unstable(); after.sort_unstable(); assert_eq!(before, after);
        let mut opaque = 0; let mut masked = 0; let mut blended = 0; let mut fallback = 0;
        for group in &groups {
            match group.retail.as_ref().map(|m| m.alpha) {
                Some(AlphaMode::Opaque) => opaque += 1,
                Some(AlphaMode::Mask(_)) => masked += 1,
                Some(_) => blended += 1,
                None => fallback += 1,
            }
        }
        let mut shadows = shadow_geometry::ShadowGeometry::default();
        let mut source_shadow_triangles: Vec<[u32; 3]> = Vec::new();
        let mut replaced_casters = 0;
        for group in &groups {
            if let Some(material) = group.retail.as_ref().filter(|m| shadow_geometry::eligible(m)) {
                shadows.add(&map.geometry, &group.indices, material.two_sided);
                source_shadow_triangles.extend(group.indices.chunks_exact(3).map(|t| [t[0], t[1], t[2]]));
                replaced_casters += 1;
            }
        }
        let mut batched_shadow_triangles: Vec<[u32; 3]> = shadows.batches.iter().flat_map(|(_, indices)| indices.chunks_exact(3).map(|t| t.try_into().unwrap())).collect();
        source_shadow_triangles.sort_unstable(); batched_shadow_triangles.sort_unstable();
        assert_eq!(source_shadow_triangles, batched_shadow_triangles);
        let proxy_vertices: usize = shadows.batches.iter().map(|(_, indices)| indices.iter().collect::<std::collections::HashSet<_>>().len()).sum();
        let bounds=groups.iter().filter(|g|g.retail.is_some()).map(|g| {
            let mut min=Vec3::splat(f32::INFINITY);let mut max=-min;
            for &i in &g.indices {let p=Vec3::from_array(map.geometry.vertices[i as usize].position);min=min.min(p);max=max.max(p);}
            bevy::camera::primitives::Aabb::from_min_max(min,max)
        }).collect();
        let shadow_index=crate::retail_render::shadow_visibility::benchmark_bounds(bounds);
        let result = serde_json::json!({"shadow_index":shadow_index,"baseline_batches":baseline.len(),"consolidated_batches":groups.len(),"opaque":opaque,"masked":masked,"blended":blended,"fallback":fallback,"triangles":after.len(),"map_lights":map.lights.len(),"triangle_multiset_preserved":true,"replaced_opaque_shadow_casters":replaced_casters,"opaque_shadow_batches":shadows.batches.len(),"shadow_triangle_multiset_preserved":true,"opaque_shadow_triangles":source_shadow_triangles.len(),"proxy_vertices":proxy_vertices,"proxy_vertex_index_bytes":proxy_vertices*24+source_shadow_triangles.len()*12});
        std::fs::write(std::env::var_os("SKATE_MAP_INVENTORY_OUT").unwrap(),serde_json::to_vec_pretty(&result).unwrap()).unwrap();
    }
    /// Explicit read-only inventory, never initializes Bevy or a GPU. Asset path
    /// stays in the environment; output contains counts/bounds only.
    #[test]
    #[ignore = "requires SKATE_MAP_INVENTORY and SKATE_MAP_INVENTORY_OUT"]
    fn runtime_batches() {
        let map = SkateMap::load(std::path::Path::new(&std::env::var_os("SKATE_MAP_INVENTORY").unwrap())).unwrap();
        let textures = render_texture_ids(&map.textures);
        let materials = render_material_ids(&map.materials, &textures);
        let groups = render_groups(&map.geometry, &materials);
        let mut spans = [0usize;4];
        let mut triangles = [0usize;4];
        let mut cells = std::collections::HashSet::new();
        for (material, indices) in &groups {
            let mut min = Vec3::splat(f32::INFINITY);
            let mut max = Vec3::splat(f32::NEG_INFINITY);
            for &i in indices { let p=Vec3::from_array(map.geometry.vertices[i as usize].position); min=min.min(p); max=max.max(p); }
            let extent=(max.x-min.x).max(max.z-min.z);
            for (i,threshold) in [50.,100.,250.,500.].iter().enumerate() {
                if extent>*threshold {spans[i]+=1;triangles[i]+=indices.len()/3;}
            }
            for tri in indices.chunks_exact(3) {
                let p=tri.iter().map(|&i|Vec3::from_array(map.geometry.vertices[i as usize].position)/3.).sum::<Vec3>();
                cells.insert((*material,(p.x/32.).floor() as i32,(p.z/32.).floor() as i32));
            }
        }
        let result=serde_json::json!({"schema":1,"runtime_material_batches":groups.len(),"extent_threshold_metres":[50,100,250,500],"batches_over_threshold":spans,"triangles_in_batches_over_threshold":triangles,"centroid_32m_cells":cells.len(),"note":"Exact current material canonicalization and grouping; no frustum, occlusion or runtime frame measurement"});
        std::fs::write(std::env::var_os("SKATE_MAP_INVENTORY_OUT").unwrap(),serde_json::to_vec_pretty(&result).unwrap()).unwrap();
    }
}

/// Draw distance in metres for world batches (0 = unlimited, the desktop
/// default). Phones set it: University submits ~4,900 batches otherwise.
pub(crate) static DRAW_DISTANCE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Hide a batch once its bounding-box centre is farther than the draw
/// distance plus the batch radius, so wide batches never vanish while in reach.
fn draw_range(positions: impl Iterator<Item = [f32; 3]>) -> Option<bevy::camera::visibility::VisibilityRange> {
    let distance = DRAW_DISTANCE.load(std::sync::atomic::Ordering::Relaxed) as f32;
    if distance <= 0. {
        return None;
    }
    let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for position in positions {
        let position = Vec3::from_array(position);
        (min, max) = (min.min(position), max.max(position));
    }
    (min.x <= max.x).then(|| bevy::camera::visibility::VisibilityRange {
        use_aabb: true,
        ..bevy::camera::visibility::VisibilityRange::abrupt(0., distance + (max - min).length() * 0.5)
    })
}

/// Phones merge compatible retail batches into few objects (see
/// retail_merged.rs). The launcher can turn it off (SKATE_MERGE_LEVEL=0).
pub(crate) fn merge_level() -> bool {
    cfg!(target_os = "android") && std::env::var_os("SKATE_MERGE_LEVEL").is_none_or(|v| v != "0")
}

#[path = "skate_world_merge.rs"]
mod merge;

fn texture_image(map: &SkateMap, source: &skate_data::skate_map::Texture, role: u8) -> Image {
    let (format, bytes) = if role == 1 {
        let mut bytes = Vec::with_capacity(source.rgba.len() * 2);
        for pixel in source.rgba.chunks_exact(4) {
            for (i, &byte) in pixel.iter().enumerate() {
                let v = f32::from(byte) / 255.;
                let linear = if i == 3 { 1. } else { v * v * 4. };
                bytes.extend_from_slice(&half::f16::from_f32(linear).to_le_bytes());
            }
        }
        (TextureFormat::Rgba16Float, bytes)
    } else {
        let srgb =
            role == 0 && source.color_space == 1 && !(2..=3).contains(&map.version);
        (
            if srgb {
                TextureFormat::Rgba8UnormSrgb
            } else {
                TextureFormat::Rgba8Unorm
            },
            source.rgba.clone(),
        )
    };
    let cube = role == 5;
    let height = if cube { source.height / 6 } else { source.height };
    let layers = if cube { 6 } else { 1 };
    let mut image = Image::new(
        Extent3d {
            width: source.width,
            height,
            depth_or_array_layers: layers,
        },
        TextureDimension::D2,
        bytes,
        format,
        RenderAssetUsages::RENDER_WORLD,
    );
    // Phones also mip clamped retail textures (role 4: lightmaps
    // and clamped surfaces) and PBR colour/normal maps (0, 2):
    // sampling full-size textures for distant surfaces thrashes
    // the Mali texture cache. Role 1 is half-float, not RGBA8.
    let phone_mips = cfg!(target_os = "android") && matches!(role, 0 | 2 | 4);
    if role == 3 || role == 6 || cube || phone_mips {
        let (bytes, levels) = crate::retail_render::mip_chain(&source.rgba, source.width, height, layers);
        image.data = Some(bytes);
        image.texture_descriptor.mip_level_count = levels;
    }
    if cube {
        image.texture_view_descriptor = Some(bevy::render::render_resource::TextureViewDescriptor {
            dimension: Some(bevy::render::render_resource::TextureViewDimension::Cube), ..default()
        });
    }
    let mut sampler = bevy::image::ImageSamplerDescriptor::linear();
    if role != 1 && role != 4 && role != 6 && !cube {
        sampler.address_mode_u = bevy::image::ImageAddressMode::Repeat;
        sampler.address_mode_v = bevy::image::ImageAddressMode::Repeat;
    }
    image.sampler = ImageSampler::Descriptor(sampler);
    image
}

/// Marks textures held back (not yet assets) while the level is merged.
const STAGED_TEXTURE: u128 = 0x5ca7_e000_0000_0000_0000_0000_0000_0000;

fn staged_index(handle: &Handle<Image>) -> Option<usize> {
    match handle {
        Handle::Uuid(id, _) if id.as_u128() >> 96 == STAGED_TEXTURE >> 96 => Some((id.as_u128() & u128::from(u32::MAX)) as usize),
        _ => None,
    }
}

/// Decodes each texture/role once. When merging, textures are held back
/// so those that end up only inside texture arrays never become assets.
struct TextureBank<'a, S> {
    map: &'a SkateMap,
    ids: &'a [u32],
    images: &'a mut S,
    cache: HashMap<(u32, u8), Handle<Image>>,
    staged: Option<Vec<(Option<Image>, merge::TextureClass, usize)>>,
    resolved: HashMap<usize, Handle<Image>>,
}
impl<S: crate::map_render::AssetSink<Image>> TextureBank<'_, S> {
    fn get(&mut self, id: u32, role: u8) -> Option<Handle<Image>> {
        let id = self.ids[id as usize];
        if id == 0 {
            return None;
        }
        let source = &self.map.textures[id as usize - 1];
        if role == 5 && source.height != source.width * 6 {
            // Older .skate exports contain only face zero. Never treat it as a cube.
            return None;
        }
        if let Some(handle) = self.cache.get(&(id, role)) {
            return Some(handle.clone());
        }
        let image = texture_image(self.map, source, role);
        let handle = match &mut self.staged {
            Some(staged) => {
                let size = image.texture_descriptor.size;
                let class = merge::TextureClass {
                    role,
                    width: size.width,
                    height: size.height,
                    format: image.texture_descriptor.format,
                    mips: image.texture_descriptor.mip_level_count,
                };
                let bytes = image.data.as_ref().map_or(0, Vec::len);
                // Cubes are never array layers.
                let class = if role == 5 { merge::TextureClass { role: u8::MAX, ..class } } else { class };
                staged.push((Some(image), class, bytes));
                Handle::Uuid(bevy::asset::uuid::Uuid::from_u128(STAGED_TEXTURE | (staged.len() - 1) as u128), default())
            }
            None => self.images.add(image),
        };
        self.cache.insert((id, role), handle.clone());
        Some(handle)
    }
    /// Array candidates: staged, non-cube textures.
    fn class(&self, handle: &Handle<Image>) -> Option<(usize, merge::TextureClass, usize)> {
        let index = staged_index(handle)?;
        let (_, class, bytes) = self.staged.as_ref()?[index];
        (class.role != u8::MAX).then_some((index, class, bytes))
    }
    /// The asset for a handle from get(), adding a held-back texture now.
    fn resolve(&mut self, handle: Handle<Image>) -> Handle<Image> {
        let Some(index) = staged_index(&handle) else { return handle };
        if let Some(handle) = self.resolved.get(&index) {
            return handle.clone();
        }
        let image = self.staged.as_mut().unwrap()[index].0.take().expect("texture still held");
        let handle = self.images.add(image);
        self.resolved.insert(index, handle.clone());
        handle
    }
    fn texture(&mut self, id: u32, role: u8) -> Option<Handle<Image>> {
        self.get(id, role).map(|handle| self.resolve(handle))
    }
    /// One array texture from same-class staged textures, in layer order.
    fn array(&self, layers: &[usize]) -> Image {
        let staged = self.staged.as_ref().unwrap();
        let first = staged[layers[0]].0.as_ref().expect("texture still held");
        let size = first.texture_descriptor.size;
        let mut image = Image::new_uninit(
            Extent3d { depth_or_array_layers: layers.len() as u32, ..size },
            TextureDimension::D2,
            first.texture_descriptor.format,
            RenderAssetUsages::RENDER_WORLD,
        );
        image.texture_descriptor.mip_level_count = first.texture_descriptor.mip_level_count;
        image.sampler = first.sampler.clone();
        // Explicit: a one-layer array would otherwise get a plain 2D view.
        image.texture_view_descriptor = Some(bevy::render::render_resource::TextureViewDescriptor {
            dimension: Some(bevy::render::render_resource::TextureViewDimension::D2Array), ..default()
        });
        let layer = first.data.as_ref().map_or(0, Vec::len);
        let mut data = Vec::with_capacity(layer * layers.len());
        // Layer-major: every mip of layer 0, then layer 1, matching Bevy's default order.
        for &index in layers {
            data.extend_from_slice(staged[index].0.as_ref().and_then(|i| i.data.as_ref()).expect("texture still held"));
        }
        image.data = Some(data);
        image
    }
    fn release(&mut self, index: usize) {
        if !self.resolved.contains_key(&index) {
            self.staged.as_mut().unwrap()[index].0 = None;
        }
    }
}

/// A mesh from (vertex index, slot) corners, preserving authored normals and
/// both UV sets. Vertex colour carries retail decal coordinates and the merged
/// material slot. Returns whether explicit tangent frames were written.
fn batch_mesh(map: &SkateMap, corners: impl Iterator<Item = (u32, u32)>) -> (Mesh, bool, Option<bevy::camera::visibility::VisibilityRange>) {
    let mut remap = HashMap::new();
    let mut vertices = Vec::new();
    let local: Vec<u32> = corners
        .map(|(index, slot)| {
            *remap.entry((index, slot)).or_insert_with(|| {
                let id = vertices.len() as u32;
                vertices.push((&map.geometry.vertices[index as usize], slot as f32));
                id
            })
        })
        .collect();
    let mut mesh = Mesh::new(
        bevy::mesh::PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vertices.iter().map(|(v, _)| v.position).collect::<Vec<_>>(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_NORMAL,
        vertices.iter().map(|(v, _)| v.normal).collect::<Vec<_>>(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_UV_0,
        vertices.iter().map(|(v, _)| v.uv).collect::<Vec<_>>(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_UV_1,
        vertices.iter().map(|(v, _)| v.lightmap_uv).collect::<Vec<_>>(),
    )
    .with_inserted_attribute(
        Mesh::ATTRIBUTE_COLOR,
        vertices.iter().map(|(v, slot)| { let uv = v.decal_uv.unwrap_or(v.uv); [uv[0], uv[1], *slot, 1.] }).collect::<Vec<_>>(),
    )
    .with_inserted_indices(bevy::mesh::Indices::U32(local));
    let range = draw_range(vertices.iter().map(|(v, _)| v.position));
    let explicit = vertices.iter().all(|(v, _)| v.tangent_frame.is_some());
    if explicit {
        let tangents: Vec<[f32; 4]> = vertices
            .iter()
            .map(|(v, _)| {
                let frame = v
                    .tangent_frame
                    .unwrap()
                    .map(|b| (b as i8 as f32 / 127.).max(-1.));
                let binormal = Vec3::new(frame[0], frame[1], frame[2]);
                let tangent = binormal.cross(Vec3::from_array(v.normal)) * frame[3];
                [tangent.x, tangent.y, tangent.z, frame[3]]
            })
            .collect();
        mesh.insert_attribute(Mesh::ATTRIBUTE_TANGENT, tangents);
    }
    (mesh, explicit, range)
}

pub(crate) fn spawn(
    map: &SkateMap,
    commands: &mut crate::map_render::SceneCommands,
    meshes: &mut impl crate::map_render::AssetSink<Mesh>,
    materials: &mut impl crate::map_render::AssetSink<StandardMaterial>,
    retail_materials: &mut impl crate::map_render::AssetSink<crate::retail_render::RetailWorldMaterial>,
    images: &mut impl crate::map_render::AssetSink<Image>,
    tuning: &crate::retail_render::MaterialTuning,
    merged: Option<&mut crate::map_render::MergedAssets>,
) {
    // Texture roles have different transfer functions even when sharing a record.
    let _span = info_span!("prepare_map_geometry_and_textures").entered();
    let texture_ids = render_texture_ids(&map.textures);
    // Without level shadows (phone default) the batches are not casters at all,
    // so Bevy's per-light shadow visibility tests skip thousands of entities.
    let casts_shadows = crate::retail_character::world_shadows();
    let mut bank = TextureBank {
        map, ids: &texture_ids, images, cache: HashMap::new(),
        staged: merged.is_some().then(Vec::new), resolved: HashMap::new(),
    };
    // Lightmaps belong to mesh entities, not StandardMaterial. Distinct baked
    // lighting still needs separate geometry batches but can share a PBR material.
    let pbr_ids = self::material_ids(&map.materials, &texture_ids, false);
    let groups = consolidated_groups(map, &texture_ids, tuning, &mut |id, role| bank.get(id, role));
    eprintln!(
        "SKATE_RENDER_BATCHES count={} triangles={}",
        groups.len(),
        map.geometry.indices.len() / 3
    );
    let mut groups = groups;
    if let Some(merged) = merged {
        let plan = merge::plan(groups, |handle| bank.class(handle));
        // Textures still needed on their own by batches that stay separate.
        let keep: std::collections::HashSet<usize> = plan.rest.iter()
            .filter_map(|g| g.retail.as_ref())
            .flat_map(|m| [&m.diffuse, &m.lightmap, &m.normal, &m.detail, &m.macro_map, &m.decal, &m.specular, &m.environment])
            .filter_map(|h| h.as_ref().and_then(staged_index))
            .collect();
        let mut array_bytes = 0;
        let arrays: Vec<Handle<Image>> = plan.arrays.iter().map(|layers| {
            let image = bank.array(layers);
            array_bytes += image.data.as_ref().map_or(0, Vec::len);
            // Free each source as soon as its copy is in an array.
            for &index in layers {
                if !keep.contains(&index) {
                    bank.release(index);
                }
            }
            bank.images.add(image)
        }).collect();
        let mut objects = 0;
        let merged_batches: usize = plan.sets.iter().map(|s| s.members().len()).sum();
        for set in &plan.sets {
            let slots: Vec<crate::retail_render::MergedSlot> = set.members().iter().map(|m| m.slot.clone()).collect();
            let buffer = merged.buffers.reserve();
            merged.slots.push((buffer.clone(), slots));
            let array = |k: usize| set.arrays[k].map(|a| arrays[a].clone());
            let material = crate::map_render::AssetSink::add(&mut merged.materials, crate::retail_render::MergedWorldMaterial {
                slots: buffer,
                diffuse: array(0),
                lightmap: array(1),
                normal: array(2),
                detail: array(3),
                macro_map: array(4),
                decal: array(5),
                specular: array(6),
                environment: set.environment.clone().map(|h| bank.resolve(h)),
                shadow_state: set.shadow_state.clone(),
                alpha: set.alpha,
                two_sided: set.two_sided,
            });
            for cell in merge::cells(map, set) {
                let (mesh, _, range) = batch_mesh(map, cell.into_iter());
                let mut entity = commands.spawn((Name::new("Merged level"), Mesh3d(meshes.add(mesh)), MeshMaterial3d(material.clone()), Transform::default()));
                if let Some(range) = range {
                    entity.insert(range);
                }
                if !casts_shadows {
                    entity.insert(bevy::light::NotShadowCaster);
                }
                objects += 1;
            }
        }
        eprintln!(
            "SKATE_MERGED_LEVEL batches={merged_batches} sets={} objects={objects} arrays={} array_mb={} separate={}",
            plan.sets.len(), plan.arrays.len(), array_bytes >> 20, plan.rest.len()
        );
        groups = plan.rest;
        for group in &mut groups {
            if let Some(material) = group.retail.as_mut() {
                for slot in material.images_mut() {
                    if let Some(handle) = slot.take() {
                        *slot = Some(bank.resolve(handle));
                    }
                }
            }
        }
    }
    let mut material_handles: Vec<Option<Handle<StandardMaterial>>> =
        vec![None; map.materials.len()];
    for RenderGroup { material: material_index, indices, retail, .. } in groups {
        let m = &map.materials[material_index];
        let (mut mesh, explicit, range) = batch_mesh(map, indices.into_iter().map(|i| (i, 0)));
        if !explicit && m.textures[2] != 0 {
            if let Err(error) = mesh.generate_tangents() {
                warn!("SKATE material {} tangent generation: {error}", m.name);
            }
        }
        if let Some(material) = retail {
            let material = retail_materials.add(material);
            let mut entity = commands.spawn((Name::new(m.name.clone()), Mesh3d(meshes.add(mesh)), MeshMaterial3d(material), Transform::default()));
            if let Some(range) = range {
                entity.insert(range);
            }
            if !casts_shadows {
                entity.insert(bevy::light::NotShadowCaster);
            }
            continue;
        }
        // Vertex colours above carry retail decal coordinates, never PBR tint.
        mesh.remove_attribute(Mesh::ATTRIBUTE_COLOR);
        let material = match &material_handles[pbr_ids[material_index]] {
            Some(handle) => handle.clone(),
            None => {
                let orm = bank.texture(m.textures[3], 2);
                let handle = materials.add(StandardMaterial {
                    base_color: Color::linear_rgb(m.color[0], m.color[1], m.color[2]),
                    base_color_texture: bank.texture(m.textures[0], 0),
                    normal_map_texture: bank.texture(m.textures[2], 2),
                    metallic_roughness_texture: orm.clone(),
                    occlusion_texture: orm,
                    metallic: if m.textures[3] != 0 { 1. } else { 0. },
                    perceptual_roughness: m.roughness,
                    emissive: LinearRgba::rgb(
                        m.color[0] * m.emissive,
                        m.color[1] * m.emissive,
                        m.color[2] * m.emissive,
                    ),
                    emissive_texture: bank.texture(m.textures[4], 0),
                    alpha_mode: match m.alpha_mode {
                        1 => AlphaMode::Mask(m.alpha_cutoff),
                        2 => AlphaMode::Blend,
                        _ => AlphaMode::Opaque,
                    },
                    lightmap_exposure: m.indirect_strength,
                    ..default()
                });
                material_handles[pbr_ids[material_index]] = Some(handle.clone());
                handle
            }
        };
        let mut entity = commands.spawn((
            Name::new(m.name.clone()),
            Mesh3d(meshes.add(mesh)),
            MeshMaterial3d(material),
            Transform::default(),
        ));
        if let Some(range) = range {
            entity.insert(range);
        }
        if !casts_shadows {
            entity.insert(bevy::light::NotShadowCaster);
        }
        if let Some(image) = bank.texture(m.textures[1], 1) {
            entity.insert(bevy::pbr::Lightmap {
                image,
                uv_rect: Rect::new(0., 0., 1., 1.),
                bicubic_sampling: false,
            });
        }
    }
    for light in &map.lights {
        let color = Color::linear_rgb(light.color[0], light.color[1], light.color[2]);
        let transform = Transform::from_translation(Vec3::from_array(light.position));
        match light.kind {
            0 => {
                commands.spawn((
                    Name::new(light.name.clone()),
                    PointLight {
                        color,
                        intensity: light.intensity,
                        range: light.range,
                        radius: light.radius,
                        ..default()
                    },
                    transform,
                ));
            }
            1 => {
                commands.spawn((
                    Name::new(light.name.clone()),
                    SpotLight {
                        color,
                        intensity: light.intensity,
                        range: light.range,
                        radius: light.radius,
                        inner_angle: light.inner_cos.acos(),
                        outer_angle: light.outer_cos.acos(),
                        ..default()
                    },
                    transform.looking_to(Vec3::from_array(light.direction), Vec3::Y),
                ));
            }
            _ => {}
        }
    }
    commands.insert_resource(ClearColor(Color::linear_rgb(
        map.environment[3],
        map.environment[4],
        map.environment[5],
    )));
    info!(
        "SKATE_WORLD_READY name={:?} render_triangles={} collision_triangles={}",
        map.name,
        map.geometry.indices.len() / 3,
        map.geometry.collision.len()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    fn compare_cluster_queries(world: &BoardWorld) -> (usize, usize) {
        let mut metadata = world.query_metadata().unwrap().clone();
        let mut single = metadata.meshes[0].clone();
        single.triangle_range = 0..world.triangles().len();
        single.local_bounds = Bounds::from_points(world.triangles().iter().flat_map(|t| t.triangle.vertices)).unwrap();
        metadata.meshes = vec![single];
        let baseline = BoardWorld::with_query_metadata(world.triangles().to_vec(), metadata).unwrap();
        let mut visited = [0usize; 2];
        let mut times = [std::time::Duration::ZERO; 2];
        for triangle in world.triangles().iter().step_by((world.triangles().len() / 256).max(1)) {
            let p = triangle.triangle.vertices[0];
            let start = Vector3::new(p.x, p.y + 1., p.z);
            let end = Vector3::new(p.x, p.y - 1., p.z);
            let mut candidates = Vec::new();
            for (i, scene) in [&baseline, world].into_iter().enumerate() {
                let bounds = scene.line_candidate_bounds(start, end, 0.1);
                visited[i] += scene.candidate_ranges(bounds).iter().map(|r| r.len()).sum::<usize>();
                let now = std::time::Instant::now();
                candidates.push(scene.line_candidates(start, end, 0.1).map(|(id, _)| id).collect::<Vec<_>>());
                times[i] += now.elapsed();
            }
            assert_eq!(candidates[0], candidates[1], "candidate identity/order changed");
        }
        eprintln!("CUSTOM_QUERY_BENCH baseline_triangles={} clustered_triangles={} baseline_ms={} clustered_ms={} clusters={}",
            visited[0], visited[1], times[0].as_secs_f64()*1000., times[1].as_secs_f64()*1000.,
            world.query_metadata().unwrap().meshes.len());
        (visited[0], visited[1])
    }

    #[test]
    fn portable_collision_clusters_preserve_queries_and_cull_distant_geometry() {
        let mut map = demo();
        let source = map.geometry.collision.remove(0);
        map.geometry.collision.clear();
        for i in 0..256 {
            map.geometry.collision.push(skate_data::skate_map::Collision {
                points: source.points.map(|p| [p[0] + i as f32 * 100., p[1], p[2]]),
                surface: source.surface, material: source.material, native_edges: source.native_edges,
            });
        }
        let world = collision_world(&map, material()).unwrap();
        assert_eq!(world.query_metadata().unwrap().meshes.len(), 4);
        let (before, after) = compare_cluster_queries(&world);
        assert!(after < before / 2);
    }

    #[test]
    #[ignore = "requires a supplied map; static collision-query benchmark only"]
    fn supplied_custom_map_collision_query_benchmark() {
        let path = std::env::var("SKATE_TEST_CUSTOM_MAP").unwrap();
        let map = SkateMap::load(std::path::Path::new(&path)).unwrap();
        let world = collision_world(&map, material()).unwrap();
        let (before, after) = compare_cluster_queries(&world);
        assert!(after < before);
    }
    fn demo() -> SkateMap {
        SkateMap::parse(include_bytes!("../../../maps/format-demo.skate")).unwrap()
    }
    fn retail_demo() -> SkateMap {
        fn definition(unused: &str) -> Vec<u8> {
            fn text(bytes: &mut Vec<u8>, value: &str) { bytes.extend_from_slice(&(value.len() as u32).to_le_bytes()); bytes.extend_from_slice(value.as_bytes()); }
            let mut bytes = vec![0; 16];
            text(&mut bytes, "world.default");
            bytes.extend_from_slice(&1u32.to_le_bytes()); // supported family
            bytes.extend_from_slice(&0u32.to_le_bytes()); // one-sided
            bytes.extend_from_slice(&0u32.to_le_bytes()); // no explicit bindings
            bytes.extend_from_slice(&1u32.to_le_bytes());
            text(&mut bytes, "unused_source_property");
            bytes.extend_from_slice(&1u32.to_le_bytes()); text(&mut bytes, unused);
            text(&mut bytes, ""); bytes
        }
        let mut map = demo();
        map.materials[0].retail_definition = Some(definition("a"));
        map.materials[0].alpha_mode = 0;
        let mut second = demo().materials.remove(0);
        second.retail_definition = Some(definition("b"));
        second.alpha_mode = 0;
        second.roughness = 0.12345; // ignored by the retail shader
        map.materials.push(second);
        let original_vertices = map.geometry.vertices.len() as u32;
        let duplicates: Vec<_> = map.geometry.vertices.iter().map(|v| { let mut v = v.clone(); v.material = 2; v.position[0] += 3.; v }).collect();
        map.geometry.vertices.extend(duplicates);
        map.geometry.indices.extend(map.geometry.indices.clone().into_iter().map(|i| i + original_vertices));
        for v in &mut map.geometry.vertices { v.tangent_frame = Some([0, 127, 0, 127]); }
        map
    }

    #[test]
    fn effective_batches_ignore_unused_metadata_but_preserve_lighting_and_blend_groups() {
        let mut map = retail_demo();
        let textures = render_texture_ids(&map.textures);
        let tuning = crate::retail_render::MaterialTuning::default();
        let mut texture = |id: u32, role: u8| (id != 0).then(|| Handle::Uuid(bevy::asset::uuid::Uuid::from_u128((u128::from(textures[id as usize]) << 8) | u128::from(role)), default()));
        assert_eq!(render_groups(&map.geometry, &render_material_ids(&map.materials, &textures)).len(), 2);
        let groups = consolidated_groups(&map, &textures, &tuning, &mut texture);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].indices.len(), map.geometry.indices.len());
        let original_lightmap = map.materials[1].textures[1];
        map.materials[1].textures[1] = if original_lightmap == 0 { map.materials[0].textures[0] } else { 0 };
        assert_eq!(consolidated_groups(&map, &textures, &tuning, &mut texture).len(), 2);
        map.materials[1].textures[1] = original_lightmap;
        for m in &mut map.materials { m.alpha_mode = 2; }
        let groups = consolidated_groups(&map, &textures, &tuning, &mut texture);
        assert_eq!(groups.len(), 2);
        assert!(groups.iter().all(|g| !shadow_geometry::eligible(g.retail.as_ref().unwrap())));
    }

    #[test]
    fn retail_batches_are_owned_and_retired_with_the_map() {
        let map = retail_demo();
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        world.init_resource::<Assets<crate::retail_render::RetailWorldMaterial>>();
        world.init_resource::<Assets<crate::retail_render::RetailSkyMaterial>>();
        world.init_resource::<Assets<Image>>();
        let mut scene = crate::map_render::PreparedScene::new(&world);
        scene.prepare(Some(&map), std::path::Path::new("unused"));
        scene.publish(&mut world);
        assert_eq!(world.query_filtered::<Entity, With<bevy::light::NotShadowCaster>>().iter(&world).count(), 0);
        crate::map_render::MapAssets::retire(&mut world);
        assert_eq!(world.query_filtered::<Entity, With<crate::map_render::MapEntity>>().iter(&world).count(), 0);
        assert_eq!(world.resource::<Assets<Mesh>>().len(), 0);
        assert_eq!(world.resource::<Assets<StandardMaterial>>().len(), 0);
        assert_eq!(world.resource::<Assets<crate::retail_render::RetailWorldMaterial>>().len(), 0);
    }
    #[test]
    fn merged_level_keeps_every_triangle_and_retires_with_the_map() {
        use crate::retail_render::MergedWorldMaterial;
        use bevy::render::storage::ShaderStorageBuffer;
        let mut map = retail_demo();
        // A shading parameter makes the two retail materials distinct batches.
        let definition = map.materials[1].retail_definition.as_mut().unwrap();
        let name = b"unused_source_property";
        let at = definition.windows(name.len()).position(|w| w == name).unwrap();
        definition.splice(at - 4..at + name.len(), [&19u32.to_le_bytes()[..], b"macroOverlayOpacity"].concat());
        let value = definition.len() - 9;
        definition.splice(value..value + 5, [&3u32.to_le_bytes()[..], b"0.5"].concat());
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        world.init_resource::<Assets<crate::retail_render::RetailWorldMaterial>>();
        world.init_resource::<Assets<crate::retail_render::RetailSkyMaterial>>();
        world.init_resource::<Assets<MergedWorldMaterial>>();
        world.init_resource::<Assets<ShaderStorageBuffer>>();
        world.init_resource::<Assets<Image>>();
        let mut scene = crate::map_render::PreparedScene::new(&world);
        scene.force_merge(&world);
        scene.prepare(Some(&map), std::path::Path::new("unused"));
        scene.publish(&mut world);
        let objects: Vec<(Handle<Mesh>, Handle<MergedWorldMaterial>)> = world
            .query::<(&Mesh3d, &MeshMaterial3d<MergedWorldMaterial>)>().iter(&world)
            .map(|(mesh, material)| (mesh.0.clone(), material.0.clone())).collect();
        assert_eq!(objects.len(), 1);
        let mut triangles = 0;
        for (mesh, material) in &objects {
            let mesh = world.resource::<Assets<Mesh>>().get(mesh).unwrap();
            let material = world.resource::<Assets<MergedWorldMaterial>>().get(material).unwrap();
            let slots = world.resource::<Assets<ShaderStorageBuffer>>().get(&material.slots).unwrap();
            let slot_count = slots.data.as_ref().unwrap().len() / <crate::retail_render::MergedSlot as bevy::render::render_resource::ShaderType>::min_size().get() as usize;
            let Some(bevy::mesh::VertexAttributeValues::Float32x4(colors)) = mesh.attribute(Mesh::ATTRIBUTE_COLOR) else { panic!() };
            assert_eq!(slot_count, 2);
            assert!(colors.iter().all(|c| (c[2] as usize) < slot_count && c[2].fract() == 0.));
            assert!(colors.iter().any(|c| c[2] == 1.));
            triangles += mesh.indices().unwrap().len() / 3;
            let images = world.resource::<Assets<Image>>();
            for array in [&material.diffuse, &material.lightmap].into_iter().flatten() {
                let image = images.get(array).unwrap();
                assert_eq!(image.texture_view_descriptor.as_ref().unwrap().dimension,
                    Some(bevy::render::render_resource::TextureViewDimension::D2Array));
                let size = image.texture_descriptor.size;
                let mut layer = 0;
                for level in 0..image.texture_descriptor.mip_level_count {
                    layer += ((size.width >> level).max(1) * (size.height >> level).max(1) * 4) as usize;
                }
                assert_eq!(image.data.as_ref().unwrap().len(), layer * size.depth_or_array_layers as usize);
            }
        }
        let separate: usize = world.query::<(&Mesh3d, &MeshMaterial3d<crate::retail_render::RetailWorldMaterial>)>().iter(&world)
            .map(|(mesh, _)| world.resource::<Assets<Mesh>>().get(&mesh.0).unwrap().indices().unwrap().len() / 3).sum();
        assert_eq!(triangles + separate, map.geometry.indices.len() / 3);
        crate::map_render::MapAssets::retire(&mut world);
        assert_eq!(world.resource::<Assets<MergedWorldMaterial>>().len(), 0);
        assert_eq!(world.resource::<Assets<ShaderStorageBuffer>>().len(), 0);
        assert_eq!(world.resource::<Assets<Image>>().len(), 0);
        assert_eq!(world.resource::<Assets<Mesh>>().len(), 0);
    }
    #[test]
    fn render_batches_preserve_complete_triangles_and_materials() {
        let map = demo();
        let material_ids = render_material_ids(&map.materials, &render_texture_ids(&map.textures));
        let chunks = render_groups(&map.geometry, &material_ids);
        assert_eq!(chunks.len(), 1);
        let mut original: Vec<_> = map
            .geometry
            .indices
            .chunks_exact(3)
            .map(|t| t.to_vec())
            .collect();
        let mut partitioned = Vec::new();
        for (material, indices) in chunks {
            for tri in indices.chunks_exact(3) {
                assert_eq!(
                    material + 1,
                    map.geometry.vertices[tri[0] as usize].material as usize
                );
                partitioned.push(tri.to_vec());
            }
        }
        original.sort();
        partitioned.sort();
        assert_eq!(partitioned, original);
    }
    #[test]
    fn render_material_sharing_requires_identical_rendered_fields() {
        let mut map = demo();
        let mut copy = demo().materials.remove(0);
        copy.name = "another source object".into();
        copy.audio = 42;
        map.materials.push(copy);
        let texture_ids = render_texture_ids(&map.textures);
        assert_eq!(render_material_ids(&map.materials, &texture_ids), [0, 0]);
        map.materials[1].roughness = 0.1234;
        assert_eq!(render_material_ids(&map.materials, &texture_ids), [0, 1]);
        map.materials[1].roughness = map.materials[0].roughness;
        map.materials[1].textures[1] = 0;
        assert_eq!(render_material_ids(&map.materials, &texture_ids), [0, 1]);
        assert_eq!(material_ids(&map.materials, &texture_ids, false), [0, 0]);
        map.materials[1].indirect_strength += 1.;
        assert_eq!(material_ids(&map.materials, &texture_ids, false), [0, 1]);
    }
    #[test]
    fn texture_sharing_preserves_pixels_dimensions_and_color_space() {
        let texture = |name: &str, color_space, pixel| skate_data::skate_map::Texture {
            name: name.into(),
            width: 1,
            height: 1,
            color_space,
            rgba: vec![pixel, 0, 0, 255],
        };
        let textures = vec![
            texture("first", 1, 30),
            texture("duplicate", 1, 30),
            texture("linear", 0, 30),
            texture("different", 1, 31),
        ];
        assert_eq!(render_texture_ids(&textures), [0, 1, 1, 3, 4]);
    }
    fn material() -> RetailContactMaterial {
        RetailContactMaterial {
            static_friction: 0.,
            dynamic_friction: 0.,
            restitution: 1.,
        }
    }
    #[test]
    fn native_edges_override_generated_adjacency() {
        let mut map = demo();
        map.geometry.collision[0].native_edges = Some([0x20, 0x42, 0x9a]);
        validate_runtime(&map).unwrap();
        let world = collision_world(&map, material()).unwrap();
        let f = world.triangles()[0].triangle.feature;
        assert!(f.edge_convex(0));
        assert!(!f.edge_convex(1));
        assert!(!f.edge_convex(2));
        assert!(!f.vertex_disabled(0));
        assert!(f.vertex_disabled(1));
        assert!(!f.vertex_disabled(2));
        assert_eq!(f.edge_cosines[0].to_bits(), 0xbe6f4f38);
        assert_eq!(f.edge_cosines[1].to_bits(), 0x3f310b0c);
        assert_eq!(f.edge_cosines[2], 1.);
        assert!(decode_native_edges([31, 0, 0]).is_err());
        assert_eq!(decode_native_edges([0, 2, 26]).unwrap().1, f.edge_cosines);
    }
    #[test]
    fn embedded_archive_is_authoritative_and_preserves_cluster_metadata() {
        let mut map = demo();
        map.geometry.collision.clear();
        map.extensions.push(skate_data::skate_map::Extension {
            tag: *b"RWCM",
            schema: 1,
            payload: include_bytes!("../../skate-data/tests/fixtures/retail-collision.rwcmset")
                .to_vec(),
        });
        validate_runtime(&map).unwrap();
        let world = collision_world(&map, material()).unwrap();
        assert_eq!(world.triangles().len(), 3);
        let metadata = world.query_metadata().unwrap();
        assert_eq!(metadata.meshes.len(), 3);
        assert_eq!(metadata.meshes[1].matching_group, 0x1234);
        assert_eq!(metadata.packed_surfaces, vec![0x4321; 3]);
        assert_eq!(world.triangles()[1].triangle.vertices[0].x, 10.);
    }
    #[test]
    fn native_unit_without_edge_data_keeps_constructor_defaults() {
        let mut archive =
            include_bytes!("../../skate-data/tests/fixtures/retail-collision.rwcmset").to_vec();
        let name_len = u32::from_le_bytes(archive[12..16].try_into().unwrap()) as usize;
        let cluster = 16 + name_len + 4 + 160;
        archive[cluster + 2..cluster + 4].copy_from_slice(&8_u16.to_be_bytes());
        archive[cluster + 80] = 0xc1;
        archive[cluster + 84..cluster + 88].copy_from_slice(&[0x34, 0x12, 0x21, 0x43]);
        let world = retail_collision_world(&archive, material()).unwrap();
        let f = world.triangles()[0].triangle.feature;
        assert_eq!(f.flags, 0x1e1);
        assert_eq!(f.edge_cosines, [-1.; 3]);
    }
    #[test]
    #[ignore = "requires SKATE_MAP_TEST_PATH pointing to a private map"]
    fn private_map_builds_collision_world() {
        let path = std::env::var("SKATE_MAP_TEST_PATH").unwrap();
        let map = SkateMap::load(std::path::Path::new(&path)).unwrap();
        validate_runtime(&map).unwrap();
        let world = collision_world(&map, material()).unwrap();
        let [x, y, z] = map.spawn;
        let hit = world
            .query_thin_line(Vector3::new(x, y + 1., z), Vector3::new(x, y - 10., z))
            .unwrap();
        assert!(hit.is_some(), "spawn has no supporting collision");
        eprintln!(
            "Private map world: {} triangles, spawn hit {:?}",
            world.triangles().len(),
            hit
        );
    }
    #[test]
    fn map_collision_uses_separate_geometry_and_surface_metadata() {
        let mut map = demo();
        map.geometry.vertices[0].position = [1000.; 3];
        map.materials[0].audio = 42;
        map.materials[0].physics = 4;
        map.materials[0].pattern = 7;
        let world = collision_world(&map, material()).unwrap();
        assert_eq!(world.triangles().len(), 2);
        assert_eq!(
            world.triangles()[0].triangle.vertices[0],
            Vector3::new(-30., 0., -30.)
        );
        assert!(world.triangles()[0].triangle.feature.normal.y > 0.999);
        assert_eq!(
            world.query_metadata().unwrap().packed_surfaces,
            vec![42 | (4 << 7) | (7 << 12); 2]
        );
        assert!(world.query_metadata().unwrap().static_edges.is_empty());
    }
    #[test]
    fn shared_flat_diagonal_is_not_a_convex_contact_edge() {
        let world = collision_world(&demo(), material()).unwrap();
        let a = world.triangles()[0].triangle.feature;
        let b = world.triangles()[1].triangle.feature;
        assert!(!a.edge_convex(2));
        assert!(!b.edge_convex(0));
        assert!(a.edge_convex(0));
        assert_eq!(a.edge_cosines[2], 1.);
        assert!(a.vertex_disabled(0));
    }
    #[test]
    fn render_adapter_creates_mesh_and_decoded_lightmap_without_a_window() {
        let map = demo();
        let mut world = World::new();
        world.init_resource::<Assets<Mesh>>();
        world.init_resource::<Assets<StandardMaterial>>();
        world.init_resource::<Assets<crate::retail_render::RetailWorldMaterial>>();
        world.init_resource::<Assets<crate::retail_render::RetailSkyMaterial>>();
        world.init_resource::<Assets<Image>>();
        let mut scene = crate::map_render::PreparedScene::new(&world);
        scene.prepare(Some(&map), std::path::Path::new("unused"));
        scene.publish(&mut world);
        assert_eq!(world.resource::<Assets<Mesh>>().len(), 1);
        assert_eq!(world.resource::<Assets<StandardMaterial>>().len(), 1);
        assert_eq!(world.resource::<Assets<Image>>().len(), 2);
        assert_eq!(world.query::<&Mesh3d>().iter(&world).count(), 1);
        let lightmap = world
            .query::<&bevy::pbr::Lightmap>()
            .single(&world)
            .unwrap();
        let image = world.resource::<Assets<Image>>().get(&lightmap.image).unwrap();
        assert_eq!(image.texture_descriptor.format, TextureFormat::Rgba16Float);
        let data = image.data.as_ref().unwrap();
        let value = half::f16::from_le_bytes([data[0], data[1]]).to_f32();
        assert!((value - (64. / 255_f32).powi(2) * 4.).abs() < 0.0002);
    }
}
