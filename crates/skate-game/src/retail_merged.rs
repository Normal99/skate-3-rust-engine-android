//! Phone path: many retail world materials drawn as one object.
//!
//! Bevy spends main-thread time on every mesh entity each frame (visibility,
//! transforms, change detection, extraction, queueing). University has ~4,900
//! world batches, one per distinct material. Phones lack bindless textures, so
//! here textures of the same role and size share a texture array, each vertex
//! carries its material slot (vertex colour z), and the slot selects per-material
//! parameters and array layers. The shading code is the same retail shader.
use bevy::{
    prelude::*,
    render::{render_resource::{AsBindGroup, ShaderType}, storage::ShaderStorageBuffer},
    shader::ShaderRef,
};
use super::WorldParams;

/// One source material inside a merged object.
#[derive(Clone, Debug, ShaderType)]
pub(crate) struct MergedSlot {
    pub params: WorldParams,
    /// Array layers: diffuse, lightmap, normal, detail.
    pub layers_a: UVec4,
    /// Array layers: macro overlay, decal, specular, unused.
    pub layers_b: UVec4,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
#[bind_group_data(MergedKey)]
pub(crate) struct MergedWorldMaterial {
    #[storage(0, read_only)]
    pub slots: Handle<ShaderStorageBuffer>,
    #[texture(1, dimension = "2d_array")]
    #[sampler(2)]
    pub diffuse: Option<Handle<Image>>,
    #[texture(3, dimension = "2d_array")]
    #[sampler(4)]
    pub lightmap: Option<Handle<Image>>,
    #[texture(5, dimension = "2d_array")]
    #[sampler(6)]
    pub normal: Option<Handle<Image>>,
    #[texture(7, dimension = "2d_array")]
    #[sampler(8)]
    pub detail: Option<Handle<Image>>,
    #[texture(9, dimension = "2d_array")]
    #[sampler(10)]
    pub macro_map: Option<Handle<Image>>,
    #[texture(11, dimension = "2d_array")]
    #[sampler(12)]
    pub decal: Option<Handle<Image>>,
    #[texture(13, dimension = "2d_array")]
    #[sampler(14)]
    pub specular: Option<Handle<Image>>,
    #[texture(15, dimension = "cube")]
    pub environment: Option<Handle<Image>>,
    #[storage(16, read_only)]
    pub shadow_state: Handle<ShaderStorageBuffer>,
    pub alpha: AlphaMode,
    pub two_sided: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct MergedKey {
    two_sided: bool,
    alpha_test: bool,
}
impl From<&MergedWorldMaterial> for MergedKey {
    fn from(m: &MergedWorldMaterial) -> Self {
        Self { two_sided: m.two_sided, alpha_test: matches!(m.alpha, AlphaMode::Mask(_)) }
    }
}

impl Material for MergedWorldMaterial {
    fn fragment_shader() -> ShaderRef {
        super::retail_shader("retail_world.wgsl")
    }
    fn prepass_fragment_shader() -> ShaderRef {
        super::retail_shader("retail_depth.wgsl")
    }
    fn alpha_mode(&self) -> AlphaMode {
        self.alpha
    }
    fn specialize(
        _: &bevy::pbr::MaterialPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _: &bevy::mesh::MeshVertexBufferLayoutRef,
        key: bevy::pbr::MaterialPipelineKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        descriptor.primitive.cull_mode = if key.bind_group_data.two_sided {
            None
        } else {
            Some(bevy::render::render_resource::Face::Back)
        };
        if let Some(fragment) = descriptor.fragment.as_mut() {
            fragment.shader_defs.push("MERGED_LEVEL".into());
            if key.bind_group_data.alpha_test {
                fragment.shader_defs.push("RETAIL_ALPHA_TEST".into());
            }
        }
        Ok(())
    }
}
