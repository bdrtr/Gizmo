use super::decode_obj_vertices_for_async;
use crate::animation::{AnimationClip, Keyframe, SkeletonHierarchy, SkeletonJoint, Track};
use crate::components::{Material, Mesh};
use crate::renderer::Vertex;
use gizmo_math::{Quat, Vec3};
use std::sync::Arc;
use wgpu::util::DeviceExt;

// ============================================================================
//  Public data structures
// ============================================================================

pub struct GltfNodeData {
    pub index: usize,
    pub name: Option<String>,
    /// Index into [`GltfSceneAsset::skeletons`] if this node drives a skin.
    pub skin_index: Option<usize>,
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
    /// (mesh, optional material) per glTF primitive on this node.
    pub primitives: Vec<(Mesh, Option<Material>)>,
    pub children: Vec<GltfNodeData>,
}

pub struct GltfSceneAsset {
    pub roots: Vec<GltfNodeData>,
    pub animations: Vec<AnimationClip>,
    pub skeletons: Vec<SkeletonHierarchy>,
}

// ============================================================================
//  AssetManager impls
// ============================================================================

impl super::AssetManager {
    // ── OBJ ──────────────────────────────────────────────────────────────────

    /// Upload an already-decoded OBJ vertex buffer to the GPU and cache it.
    ///
    /// Called by [`AsyncAssetLoader`](crate::async_assets::AsyncAssetLoader)
    /// after decoding completes on a worker thread.
    pub fn install_obj_mesh(
        &mut self,
        device: &wgpu::Device,
        file_path: &str,
        vertices: Vec<Vertex>,
        _aabb: gizmo_math::Aabb,
    ) -> Mesh {
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("OBJ VBuf: {file_path}")),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let mesh = Mesh::new(
            device,
            Arc::new(vbuf),
            &vertices,
            Vec3::ZERO,
            format!("obj:{file_path}"),
        );
        self.mesh_cache.insert(file_path.to_string(), mesh.clone());
        mesh
    }

    /// Load an OBJ file from disk (or return the cached copy).
    pub fn load_obj(&mut self, device: &wgpu::Device, file_path_or_uuid: &str) -> Mesh {
        let file_path = match self.resolve_path_from_meta_source(file_path_or_uuid) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("[AssetManager] ERROR: {e}");
                return self.loading_placeholder_mesh(device);
            }
        };

        // Prefer UUID as cache key when available.
        let cache_key = self
            .get_uuid(&file_path)
            .map(|id| id.to_string())
            .unwrap_or_else(|| file_path.clone());

        if let Some(cached) = self.mesh_cache.get(&cache_key) {
            return cached.clone();
        }

        let (vertices, aabb) = match decode_obj_vertices_for_async(&file_path) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[AssetManager] OBJ load failed: {file_path} — {e}");
                // Return a valid-but-empty mesh so nothing downstream panics.
                let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Fallback VBuf (not found)"),
                    contents: &[],
                    usage: wgpu::BufferUsages::VERTEX,
                });
                return Mesh::empty(Arc::new(vbuf), format!("obj:missing_{file_path}"));
            }
        };

        self.install_obj_mesh(device, &cache_key, vertices, aabb)
    }

    // ── glTF — top-level entry points ────────────────────────────────────────

    /// Load a glTF scene from disk (or embedded data) and upload it to the GPU.
    ///
    /// The returned [`GltfSceneAsset`] is pure CPU/ECS data; a separate scene
    /// builder is responsible for spawning ECS entities from it.
    pub fn load_gltf_scene(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture_bind_group_layout: &wgpu::BindGroupLayout,
        default_tbind: Arc<wgpu::BindGroup>,
        path_or_uuid: &str,
    ) -> Result<GltfSceneAsset, String> {
        let file_path = self.resolve_path_from_meta_source(path_or_uuid)?;
        let cache_key = self
            .get_uuid(&file_path)
            .map(|id| id.to_string())
            .unwrap_or_else(|| file_path.clone());

        let import_result = if let Some(data) = self.embedded_assets.get(&file_path) {
            gltf::import_slice(data.as_ref())
                .map_err(|e| format!("Embedded glTF read failed ({file_path}): {e}"))
        } else {
            gltf::import(&file_path)
                .map_err(|e| format!("glTF file load failed ({file_path}): {e}"))
        };

        let (document, buffers, images) = import_result?;
        self.load_gltf_from_import(
            device,
            queue,
            texture_bind_group_layout,
            default_tbind,
            &cache_key,
            document,
            buffers,
            images,
        )
    }

    /// Upload a pre-parsed glTF import to the GPU.
    ///
    /// Split from `load_gltf_scene` so that `gltf::import` (which is
    /// CPU-bound and blocks) can be called off the main thread while GPU
    /// upload happens here on the main thread.
    pub fn load_gltf_from_import(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture_bind_group_layout: &wgpu::BindGroupLayout,
        default_tbind: Arc<wgpu::BindGroup>,
        file_path: &str,
        document: gltf::Document,
        buffers: Vec<gltf::buffer::Data>,
        images: Vec<gltf::image::Data>,
    ) -> Result<GltfSceneAsset, String> {
        // ── 1. Textures ───────────────────────────────────────────────────
        let gltf_textures =
            self.upload_gltf_textures(device, queue, texture_bind_group_layout, file_path, &images);

        // ── 2. Materials ──────────────────────────────────────────────────
        let gltf_materials = build_gltf_materials(&document, &gltf_textures, &default_tbind);

        // ── 3. Node tree ──────────────────────────────────────────────────
        let mut roots = Vec::new();
        for scene in document.scenes() {
            for node in scene.nodes() {
                roots.push(self.parse_gltf_node(
                    device,
                    &node,
                    &buffers,
                    &gltf_materials,
                    file_path,
                ));
            }
        }

        // ── 4. Animations ─────────────────────────────────────────────────
        let animations = parse_animations(&document, &buffers);

        // ── 5. Skeletons ──────────────────────────────────────────────────
        // Build a node-index → parent-node-index lookup (used when resolving
        // bone parents and the armature root transform).
        let node_parents: std::collections::HashMap<usize, usize> = document
            .nodes()
            .flat_map(|parent| {
                parent
                    .children()
                    .map(move |child| (child.index(), parent.index()))
            })
            .collect();

        // Build a fast node-index → Node lookup so we avoid O(n) `.nth()`.
        let nodes_by_index: Vec<gltf::Node> = document.nodes().collect();

        let skeletons = parse_skeletons(&document, &buffers, &node_parents, &nodes_by_index);

        Ok(GltfSceneAsset {
            roots,
            animations,
            skeletons,
        })
    }

    // ── glTF — texture upload ────────────────────────────────────────────────

    fn upload_gltf_textures(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture_bind_group_layout: &wgpu::BindGroupLayout,
        file_path: &str,
        images: &[gltf::image::Data],
    ) -> Vec<(Arc<wgpu::BindGroup>, String)> {
        let mut gltf_textures = Vec::with_capacity(images.len());

        for (i, image) in images.iter().enumerate() {
            let (width, height) = (image.width, image.height);

            // Convert every format to RGBA8 for uniform GPU handling.
            let rgba: Vec<u8> = convert_image_to_rgba8(image, i, file_path);

            let texture_size = wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            };

            let texture = device.create_texture(&wgpu::TextureDescriptor {
                size: texture_size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                label: Some(&format!("{file_path}_tex_{i}")),
                view_formats: &[],
            });

            queue.write_texture(
                wgpu::ImageCopyTexture {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &rgba,
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * width),
                    rows_per_image: Some(height),
                },
                texture_size,
            );

            let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
            let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
                address_mode_u: wgpu::AddressMode::Repeat,
                address_mode_v: wgpu::AddressMode::Repeat,
                address_mode_w: wgpu::AddressMode::Repeat,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            });

            let bg = Arc::new(device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&format!("{file_path}_bg_{i}")),
                layout: texture_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            }));

            let tex_source = format!("gltf_tex_{file_path}_{i}");
            self.texture_cache.insert(tex_source.clone(), bg.clone());
            gltf_textures.push((bg, tex_source));
        }

        gltf_textures
    }

    // ── glTF — node parsing ───────────────────────────────────────────────────

    fn parse_gltf_node(
        &mut self,
        device: &wgpu::Device,
        node: &gltf::Node,
        buffers: &[gltf::buffer::Data],
        materials: &[Material],
        file_name: &str,
    ) -> GltfNodeData {
        let (translation, rotation, scale) = node.transform().decomposed();

        let mut primitives = Vec::new();

        if let Some(mesh) = node.mesh() {
            for (prim_i, primitive) in mesh.primitives().enumerate() {
                // Only handle triangles — skip lines, points, strips, etc.
                if primitive.mode() != gltf::mesh::Mode::Triangles {
                    eprintln!(
                        "[GLTF WARN] Skipping non-triangle primitive (mode={:?}) on node '{}'",
                        primitive.mode(),
                        node.name().unwrap_or("<unnamed>"),
                    );
                    continue;
                }

                let reader = primitive.reader(|buf| Some(&buffers[buf.index()]));

                let positions: Vec<[f32; 3]> = reader
                    .read_positions()
                    .map(|it| it.collect())
                    .unwrap_or_default();

                if positions.is_empty() {
                    continue; // nothing to upload
                }

                let supplied_normals: Option<Vec<[f32; 3]>> =
                    reader.read_normals().map(|it| it.collect());

                let tex_coords: Vec<[f32; 2]> = reader
                    .read_tex_coords(0)
                    .map(|it| it.into_f32().collect())
                    .unwrap_or_else(|| vec![[0.0, 0.0]; positions.len()]);

                let joints: Option<Vec<[u16; 4]>> =
                    reader.read_joints(0).map(|it| it.into_u16().collect());
                let weights: Option<Vec<[f32; 4]>> =
                    reader.read_weights(0).map(|it| it.into_f32().collect());

                // Expand indexed geometry into a flat vertex list.
                let mut all_vertices: Vec<Vertex> = Vec::new();
                let mut aabb = gizmo_math::Aabb::empty();

                let make_vertex = |idx: usize| -> Vertex {
                    let pos = positions[idx];

                    // Safe access — attribute arrays may be shorter than positions.
                    let normal = supplied_normals
                        .as_ref()
                        .and_then(|n| n.get(idx).copied())
                        .unwrap_or([0.0, 1.0, 0.0]);
                    let uv = tex_coords.get(idx).copied().unwrap_or([0.0, 0.0]);
                    let j = joints
                        .as_ref()
                        .and_then(|js| js.get(idx))
                        .map(|&[a, b, c, d]| [a as u32, b as u32, c as u32, d as u32])
                        .unwrap_or([0; 4]);
                    let w = weights
                        .as_ref()
                        .and_then(|ws| ws.get(idx))
                        .copied()
                        .unwrap_or([0.0; 4]);

                    Vertex {
                        position: pos,
                        normal,
                        tex_coords: uv,
                        color: [1.0, 1.0, 1.0],
                        joint_indices: j,
                        joint_weights: w,
                    }
                };

                if let Some(indices) = reader.read_indices() {
                    for idx in indices.into_u32() {
                        let i = idx as usize;
                        if i < positions.len() {
                            let pos = positions[i];
                            aabb.extend(Vec3::new(pos[0], pos[1], pos[2]));
                            all_vertices.push(make_vertex(i));
                        }
                    }
                } else {
                    for i in 0..positions.len() {
                        let pos = positions[i];
                        aabb.extend(Vec3::new(pos[0], pos[1], pos[2]));
                        all_vertices.push(make_vertex(i));
                    }
                }

                // Compute flat normals when the file did not supply any.
                // We only do this for triangle lists (guaranteed above).
                if supplied_normals.is_none() {
                    compute_flat_normals(&mut all_vertices);
                }

                let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(&format!("GLTF VBuf: {file_name}_prim{prim_i}")),
                    contents: bytemuck::cast_slice(&all_vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });

                // Use a deterministic cache key that doesn't depend on Debug formatting.
                let mesh_source = format!(
                    "gltf_mesh_{file_name}_{}_p{prim_i}",
                    node.name().unwrap_or("<unnamed>")
                );
                let mesh_comp = Mesh::new(
                    device,
                    Arc::new(vbuf),
                    &all_vertices,
                    Vec3::ZERO,
                    mesh_source.clone(),
                );
                self.mesh_cache.insert(mesh_source, mesh_comp.clone());

                let mat_opt = primitive
                    .material()
                    .index()
                    .and_then(|idx| materials.get(idx).cloned());

                primitives.push((mesh_comp, mat_opt));
            }
        }

        let children = node
            .children()
            .map(|child| self.parse_gltf_node(device, &child, buffers, materials, file_name))
            .collect();

        GltfNodeData {
            index: node.index(),
            name: node.name().map(str::to_owned),
            skin_index: node.skin().map(|s| s.index()),
            translation,
            rotation,
            scale,
            primitives,
            children,
        }
    }
}

// ============================================================================
//  Free helpers — image conversion
// ============================================================================

/// Convert any glTF image format to RGBA8, always producing `width * height * 4` bytes.
fn convert_image_to_rgba8(image: &gltf::image::Data, idx: usize, file_path: &str) -> Vec<u8> {
    let (w, h) = (image.width as usize, image.height as usize);
    let pixel_count = w * h;

    match image.format {
        gltf::image::Format::R8G8B8A8 => {
            // Already in the right format — clone and return.
            // Guard against truncated data so write_texture can't panic.
            let expected = pixel_count * 4;
            if image.pixels.len() >= expected {
                image.pixels[..expected].to_vec()
            } else {
                // Pad with opaque black.
                let mut out = image.pixels.clone();
                out.resize(expected, 255);
                out
            }
        }

        gltf::image::Format::R8G8B8 => {
            // Drop the boundary check: chunks_exact only yields complete 3-byte chunks,
            // silently ignoring a trailing 1 or 2 bytes. A trailing partial pixel is
            // a malformed file; padding it to opaque black is the safest recovery.
            let mut out = Vec::with_capacity(pixel_count * 4);
            for chunk in image.pixels.chunks_exact(3) {
                out.extend_from_slice(&[chunk[0], chunk[1], chunk[2], 255]);
            }
            // Pad if the source was shorter than expected.
            out.resize(pixel_count * 4, 255);
            out
        }

        gltf::image::Format::R8G8 => {
            // Luminance + Alpha: replicate L to R, G, B; keep A.
            let mut out = Vec::with_capacity(pixel_count * 4);
            for chunk in image.pixels.chunks_exact(2) {
                out.extend_from_slice(&[chunk[0], chunk[0], chunk[0], chunk[1]]);
            }
            out.resize(pixel_count * 4, 255);
            out
        }

        gltf::image::Format::R8 => {
            // Single-channel luminance: replicate to RGB, full alpha.
            let mut out = Vec::with_capacity(pixel_count * 4);
            for &lum in &image.pixels {
                out.extend_from_slice(&[lum, lum, lum, 255]);
            }
            out.resize(pixel_count * 4, 255);
            out
        }

        unknown => {
            eprintln!(
                "[GLTF WARN] Unknown pixel format {unknown:?} on image {idx} in '{file_path}'. \
                 Falling back to RGBA8 with clamped copy."
            );
            let expected = pixel_count * 4;
            // Opaque black canvas — copy whatever bytes we have.
            let mut out = vec![0u8; expected];
            // Set alpha channel of every pixel to 255 (opaque).
            for px in 0..pixel_count {
                out[px * 4 + 3] = 255;
            }
            let copy_len = image.pixels.len().min(expected);
            out[..copy_len].copy_from_slice(&image.pixels[..copy_len]);
            out
        }
    }
}

// ============================================================================
//  Free helpers — flat normal generation
// ============================================================================

/// Compute per-triangle flat normals and assign them to each vertex in the
/// triangle.  Vertices must already be in expanded (non-indexed) form and the
/// primitive mode must be `Triangles` (guaranteed by the caller).
fn compute_flat_normals(vertices: &mut [Vertex]) {
    for tri in vertices.chunks_exact_mut(3) {
        let v0 = Vec3::from(tri[0].position);
        let v1 = Vec3::from(tri[1].position);
        let v2 = Vec3::from(tri[2].position);

        let edge1 = v1 - v0;
        let edge2 = v2 - v0;
        let cross = edge1.cross(edge2);

        let normal = if cross.length_squared() > 1e-10 {
            cross.normalize()
        } else {
            Vec3::Y // degenerate triangle → point up
        };

        let n = [normal.x, normal.y, normal.z];
        tri[0].normal = n;
        tri[1].normal = n;
        tri[2].normal = n;
    }
}

// ============================================================================
//  Free helpers — material building
// ============================================================================

fn build_gltf_materials(
    document: &gltf::Document,
    gltf_textures: &[(Arc<wgpu::BindGroup>, String)],
    default_tbind: &Arc<wgpu::BindGroup>,
) -> Vec<Material> {
    document
        .materials()
        .map(|material| {
            let pbr = material.pbr_metallic_roughness();
            let base_color = pbr.base_color_factor();

            let mut mat = pbr
                .base_color_texture()
                .and_then(|ti| gltf_textures.get(ti.texture().source().index()))
                .map(|(bg, src)| {
                    let mut m = Material::new(bg.clone());
                    m.texture_source = Some(src.clone());
                    m
                })
                .unwrap_or_else(|| Material::new(default_tbind.clone()));

            // Some Blender exporters write alpha=0 for opaque materials, making
            // meshes invisible.  Override alpha to 1.0 for opaque alpha modes.
            let alpha = if material.alpha_mode() == gltf::material::AlphaMode::Opaque {
                1.0
            } else {
                base_color[3]
            };

            mat.albedo = gizmo_math::Vec4::new(base_color[0], base_color[1], base_color[2], alpha);
            mat.metallic = pbr.metallic_factor();
            mat.roughness = pbr.roughness_factor();

            mat.is_transparent = false;
            mat.is_double_sided = material.double_sided();

            mat
        })
        .collect()
}

// ============================================================================
//  Free helpers — animation parsing
// ============================================================================

fn parse_animations(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
) -> Vec<AnimationClip> {
    document
        .animations()
        .map(|anim| {
            let mut translations = Vec::new();
            let mut rotations = Vec::new();
            let mut scales = Vec::new();

            for channel in anim.channels() {
                let target_node = channel.target().node().index();
                let target_node_name = channel.target().node().name().map(str::to_owned);
                let reader = channel.reader(|b| Some(&buffers[b.index()]));

                let times: Vec<f32> = match reader.read_inputs() {
                    Some(it) => it.collect(),
                    None => continue,
                };

                let interp = match channel.sampler().interpolation() {
                    gltf::animation::Interpolation::Step => {
                        crate::animation::InterpolationMode::Step
                    }
                    gltf::animation::Interpolation::CubicSpline => {
                        crate::animation::InterpolationMode::CubicSpline
                    }
                    _ => crate::animation::InterpolationMode::Linear,
                };

                let outputs = match reader.read_outputs() {
                    Some(o) => o,
                    None => continue,
                };

                match outputs {
                    gltf::animation::util::ReadOutputs::Translations(tr) => {
                        let keyframes = times
                            .iter()
                            .zip(tr)
                            .map(|(&t, v)| Keyframe {
                                time: t,
                                value: Vec3::new(v[0], v[1], v[2]),
                            })
                            .collect();
                        translations.push(Track {
                            target_node,
                            target_node_name: target_node_name.clone(),
                            interpolation: interp,
                            keyframes,
                        });
                    }
                    gltf::animation::util::ReadOutputs::Rotations(rt) => {
                        let keyframes = times
                            .iter()
                            .zip(rt.into_f32())
                            .map(|(&t, v)| Keyframe {
                                time: t,
                                value: Quat::from_xyzw(v[0], v[1], v[2], v[3]),
                            })
                            .collect();
                        rotations.push(Track {
                            target_node,
                            target_node_name: target_node_name.clone(),
                            interpolation: interp,
                            keyframes,
                        });
                    }
                    gltf::animation::util::ReadOutputs::Scales(sc) => {
                        let keyframes = times
                            .iter()
                            .zip(sc)
                            .map(|(&t, v)| Keyframe {
                                time: t,
                                value: Vec3::new(v[0], v[1], v[2]),
                            })
                            .collect();
                        scales.push(Track {
                            target_node,
                            target_node_name,
                            interpolation: interp,
                            keyframes,
                        });
                    }
                    _ => {} // Morph targets and other outputs are intentionally ignored.
                }
            }

            // Duration = time of the last keyframe across all tracks.
            let d_tr = translations
                .iter()
                .filter_map(|t| t.keyframes.last().map(|k| k.time))
                .fold(0.0f32, f32::max);
            let d_rot = rotations
                .iter()
                .filter_map(|t| t.keyframes.last().map(|k| k.time))
                .fold(0.0f32, f32::max);
            let d_scl = scales
                .iter()
                .filter_map(|t| t.keyframes.last().map(|k| k.time))
                .fold(0.0f32, f32::max);
            let duration = d_tr.max(d_rot).max(d_scl);

            AnimationClip {
                name: anim.name().unwrap_or("unnamed").to_string(),
                duration,
                translations,
                rotations,
                scales,
            }
        })
        .collect()
}

// ============================================================================
//  Free helpers — skeleton parsing
// ============================================================================

fn parse_skeletons(
    document: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    node_parents: &std::collections::HashMap<usize, usize>,
    nodes_by_index: &[gltf::Node],
) -> Vec<SkeletonHierarchy> {
    document
        .skins()
        .map(|skin| {
            let reader = skin.reader(|b| Some(&buffers[b.index()]));

            let identity_mat = [
                [1.0, 0., 0., 0.],
                [0., 1., 0., 0.],
                [0., 0., 1., 0.],
                [0., 0., 0., 1.],
            ];
            let ibm: Vec<[[f32; 4]; 4]> = reader
                .read_inverse_bind_matrices()
                .map(|v| v.collect())
                .unwrap_or_else(|| vec![identity_mat; skin.joints().count()]);

            // Map node_index → bone_index for O(1) parent lookups.
            let node_to_bone: std::collections::HashMap<usize, usize> = skin
                .joints()
                .enumerate()
                .map(|(bone_idx, node)| (node.index(), bone_idx))
                .collect();

            let joints: Vec<SkeletonJoint> = skin
                .joints()
                .enumerate()
                .map(|(bone_idx, joint_node)| {
                    let inverse_bind_matrix = gizmo_math::Mat4::from_cols_array_2d(&ibm[bone_idx]);

                    let parent_index = node_parents
                        .get(&joint_node.index())
                        .and_then(|p| node_to_bone.get(p).copied());

                    let (t, r, s) = joint_node.transform().decomposed();
                    let bind_translation = Vec3::new(t[0], t[1], t[2]);
                    let bind_rotation = Quat::from_array(r);
                    let bind_scale = Vec3::new(s[0], s[1], s[2]);

                    let local_bind_transform = gizmo_math::Mat4::from_translation(bind_translation)
                        * gizmo_math::Mat4::from_quat(bind_rotation)
                        * gizmo_math::Mat4::from_scale(bind_scale);

                    SkeletonJoint {
                        name: joint_node.name().unwrap_or("bone").to_string(),
                        node_index: joint_node.index(),
                        inverse_bind_matrix,
                        parent_index,
                        local_bind_transform,
                        bind_translation,
                        bind_rotation,
                        bind_scale,
                    }
                })
                .collect();

            // Compute the combined transform of all non-joint ancestor nodes
            // (the "armature" transform).  `calculate_global_matrices` relies
            // on this so that joint matrices are identity in the bind pose.
            //
            // We use `nodes_by_index` for O(1) node lookup instead of O(n) `.nth()`.
            let root_transform =
                compute_armature_root_transform(&skin, node_parents, &node_to_bone, nodes_by_index);

            SkeletonHierarchy {
                joints,
                root_transform,
            }
        })
        .collect()
}

/// Walk the parent chain of the first joint upward until we hit a joint or the
/// root, accumulating the transforms of all non-joint ancestors.
fn compute_armature_root_transform(
    skin: &gltf::Skin,
    node_parents: &std::collections::HashMap<usize, usize>,
    node_to_bone: &std::collections::HashMap<usize, usize>,
    nodes_by_index: &[gltf::Node],
) -> gizmo_math::Mat4 {
    let mut root_transform = gizmo_math::Mat4::IDENTITY;

    let first_joint = match skin.joints().next() {
        Some(j) => j,
        None => return root_transform,
    };

    let mut current_idx = first_joint.index();
    let mut ancestor_transforms: Vec<gizmo_math::Mat4> = Vec::new();

    while let Some(&parent_idx) = node_parents.get(&current_idx) {
        // Stop when we reach another bone — its transform is already baked
        // into the skeleton hierarchy.
        if node_to_bone.contains_key(&parent_idx) {
            break;
        }

        if let Some(parent_node) = nodes_by_index.get(parent_idx) {
            let (t, r, s) = parent_node.transform().decomposed();
            let mat = gizmo_math::Mat4::from_translation(Vec3::new(t[0], t[1], t[2]))
                * gizmo_math::Mat4::from_quat(Quat::from_array(r))
                * gizmo_math::Mat4::from_scale(Vec3::new(s[0], s[1], s[2]));
            ancestor_transforms.push(mat);
        }

        current_idx = parent_idx;
    }

    // Apply transforms from root downward (reverse of collection order).
    for mat in ancestor_transforms.into_iter().rev() {
        root_transform *= mat;
    }

    root_transform
}
