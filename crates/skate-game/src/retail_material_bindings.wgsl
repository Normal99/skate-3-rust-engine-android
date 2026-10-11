#define_import_path skate_retail::material_bindings
struct WorldParams {
    mode: vec4<f32>, foliage_debug: vec4<f32>, surface: vec4<f32>, family: vec4<f32>,
    fog_ramp: vec4<f32>, fog_color: vec4<f32>, shadow_color: vec4<f32>, sun_direction: vec4<f32>, decal: vec4<f32>, water: array<vec4<f32>, 4>,
}
struct FrameState { shadow: vec4<f32>, clock: vec4<f32>, pca: array<vec4<f32>, 7> }

#ifdef BINDLESS
struct MaterialIndices {
    params: u32,
    diffuse: u32,
    diffuse_sampler: u32,
    lightmap: u32,
    lm_sampler: u32,
    normal_map: u32,
    normal_sampler: u32,
    detail_map: u32,
    detail_sampler: u32,
    macro_map: u32,
    macro_sampler: u32,
    decal_map: u32,
    decal_sampler: u32,
    specular_map: u32,
    specular_sampler: u32,
    environment_map: u32,
    frame_state: u32,
}
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<storage> indices: array<MaterialIndices>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var samplers: binding_array<sampler>;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var textures: binding_array<texture_2d<f32>>;
@group(#{MATERIAL_BIND_GROUP}) @binding(8) var cubes: binding_array<texture_cube<f32>>;
@group(#{MATERIAL_BIND_GROUP}) @binding(17) var<storage> params: array<WorldParams>;
@group(#{MATERIAL_BIND_GROUP}) @binding(18) var<storage> frames: binding_array<FrameState>;
#else
#ifdef MERGED_LEVEL
// Phone merged level: the slot comes from vertex colour z and selects the
// material's parameters and its layer in each same-sized texture array.
struct MergedSlot { params: WorldParams, layers_a: vec4<u32>, layers_b: vec4<u32> }
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<storage, read> slots: array<MergedSlot>;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var diffuse: texture_2d_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var diffuse_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var lightmap: texture_2d_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var lm_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var normal_map: texture_2d_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var normal_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(7) var detail_map: texture_2d_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(8) var detail_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(9) var macro_map: texture_2d_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(10) var macro_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(11) var decal_map: texture_2d_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(12) var decal_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(13) var specular_map: texture_2d_array<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(14) var specular_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(15) var environment_map: texture_cube<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(16) var<storage, read> frame_state: FrameState;
#else
@group(#{MATERIAL_BIND_GROUP}) @binding(0) var<uniform> p: WorldParams;
@group(#{MATERIAL_BIND_GROUP}) @binding(1) var diffuse: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(2) var diffuse_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(3) var lightmap: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(4) var lm_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(5) var normal_map: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(6) var normal_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(7) var detail_map: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(8) var detail_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(9) var macro_map: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(10) var macro_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(11) var decal_map: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(12) var decal_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(13) var specular_map: texture_2d<f32>;
@group(#{MATERIAL_BIND_GROUP}) @binding(14) var specular_sampler: sampler;
@group(#{MATERIAL_BIND_GROUP}) @binding(15) var environment_map: texture_cube<f32>;

@group(#{MATERIAL_BIND_GROUP}) @binding(16) var<storage, read> frame_state: FrameState;
#endif
#endif

// Keep resource-array access at sampling sites. Passing selected resources into
// a shading function crashes the tested Vulkan pipeline compiler (v6 probe).

fn sample_diffuse(slot: u32, uv: vec2<f32>) -> vec4<f32> {
#ifdef BINDLESS
    let index = indices[slot];
    return textureSample(textures[index.diffuse],samplers[index.diffuse_sampler],uv);
#else
#ifdef MERGED_LEVEL
    return textureSample(diffuse,diffuse_sampler,uv,slots[slot].layers_a.x);
#else
    return textureSample(diffuse,diffuse_sampler,uv);
#endif
#endif
}

fn sample_normal_map(slot: u32, uv: vec2<f32>) -> vec4<f32> {
#ifdef BINDLESS
    let index = indices[slot];
    return textureSample(textures[index.normal_map],samplers[index.normal_sampler],uv);
#else
#ifdef MERGED_LEVEL
    return textureSample(normal_map,normal_sampler,uv,slots[slot].layers_a.z);
#else
    return textureSample(normal_map,normal_sampler,uv);
#endif
#endif
}

fn sample_detail_map(slot: u32, uv: vec2<f32>) -> vec4<f32> {
#ifdef BINDLESS
    let index = indices[slot];
    return textureSample(textures[index.detail_map],samplers[index.detail_sampler],uv);
#else
#ifdef MERGED_LEVEL
    return textureSample(detail_map,detail_sampler,uv,slots[slot].layers_a.w);
#else
    return textureSample(detail_map,detail_sampler,uv);
#endif
#endif
}

fn sample_macro_map(slot: u32, uv: vec2<f32>) -> vec4<f32> {
#ifdef BINDLESS
    let index = indices[slot];
    return textureSample(textures[index.macro_map],samplers[index.macro_sampler],uv);
#else
#ifdef MERGED_LEVEL
    return textureSample(macro_map,macro_sampler,uv,slots[slot].layers_b.x);
#else
    return textureSample(macro_map,macro_sampler,uv);
#endif
#endif
}

fn sample_decal_map(slot: u32, uv: vec2<f32>) -> vec4<f32> {
#ifdef BINDLESS
    let index = indices[slot];
    return textureSample(textures[index.decal_map],samplers[index.decal_sampler],uv);
#else
#ifdef MERGED_LEVEL
    return textureSample(decal_map,decal_sampler,uv,slots[slot].layers_b.y);
#else
    return textureSample(decal_map,decal_sampler,uv);
#endif
#endif
}

fn sample_specular_map(slot: u32, uv: vec2<f32>) -> vec4<f32> {
#ifdef BINDLESS
    let index = indices[slot];
    return textureSample(textures[index.specular_map],samplers[index.specular_sampler],uv);
#else
#ifdef MERGED_LEVEL
    return textureSample(specular_map,specular_sampler,uv,slots[slot].layers_b.z);
#else
    return textureSample(specular_map,specular_sampler,uv);
#endif
#endif
}

fn sample_lightmap(slot: u32, uv: vec2<f32>, level: f32) -> vec4<f32> {
#ifdef BINDLESS
    let index = indices[slot];
    return textureSampleLevel(textures[index.lightmap],samplers[index.lm_sampler],uv,level);
#else
#ifdef MERGED_LEVEL
    return textureSampleLevel(lightmap,lm_sampler,uv,slots[slot].layers_a.y,level);
#else
    return textureSampleLevel(lightmap,lm_sampler,uv,level);
#endif
#endif
}

fn sample_environment(slot: u32, direction: vec3<f32>, bias: f32) -> vec4<f32> {
#ifdef BINDLESS
    let index = indices[slot];
    return textureSampleBias(cubes[index.environment_map],samplers[index.diffuse_sampler],direction,bias);
#else
#ifdef MERGED_LEVEL
    return textureSampleBias(environment_map,diffuse_sampler,direction,bias);
#else
    return textureSampleBias(environment_map,diffuse_sampler,direction,bias);
#endif
#endif
}

fn load_detail(slot: u32, uv: vec2<i32>, level: i32) -> vec4<f32> {
#ifdef BINDLESS
    let index = indices[slot];
    return textureLoad(textures[index.detail_map],uv,level);
#else
#ifdef MERGED_LEVEL
    return textureLoad(detail_map,uv,slots[slot].layers_a.w,level);
#else
    return textureLoad(detail_map,uv,level);
#endif
#endif
}

fn lightmap_dimensions(slot: u32) -> vec2<u32> {
#ifdef BINDLESS
    let index = indices[slot];
    return textureDimensions(textures[index.lightmap]);
#else
#ifdef MERGED_LEVEL
    return textureDimensions(lightmap);
#else
    return textureDimensions(lightmap);
#endif
#endif
}
