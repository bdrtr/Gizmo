//! glTF material building — sampler resolution, KHR_texture_transform / emissive-strength,
//! and the per-material bind group. Extracted verbatim from `loaders.rs` (pure move).
//! `build_gltf_materials` is called from `load_gltf_from_import`.

use super::*;
use super::images::GpuImage;

/// glTF sampler settings resolved to wgpu enums. Hashable so identical
/// configurations across materials share a single `wgpu::Sampler`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct SamplerKey {
    wrap_u: wgpu::AddressMode,
    wrap_v: wgpu::AddressMode,
    mag: wgpu::FilterMode,
    min: wgpu::FilterMode,
}

impl SamplerKey {
    /// glTF default when a texture references no sampler: repeat + linear.
    const DEFAULT: SamplerKey = SamplerKey {
        wrap_u: wgpu::AddressMode::Repeat,
        wrap_v: wgpu::AddressMode::Repeat,
        mag: wgpu::FilterMode::Linear,
        min: wgpu::FilterMode::Linear,
    };

    fn from_gltf(s: &gltf::texture::Sampler) -> SamplerKey {
        SamplerKey {
            wrap_u: wrap_to_wgpu(s.wrap_s()),
            wrap_v: wrap_to_wgpu(s.wrap_t()),
            mag: mag_to_wgpu(s.mag_filter()),
            min: min_to_wgpu(s.min_filter()),
        }
    }
}

fn wrap_to_wgpu(m: gltf::texture::WrappingMode) -> wgpu::AddressMode {
    use gltf::texture::WrappingMode;
    match m {
        WrappingMode::ClampToEdge => wgpu::AddressMode::ClampToEdge,
        WrappingMode::MirroredRepeat => wgpu::AddressMode::MirrorRepeat,
        WrappingMode::Repeat => wgpu::AddressMode::Repeat,
    }
}

fn mag_to_wgpu(f: Option<gltf::texture::MagFilter>) -> wgpu::FilterMode {
    match f {
        Some(gltf::texture::MagFilter::Nearest) => wgpu::FilterMode::Nearest,
        _ => wgpu::FilterMode::Linear,
    }
}

fn min_to_wgpu(f: Option<gltf::texture::MinFilter>) -> wgpu::FilterMode {
    use gltf::texture::MinFilter;
    match f {
        Some(MinFilter::Nearest)
        | Some(MinFilter::NearestMipmapNearest)
        | Some(MinFilter::NearestMipmapLinear) => wgpu::FilterMode::Nearest,
        _ => wgpu::FilterMode::Linear,
    }
}

fn create_gltf_sampler(device: &wgpu::Device, key: SamplerKey) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("gltf_material_sampler"),
        address_mode_u: key.wrap_u,
        address_mode_v: key.wrap_v,
        address_mode_w: wgpu::AddressMode::Repeat,
        mag_filter: key.mag,
        min_filter: key.min,
        mipmap_filter: wgpu::MipmapFilterMode::Nearest, // single mip level
        ..Default::default()
    })
}

/// Resolve which sampler configuration a material's maps should use.
///
/// The material bind group carries a single shared sampler (the g-buffer samples
/// every map through one `s_diffuse`), so we honour the sampler of the first
/// defined map in priority order (base → normal → MR → emissive → AO). Real
/// glTF exporters assign one sampler per material, so this matches the asset; a
/// material whose maps reference *divergent* samplers (rare) uses the first.
fn material_sampler_key(material: &gltf::Material) -> SamplerKey {
    let pbr = material.pbr_metallic_roughness();
    let tex = pbr
        .base_color_texture()
        .map(|t| t.texture())
        .or_else(|| material.normal_texture().map(|t| t.texture()))
        .or_else(|| pbr.metallic_roughness_texture().map(|t| t.texture()))
        .or_else(|| material.emissive_texture().map(|t| t.texture()))
        .or_else(|| material.occlusion_texture().map(|t| t.texture()));
    match tex {
        Some(t) => SamplerKey::from_gltf(&t.sampler()),
        None => SamplerKey::DEFAULT,
    }
}

/// Apply `KHR_materials_emissive_strength` to the emissive factor.
///
/// The extension multiplies the emissive colour by a scalar (default 1.0) to
/// express HDR glow. The g-buffer stores emissive additively (LDR approximation),
/// so strengths > 1 brighten emission up to the render-target range; full unlit
/// HDR bloom still needs a dedicated emissive target (tracked in the ROADMAP).
fn emissive_with_strength(factor: [f32; 3], strength: Option<f32>) -> [f32; 3] {
    let s = strength.unwrap_or(1.0);
    [factor[0] * s, factor[1] * s, factor[2] * s]
}

/// Resolve the `KHR_texture_transform` (UV offset / rotation / scale) for a
/// material, from its base-colour texture (identity when absent).
///
/// The g-buffer applies a single UV transform per material to every map, so we
/// take the base-colour map's transform — real assets that tile/offset a
/// material apply the same transform across its maps. A per-map transform on a
/// non-base map, or a `texCoord` set override, is not represented (single UV
/// channel); such rare cases fall back to the base-colour transform.
fn material_uv_transform(material: &gltf::Material) -> crate::gpu_types::UvTransform {
    match material
        .pbr_metallic_roughness()
        .base_color_texture()
        .and_then(|ti| ti.texture_transform())
    {
        Some(tt) => crate::gpu_types::UvTransform {
            offset: tt.offset(),
            rotation: tt.rotation(),
            scale: tt.scale(),
        },
        None => crate::gpu_types::UvTransform::default(),
    }
}

pub(super) fn build_gltf_materials(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    document: &gltf::Document,
    gpu_images: &[GpuImage],
    defaults: &crate::asset::MaterialDefaults,
    default_tbind: &Arc<wgpu::BindGroup>,
) -> Vec<Material> {
    // One wgpu sampler per distinct glTF sampler configuration used by the
    // document's materials — honours wrap + filter settings instead of forcing
    // repeat/linear. Identical configurations are shared via the cache key.
    let mut sampler_cache: std::collections::HashMap<SamplerKey, wgpu::Sampler> =
        std::collections::HashMap::new();
    for material in document.materials() {
        let key = material_sampler_key(&material);
        sampler_cache
            .entry(key)
            .or_insert_with(|| create_gltf_sampler(device, key));
    }
    // Textureless materials still need *a* sampler for their bind group.
    sampler_cache
        .entry(SamplerKey::DEFAULT)
        .or_insert_with(|| create_gltf_sampler(device, SamplerKey::DEFAULT));

    document
        .materials()
        .map(|material| {
            let pbr = material.pbr_metallic_roughness();
            let base_color = pbr.base_color_factor();

            // Shared sampler honouring this material's glTF wrap/filter settings.
            let mat_sampler = &sampler_cache[&material_sampler_key(&material)];

            // Resolve each map's image view (or the neutral default).
            let base_view = pbr
                .base_color_texture()
                .and_then(|ti| gpu_images.get(ti.texture().source().index()))
                .map(|img| &img.view)
                .unwrap_or(&defaults.white_view);
            let normal_view = material
                .normal_texture()
                .and_then(|nt| gpu_images.get(nt.texture().source().index()))
                .map(|img| &img.view)
                .unwrap_or(&defaults.flat_normal_view);
            let mr_view = pbr
                .metallic_roughness_texture()
                .and_then(|ti| gpu_images.get(ti.texture().source().index()))
                .map(|img| &img.view)
                .unwrap_or(&defaults.white_view);
            let emissive_view = material
                .emissive_texture()
                .and_then(|ti| gpu_images.get(ti.texture().source().index()))
                .map(|img| &img.view)
                .unwrap_or(&defaults.white_view);
            let ao_view = material
                .occlusion_texture()
                .and_then(|ot| gpu_images.get(ot.texture().source().index()))
                .map(|img| &img.view)
                .unwrap_or(&defaults.white_view);

            let has_base = pbr.base_color_texture().is_some();
            let has_any_map = has_base
                || material.normal_texture().is_some()
                || pbr.metallic_roughness_texture().is_some()
                || material.emissive_texture().is_some()
                || material.occlusion_texture().is_some();

            // Per-material scalar params (glTF factors that modulate the maps).
            // KHR_materials_emissive_strength scales the emissive factor for HDR
            // glow; folded into the factor here (LDR-additive in the g-buffer).
            let emissive =
                emissive_with_strength(material.emissive_factor(), material.emissive_strength());
            let normal_scale = material.normal_texture().map(|nt| nt.scale()).unwrap_or(1.0);
            let occlusion_strength = material
                .occlusion_texture()
                .map(|ot| ot.strength())
                .unwrap_or(1.0);
            let uv_transform = material_uv_transform(&material);
            // glTF alpha cutout: `AlphaMode::Mask` is OPAQUE geometry with a hard discard
            // at `alphaCutoff` (default 0.5), NOT alpha blending. 0.0 → no cutout (Opaque/Blend).
            let alpha_cutoff = if material.alpha_mode() == gltf::material::AlphaMode::Mask {
                material.alpha_cutoff().unwrap_or(0.5)
            } else {
                0.0
            };
            let params = crate::gpu_types::MaterialParams::new(
                emissive,
                normal_scale,
                occlusion_strength,
                uv_transform,
                alpha_cutoff,
            );
            let is_default_params = emissive == [0.0, 0.0, 0.0]
                && normal_scale == 1.0
                && occlusion_strength == 1.0
                && uv_transform.is_identity()
                && alpha_cutoff == 0.0;

            // Fast path: no textures and neutral params → reuse the shared white
            // fallback bind group. Otherwise assemble a dedicated one.
            let bind_group = if !has_any_map && is_default_params {
                default_tbind.clone()
            } else {
                let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(&format!(
                        "gltf_material_params_{}",
                        material.index().unwrap_or(usize::MAX)
                    )),
                    contents: bytemuck::cast_slice(&[params]),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
                crate::asset::AssetManager::assemble_material_bind_group(
                    device,
                    layout,
                    base_view,
                    mat_sampler,
                    normal_view,
                    mr_view,
                    emissive_view,
                    ao_view,
                    &params_buffer,
                    &format!("gltf_material_{}", material.index().unwrap_or(usize::MAX)),
                )
            };

            let mut mat = Material::new(bind_group);
            if has_base {
                mat.texture_source = Some(format!(
                    "gltf_tex_base_{}",
                    material.index().unwrap_or(usize::MAX)
                ));
            }

            let mat_name = material.name().unwrap_or("").to_lowercase();
            let is_glass = mat_name.contains("glass");

            let alpha = if is_glass {
                0.25 // Glass bulb should be translucent and glowing!
            } else if material.alpha_mode() == gltf::material::AlphaMode::Opaque {
                1.0
            } else {
                base_color[3]
            };

            tracing::debug!("GLTF LOAD MAT: name={:?}, alpha_mode={:?}, alpha_factor={}, base_color={:?}, double_sided={}",
                material.name(), material.alpha_mode(), alpha, base_color, material.double_sided());

            mat.albedo = gizmo_math::Vec4::new(base_color[0], base_color[1], base_color[2], alpha);
            mat.metallic = pbr.metallic_factor();
            mat.roughness = pbr.roughness_factor();

            // Only `Blend` routes to the transparent pass. `Mask` (cutout) is opaque geometry
            // with a per-texel discard (handled in gbuffer.wgsl via alpha_cutoff) — routing it
            // as blend gave soft translucent fringes + depth-sort artifacts instead of a crisp
            // cutout. `Opaque` stays opaque; a sub-unit base alpha or a glass name is translucent.
            mat.is_transparent = material.alpha_mode() == gltf::material::AlphaMode::Blend
                || alpha < 0.99
                || is_glass;
            mat.is_double_sided = material.double_sided();

            mat
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emissive_strength_scales_factor() {
        // Absent extension → factor unchanged.
        assert_eq!(emissive_with_strength([1.0, 0.5, 0.0], None), [1.0, 0.5, 0.0]);
        // Strength multiplies each channel (HDR glow).
        assert_eq!(emissive_with_strength([1.0, 0.5, 0.25], Some(4.0)), [4.0, 2.0, 1.0]);
        // Zero strength kills emission.
        assert_eq!(emissive_with_strength([1.0, 1.0, 1.0], Some(0.0)), [0.0, 0.0, 0.0]);
    }

    #[test]
    fn sampler_filter_and_wrap_converters() {
        use gltf::texture::{MagFilter, MinFilter, WrappingMode};
        assert_eq!(wrap_to_wgpu(WrappingMode::ClampToEdge), wgpu::AddressMode::ClampToEdge);
        assert_eq!(wrap_to_wgpu(WrappingMode::MirroredRepeat), wgpu::AddressMode::MirrorRepeat);
        assert_eq!(wrap_to_wgpu(WrappingMode::Repeat), wgpu::AddressMode::Repeat);

        assert_eq!(mag_to_wgpu(Some(MagFilter::Nearest)), wgpu::FilterMode::Nearest);
        assert_eq!(mag_to_wgpu(Some(MagFilter::Linear)), wgpu::FilterMode::Linear);
        assert_eq!(mag_to_wgpu(None), wgpu::FilterMode::Linear);

        assert_eq!(min_to_wgpu(Some(MinFilter::Nearest)), wgpu::FilterMode::Nearest);
        assert_eq!(min_to_wgpu(Some(MinFilter::NearestMipmapLinear)), wgpu::FilterMode::Nearest);
        assert_eq!(min_to_wgpu(Some(MinFilter::Linear)), wgpu::FilterMode::Linear);
        assert_eq!(min_to_wgpu(Some(MinFilter::LinearMipmapLinear)), wgpu::FilterMode::Linear);
        assert_eq!(min_to_wgpu(None), wgpu::FilterMode::Linear);
    }

    #[test]
    fn gltf_material_sampler_and_emissive_strength_parsed() {
        // Minimal glTF: a material whose base-colour texture references a sampler
        // with mixed wrap/filter modes, plus KHR_materials_emissive_strength = 4.
        let json = r#"{
          "asset": { "version": "2.0" },
          "extensionsUsed": ["KHR_materials_emissive_strength"],
          "samplers": [
            { "wrapS": 33071, "wrapT": 10497, "magFilter": 9728, "minFilter": 9729 }
          ],
          "images": [ { "uri": "dummy.png" } ],
          "textures": [ { "sampler": 0, "source": 0 } ],
          "materials": [
            {
              "pbrMetallicRoughness": { "baseColorTexture": { "index": 0 } },
              "emissiveFactor": [1.0, 0.5, 0.25],
              "extensions": { "KHR_materials_emissive_strength": { "emissiveStrength": 4.0 } }
            }
          ]
        }"#;
        let doc = gltf::Gltf::from_slice(json.as_bytes()).expect("parse minimal glTF");
        let material = doc.materials().next().expect("one material");

        // Sampler settings honoured (not the old hardcoded repeat/linear).
        let key = material_sampler_key(&material);
        assert_eq!(key.wrap_u, wgpu::AddressMode::ClampToEdge);
        assert_eq!(key.wrap_v, wgpu::AddressMode::Repeat);
        assert_eq!(key.mag, wgpu::FilterMode::Nearest);
        assert_eq!(key.min, wgpu::FilterMode::Linear);

        // KHR_materials_emissive_strength scales the emissive factor.
        assert_eq!(material.emissive_strength(), Some(4.0));
        let emissive =
            emissive_with_strength(material.emissive_factor(), material.emissive_strength());
        assert_eq!(emissive, [4.0, 2.0, 1.0]);
    }

    #[test]
    fn material_without_textures_uses_default_sampler_key() {
        let json = r#"{
          "asset": { "version": "2.0" },
          "materials": [ { "emissiveFactor": [0.0, 0.0, 0.0] } ]
        }"#;
        let doc = gltf::Gltf::from_slice(json.as_bytes()).expect("parse");
        let material = doc.materials().next().expect("one material");
        assert_eq!(material_sampler_key(&material), SamplerKey::DEFAULT);
        // Absent extension → no strength (folds to factor unchanged).
        assert_eq!(material.emissive_strength(), None);
    }

    #[test]
    fn gltf_texture_transform_parsed_and_packed() {
        // KHR_texture_transform on the base-colour texture: offset/rotation/scale.
        let json = r#"{
          "asset": { "version": "2.0" },
          "extensionsUsed": ["KHR_texture_transform"],
          "images": [ { "uri": "dummy.png" } ],
          "samplers": [ {} ],
          "textures": [ { "sampler": 0, "source": 0 } ],
          "materials": [
            {
              "pbrMetallicRoughness": {
                "baseColorTexture": {
                  "index": 0,
                  "extensions": {
                    "KHR_texture_transform": {
                      "offset": [0.1, 0.2],
                      "rotation": 1.5,
                      "scale": [2.0, 3.0]
                    }
                  }
                }
              }
            }
          ]
        }"#;
        let doc = gltf::Gltf::from_slice(json.as_bytes()).expect("parse");
        let material = doc.materials().next().expect("one material");

        let uv = material_uv_transform(&material);
        assert_eq!(uv.offset, [0.1, 0.2]);
        assert!((uv.rotation - 1.5).abs() < 1e-6);
        assert_eq!(uv.scale, [2.0, 3.0]);
        assert!(!uv.is_identity());

        // Packed into MaterialParams in the documented slots:
        // occlusion_uv_rot_offset = [occlusion, rotation, offset.x, offset.y].
        let params = crate::gpu_types::MaterialParams::new([0.0; 3], 1.0, 1.0, uv, 0.0);
        assert_eq!(params.occlusion_uv_rot_offset, [1.0, 1.5, 0.1, 0.2]);
        assert_eq!(params.uv_scale, [2.0, 3.0, 0.0, 0.0]);
    }

    #[test]
    fn material_without_texture_transform_is_identity() {
        let json = r#"{
          "asset": { "version": "2.0" },
          "images": [ { "uri": "dummy.png" } ],
          "samplers": [ {} ],
          "textures": [ { "sampler": 0, "source": 0 } ],
          "materials": [ { "pbrMetallicRoughness": { "baseColorTexture": { "index": 0 } } } ]
        }"#;
        let doc = gltf::Gltf::from_slice(json.as_bytes()).expect("parse");
        let material = doc.materials().next().expect("one material");
        assert!(material_uv_transform(&material).is_identity());
        // Default MaterialParams carries an identity UV (unit scale, zero offset/rot).
        let d = crate::gpu_types::MaterialParams::default();
        assert_eq!(d.uv_scale, [1.0, 1.0, 0.0, 0.0]);
        assert_eq!(d.occlusion_uv_rot_offset, [1.0, 0.0, 0.0, 0.0]);
    }
}
