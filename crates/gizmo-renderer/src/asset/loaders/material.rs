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
    // `has_mips = true`: `loaders::images` artık her glTF dokusunu tam mip zinciriyle
    // yüklüyor. Buradaki eski yorum ("single mip level") o zincir üretilmeden önce
    // doğruydu; sampler `Nearest` kaldığı sürece zincir üretilse bile hiç örneklenmezdi.
    // Materyalin kendi `Nearest` tercihi korunuyor — `material_sampler` o durumda
    // anizotropiyi kapatıyor (wgpu üç filtrenin de `Linear` olmasını şart koşuyor).
    crate::texture_quality::material_sampler(
        device,
        "gltf_material_sampler",
        key.wrap_u,
        key.wrap_v,
        key.mag,
        key.min,
        true,
    )
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

/// F0 for a glTF material: `KHR_materials_ior` (default 1.5), times `KHR_materials_specular`'s factor.
///
/// The extension's own expression is `min(f0 * specular_color, 1) * specular_factor`, per channel; the
/// colour is [`Material::specular_tint`](crate::components::Material::specular_tint), which the shaders
/// multiply in, so this is the number and that is the hue. The `min` only bites past an F0 of 1, which an
/// index of refraction never reaches. Its textures are not read.
fn gltf_reflectance(ior: Option<f32>, specular: Option<(f32, [f32; 3])>) -> f32 {
    let f0 = crate::components::reflectance_from_ior(ior.unwrap_or(1.5));
    let (factor, _) = specular.unwrap_or((1.0, [1.0; 3]));
    f0.min(1.0) * factor
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

/// The name a glTF file's image goes by as a texture: `gltf:<file>#<image index>`. What a glTF
/// material's [`texture_source`](crate::components::Material::texture_source) and
/// [`maps`](crate::components::Material::maps) hold, and what
/// [`AssetManager::load_texture_view`](crate::asset::AssetManager::load_texture_view) resolves.
pub fn gltf_image_key(file_path: &str, image: usize) -> String {
    format!("gltf:{file_path}#{image}")
}

/// The file and the image index a [`gltf_image_key`] names, `None` for any other name. The index is
/// after the last `#`, so a `#` in the file's own name is kept in the path.
pub fn split_gltf_image_key(key: &str) -> Option<(&str, usize)> {
    let rest = key.strip_prefix("gltf:")?;
    let (file, index) = rest.rsplit_once('#')?;
    Some((file, index.parse().ok()?))
}

pub(super) fn build_gltf_materials(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    file_path: &str,
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
            let mat_sampler_key = material_sampler_key(&material);
            let mat_sampler = &sampler_cache[&mat_sampler_key];

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
            // With an emissive texture the factor multiplies it, in the material's params; without
            // one glTF's emission is the factor alone, which is what `Material::emissive` adds. Kept
            // there it is data (`MaterialMaps` zeroes a factor that has no map), so the material
            // shades the same when its bind group is built again from its description.
            let (emissive, instance_emissive) = if material.emissive_texture().is_some() {
                (emissive, [0.0; 3])
            } else {
                ([0.0; 3], emissive)
            };
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
                    &defaults.white_view,
                    &params_buffer,
                    &format!("gltf_material_{}", material.index().unwrap_or(usize::MAX)),
                )
            };

            let mut mat = Material::new(bind_group);
            // Every texture named by the image it is in this file (`gltf_image_key`), so the material
            // is data like any other: a scene saves these names, and `AssetManager::load_material`
            // finds the images again — imported once more if this run has not read the file.
            let key = |image: Option<usize>| image.map(|i| gltf_image_key(file_path, i));
            mat.texture_source = key(pbr.base_color_texture().map(|t| t.texture().source().index()));
            let image = |info: Option<gltf::texture::Info>| key(info.map(|t| t.texture().source().index()));
            mat.maps = crate::components::MaterialMaps {
                normal: key(material.normal_texture().map(|t| t.texture().source().index())),
                normal_scale,
                metallic_roughness: image(pbr.metallic_roughness_texture()),
                emissive: image(material.emissive_texture()),
                emissive_factor: gizmo_math::Vec3::from(emissive),
                occlusion: key(material.occlusion_texture().map(|t| t.texture().source().index())),
                occlusion_strength,
                // glTF has no lightmap of its own: a baking tool's goes on the material afterwards.
                lightmap: None,
                lightmap_intensity: 1.0,
                uv_scale: uv_transform.scale,
                uv_rotation: uv_transform.rotation,
                uv_offset: uv_transform.offset,
                wrap_u: crate::components::TextureWrap::from_address_mode(mat_sampler_key.wrap_u),
                wrap_v: crate::components::TextureWrap::from_address_mode(mat_sampler_key.wrap_v),
                mag_filter: crate::components::TextureFilter::from_filter_mode(mat_sampler_key.mag),
                min_filter: crate::components::TextureFilter::from_filter_mode(mat_sampler_key.min),
            };
            // The bind group above holds these maps: not to be built again on the first frame.
            mat.mark_maps_bound();
            mat.emissive = gizmo_math::Vec3::from(instance_emissive);

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
            // So the draw loops know it discards. The material's own uniform is what the shader tests.
            mat.alpha_cutoff = alpha_cutoff;
            mat = mat.with_reflectance(gltf_reflectance(
                material.ior(),
                material.specular().map(|s| (s.specular_factor(), s.specular_color_factor())),
            ));
            if let Some(s) = material.specular() {
                mat = mat.with_specular_tint(gizmo_math::Vec3::from(s.specular_color_factor()));
            }

            // Only `Blend` routes to the transparent pass. `Mask` (cutout) is opaque geometry
            // with a per-texel discard (handled in gbuffer.wgsl via alpha_cutoff) — routing it
            // as blend gave soft translucent fringes + depth-sort artifacts instead of a crisp
            // cutout. `Opaque` stays opaque; a sub-unit base alpha or a glass name is translucent.
            mat.is_transparent = material.alpha_mode() == gltf::material::AlphaMode::Blend
                || alpha < 0.99
                || is_glass;
            mat.is_double_sided = material.double_sided();
            // `KHR_materials_transmission` and `KHR_materials_volume`, without their textures.
            if let Some(t) = material.transmission() {
                mat = mat.with_transmission(t.transmission_factor());
            }
            if let Some(v) = material.volume() {
                mat = mat.with_thickness(v.thickness_factor());
                // glTF's default distance is infinite: no attenuation, which is this engine's 0.
                let distance = v.attenuation_distance();
                if distance.is_finite() {
                    mat = mat.with_attenuation(gizmo_math::Vec3::from(v.attenuation_color()), distance);
                }
            }

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
    fn ior_and_specular_extensions_reach_the_reflectance() {
        let json = r#"{
          "asset": { "version": "2.0" },
          "extensionsUsed": ["KHR_materials_ior", "KHR_materials_specular"],
          "materials": [
            {},
            { "extensions": { "KHR_materials_ior": { "ior": 2.0 } } },
            { "extensions": { "KHR_materials_specular": { "specularFactor": 0.5 } } },
            { "extensions": {
                "KHR_materials_ior": { "ior": 2.0 },
                "KHR_materials_specular": { "specularColorFactor": [0.0, 1.0, 0.0] } } }
          ]
        }"#;
        let doc = gltf::Gltf::from_slice(json.as_bytes()).expect("parse");
        let got: Vec<f32> = doc
            .materials()
            .map(|m| gltf_reflectance(m.ior(), m.specular().map(|s| (s.specular_factor(), s.specular_color_factor()))))
            .collect();
        let close = |a: f32, b: f32| (a - b).abs() < 1e-5;
        assert!(close(got[0], 0.04), "no extension is ior 1.5: {}", got[0]);
        assert!(close(got[1], 1.0 / 9.0), "ior 2: {}", got[1]);
        assert!(close(got[2], 0.02), "half the specular: {}", got[2]);
        assert!(close(got[3], 1.0 / 9.0), "a green tint leaves the number to the index: {}", got[3]);
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
