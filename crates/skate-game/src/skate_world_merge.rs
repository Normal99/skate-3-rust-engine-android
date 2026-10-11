//! Phone level merging: groups compatible retail batches into few objects.
//! The rendering side is retail_merged.rs. Desktop never runs this.
use super::RenderGroup;
use crate::retail_render::MergedSlot;
use bevy::{
    prelude::*,
    render::{render_resource::TextureFormat, storage::ShaderStorageBuffer},
};
use skate_data::skate_map::SkateMap;
use std::collections::{BTreeMap, HashMap};

/// Texture slots held in arrays: diffuse, lightmap, normal, detail, macro
/// overlay, decal, specular. The environment cube stays a single texture.
pub(super) const ARRAY_SLOTS: usize = 7;
/// Vulkan guarantees 256 array layers; Mali allows more, other GPUs may not.
const MAX_LAYERS: usize = 256;
/// Keeps each upload a moderate allocation on a phone.
const MAX_ARRAY_BYTES: usize = 96 << 20;
/// Large merged objects are cut into cells this wide (metres) so frustum
/// culling and the draw distance still skip distant parts.
const CELL: f32 = 80.;
/// Objects smaller than this stay whole.
const SPLIT_TRIANGLES: usize = 4096;

/// Textures can share an array only with identical layout and sampler role.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(super) struct TextureClass {
    pub role: u8,
    pub width: u32,
    pub height: u32,
    pub format: TextureFormat,
    pub mips: u32,
}

pub(super) struct Member {
    pub indices: Vec<u32>,
    pub slot: MergedSlot,
}

pub(super) struct MergeSet {
    /// Index into Plan::arrays per texture slot; None binds a blank array.
    pub arrays: [Option<usize>; ARRAY_SLOTS],
    pub environment: Option<Handle<Image>>,
    pub alpha: AlphaMode,
    pub two_sided: bool,
    pub shadow_state: Handle<ShaderStorageBuffer>,
    members: Vec<Member>,
    /// A member reads this slot although it has no texture there (white
    /// fallback), so no other array may be bound to it.
    pinned_empty: [bool; ARRAY_SLOTS],
}
impl MergeSet {
    pub fn members(&self) -> &[Member] {
        &self.members
    }
}

pub(super) struct Plan {
    /// Staged texture indices, one per layer, for each array.
    pub arrays: Vec<Vec<usize>>,
    pub sets: Vec<MergeSet>,
    /// Batches that keep their own object (blended, fallback, generated tangents).
    pub rest: Vec<RenderGroup>,
}

struct Candidate {
    group: RenderGroup,
    staged: [Option<usize>; ARRAY_SLOTS],
    /// The shader skips this slot when its texture is absent (flag-gated).
    optional: [bool; ARRAY_SLOTS],
}

/// `staged` maps a texture handle to its staged index and class; only
/// staged textures can be copied into arrays.
pub(super) fn plan(
    groups: Vec<RenderGroup>,
    staged: impl Fn(&Handle<Image>) -> Option<(usize, TextureClass, usize)>,
) -> Plan {
    let mut rest = Vec::new();
    let mut candidates = Vec::new();
    let mut classes: HashMap<usize, (TextureClass, usize)> = HashMap::new();
    for group in groups {
        let eligible = group.tangent_mode != 1 && group.retail.as_ref().is_some_and(|m| {
            matches!(m.alpha, AlphaMode::Opaque | AlphaMode::Mask(_))
                && [&m.diffuse, &m.lightmap, &m.normal, &m.detail, &m.macro_map, &m.decal, &m.specular]
                    .iter().all(|h| h.as_ref().is_none_or(|h| staged(h).is_some()))
        });
        if !eligible {
            rest.push(group);
            continue;
        }
        let m = group.retail.as_ref().unwrap();
        let handles = [&m.diffuse, &m.lightmap, &m.normal, &m.detail, &m.macro_map, &m.decal, &m.specular];
        let staged = handles.map(|h| h.as_ref().map(|h| {
            let (index, class, bytes) = staged(h).unwrap();
            classes.insert(index, (class, bytes));
            index
        }));
        // retail_world.wgsl samples normal/detail/macro/decal/specular only
        // when their flag (set iff the texture exists) is on, except water
        // families, which read normal and detail unconditionally.
        let family = m.params.mode.x as u32;
        let optional = std::array::from_fn(|k| k >= 2 && (family < 30 || k >= 4));
        candidates.push(Candidate { group, staged, optional });
    }
    // Fill sets with the most complete materials first, so sparser ones
    // can join without opening new sets.
    candidates.sort_by_key(|c| std::cmp::Reverse(c.staged.iter().flatten().count()));

    // Array layers, assigned on first use. Each texture lives in one array.
    let mut arrays: Vec<Vec<usize>> = Vec::new();
    let mut array_bytes: Vec<usize> = Vec::new();
    let mut open: HashMap<TextureClass, usize> = HashMap::new();
    let mut layer_of: HashMap<usize, (usize, u32)> = HashMap::new();
    for candidate in &candidates {
        for &index in candidate.staged.iter().flatten() {
            if layer_of.contains_key(&index) {
                continue;
            }
            let (class, bytes) = classes[&index];
            let array = match open.get(&class) {
                Some(&a) if arrays[a].len() < MAX_LAYERS && array_bytes[a] + bytes <= MAX_ARRAY_BYTES => a,
                _ => {
                    arrays.push(Vec::new());
                    array_bytes.push(0);
                    open.insert(class, arrays.len() - 1);
                    arrays.len() - 1
                }
            };
            layer_of.insert(index, (array, arrays[array].len() as u32));
            arrays[array].push(index);
            array_bytes[array] += bytes;
        }
    }

    let mut sets: Vec<MergeSet> = Vec::new();
    let mut by_pipeline: HashMap<(bool, bool, u8, AssetId<ShaderStorageBuffer>), Vec<usize>> = HashMap::new();
    for Candidate { group, staged, optional } in candidates {
        let m = group.retail.as_ref().unwrap();
        let wanted = staged.map(|s| s.map(|i| layer_of[&i].0));
        let environment = m.environment.clone();
        let pipeline = (matches!(m.alpha, AlphaMode::Mask(_)), m.two_sided, group.tangent_mode, m.shadow_state.id());
        let list = by_pipeline.entry(pipeline).or_default();
        let fits = |set: &MergeSet| {
            (0..ARRAY_SLOTS).all(|k| match (set.arrays[k], wanted[k]) {
                (a, b) if a == b => true,
                (None, Some(_)) => !set.pinned_empty[k],
                (Some(_), None) => optional[k],
                _ => false,
            }) && match (&set.environment, &environment) {
                (Some(a), Some(b)) => a.id() == b.id(),
                _ => true,
            }
        };
        let index = match list.iter().copied().find(|&s| fits(&sets[s])) {
            Some(s) => s,
            None => {
                sets.push(MergeSet {
                    arrays: [None; ARRAY_SLOTS],
                    environment: None,
                    alpha: m.alpha,
                    two_sided: m.two_sided,
                    shadow_state: m.shadow_state.clone(),
                    members: Vec::new(),
                    pinned_empty: [false; ARRAY_SLOTS],
                });
                list.push(sets.len() - 1);
                sets.len() - 1
            }
        };
        let set = &mut sets[index];
        for k in 0..ARRAY_SLOTS {
            match wanted[k] {
                Some(a) => set.arrays[k] = Some(a),
                None if !optional[k] => set.pinned_empty[k] = true,
                None => {}
            }
        }
        if set.environment.is_none() {
            set.environment = environment;
        }
        let layers: [u32; ARRAY_SLOTS] = staged.map(|s| s.map_or(0, |i| layer_of[&i].1));
        let RenderGroup { indices, retail, .. } = group;
        set.members.push(Member {
            indices,
            slot: MergedSlot {
                params: retail.unwrap().params,
                layers_a: UVec4::new(layers[0], layers[1], layers[2], layers[3]),
                layers_b: UVec4::new(layers[4], layers[5], layers[6], 0),
            },
        });
    }
    Plan { arrays, sets, rest }
}

/// Corners (vertex index, member slot) of each object a set is drawn as.
pub(super) fn cells(map: &SkateMap, set: &MergeSet) -> Vec<Vec<(u32, u32)>> {
    let triangles: usize = set.members.iter().map(|m| m.indices.len() / 3).sum();
    let mut cells: BTreeMap<(i32, i32), Vec<(u32, u32)>> = BTreeMap::new();
    for (slot, member) in set.members.iter().enumerate() {
        for tri in member.indices.chunks_exact(3) {
            let key = if triangles < SPLIT_TRIANGLES {
                (0, 0)
            } else {
                let centre = tri.iter().map(|&i| Vec3::from_array(map.geometry.vertices[i as usize].position)).sum::<Vec3>() / 3.;
                ((centre.x / CELL).floor() as i32, (centre.z / CELL).floor() as i32)
            };
            cells.entry(key).or_default().extend(tri.iter().map(|&i| (i, slot as u32)));
        }
    }
    cells.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle(n: u128) -> Handle<Image> {
        Handle::Uuid(bevy::asset::uuid::Uuid::from_u128(n), default())
    }
    fn group(family: f32, diffuse: u128, lightmap: Option<u128>, normal: Option<u128>, alpha: AlphaMode) -> RenderGroup {
        let mut material = crate::retail_render::test_world_material();
        material.params.mode.x = family;
        material.diffuse = Some(handle(diffuse));
        material.lightmap = lightmap.map(handle);
        material.normal = normal.map(handle);
        material.alpha = alpha;
        RenderGroup { material: 0, indices: vec![0, 1, 2], retail: Some(material), tangent_mode: 2 }
    }
    fn classify(h: &Handle<Image>) -> Option<(usize, TextureClass, usize)> {
        let Handle::Uuid(id, _) = h else { return None };
        let n = id.as_u128();
        // Hundreds are 64x64 textures, everything else 128x128.
        let size = if n >= 100 { 64 } else { 128 };
        Some((n as usize, TextureClass { role: 3, width: size, height: size, format: TextureFormat::Rgba8Unorm, mips: 1 }, 4))
    }

    #[test]
    fn materials_share_sets_by_array_layout_and_keep_their_layers() {
        let groups = vec![
            group(1., 1, Some(2), Some(3), AlphaMode::Opaque),
            // Same sizes: joins the first set with its own layers.
            group(1., 4, Some(5), Some(6), AlphaMode::Opaque),
            // No normal map: the flag-gated slot is a wildcard.
            group(1., 7, Some(8), None, AlphaMode::Opaque),
            // Different lightmap size: its own set.
            group(1., 9, Some(100), Some(10), AlphaMode::Opaque),
            // Alpha-tested: separate pipeline.
            group(1., 11, Some(12), Some(13), AlphaMode::Mask(0.1)),
            // Blended stays a separate object.
            group(1., 14, Some(15), None, AlphaMode::Blend),
        ];
        let plan = plan(groups, classify);
        assert_eq!(plan.rest.len(), 1);
        assert_eq!(plan.sets.len(), 3);
        let first = &plan.sets[0];
        assert_eq!(first.members().len(), 3);
        let layers: Vec<_> = first.members().iter().map(|m| m.slot.layers_a).collect();
        assert_eq!(layers[0].x, 0);
        assert_ne!(layers[0].x, layers[1].x);
        assert_eq!(plan.arrays[first.arrays[0].unwrap()][layers[2].x as usize], 7);
        // Every texture is in exactly one array.
        let mut all: Vec<usize> = plan.arrays.iter().flatten().copied().collect();
        let count = all.len();
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), count);
    }

    #[test]
    fn a_missing_lightmap_is_not_a_wildcard() {
        // The shader always samples the lightmap; without one it reads white.
        let plan = plan(vec![
            group(1., 1, Some(2), None, AlphaMode::Opaque),
            group(1., 3, None, None, AlphaMode::Opaque),
        ], classify);
        assert_eq!(plan.sets.len(), 2);
        // Water reads its normal map unconditionally.
        let plan = super::plan(vec![
            group(30., 1, Some(2), Some(3), AlphaMode::Opaque),
            group(30., 4, Some(5), None, AlphaMode::Opaque),
        ], classify);
        assert_eq!(plan.sets.len(), 2);
    }

    #[test]
    fn small_sets_stay_whole() {
        let plan = plan(vec![group(1., 1, Some(2), None, AlphaMode::Opaque)], classify);
        let map = SkateMap::parse(include_bytes!("../../../maps/format-demo.skate")).unwrap();
        let cells = cells(&map, &plan.sets[0]);
        assert_eq!(cells, vec![vec![(0, 0), (1, 0), (2, 0)]]);
    }
}
