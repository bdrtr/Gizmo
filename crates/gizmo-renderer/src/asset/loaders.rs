use crate::components::{Material, Mesh};
use crate::animation::{AnimationClip, SkeletonHierarchy, SkeletonJoint, Track, Keyframe};
use crate::renderer::Vertex;
use gizmo_math::{Vec3, Quat};
use std::sync::Arc;
use wgpu::util::DeviceExt;
use super::decode_obj_vertices_for_async;

impl super::AssetManager {
    /// GPU'ya OBJ vertex verisini yazar ve önbelleğe koyar ([`AsyncAssetLoader`](crate::async_assets::AsyncAssetLoader) tamamlanınca).
    pub fn install_obj_mesh(
        &mut self,
        device: &wgpu::Device,
        file_path: &str,
        vertices: Vec<Vertex>,
        aabb: gizmo_math::Aabb,
    ) -> Mesh {
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("Obj VBuf: {}", file_path)),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let mesh = Mesh::new(
            Arc::new(vbuf),
            vertices.len() as u32,
            Vec3::ZERO,
            format!("obj:{}", file_path),
            aabb,
        );
        self.mesh_cache.insert(file_path.to_string(), mesh.clone());
        mesh
    }

    /// Bir .obj dosyasını diskten okur ve Mesh ECS bileşenine dönüştürür.
    /// Daha önce okunmuşsa, RAM ve VRAM tüketimini önlemek için önbellekten direkt kopya döndürür.
    pub fn load_obj(&mut self, device: &wgpu::Device, file_path_or_uuid: &str) -> Mesh {
        let file_path = self.resolve_path_from_meta_source(file_path_or_uuid);
        let id_str = if let Some(id) = self.get_uuid(&file_path) {
            id.to_string()
        } else {
            file_path.clone()
        };

        if let Some(cached) = self.mesh_cache.get(&id_str) {
            return cached.clone();
        }

        let (vertices, aabb) = match decode_obj_vertices_for_async(&file_path) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("[ERROR] AssetManager: OBJ yuklenirken hata! {} ({})", file_path, e);
                // Çökmek yerine WGPU tarafında sorun çıkarmayacak boş (0 vertex) bir Mesh dönüyoruz
                let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Fallback VBuf (Not Found)"),
                    contents: &[],
                    usage: wgpu::BufferUsages::VERTEX,
                });
                let mesh = Mesh::new(
                    Arc::new(vbuf),
                    0,
                    Vec3::ZERO,
                    format!("obj:missing_{}", file_path),
                    gizmo_math::Aabb::empty(),
                );
                return mesh;
            }
        };
        self.install_obj_mesh(device, &id_str, vertices, aabb)
    }
    /// (Bu veri yapısı daha sonra motorun diğer parçaları (örn: scene builder) tarafından ECS Entity'lerine basılır)
    pub fn load_gltf_scene(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture_bind_group_layout: &wgpu::BindGroupLayout,
        default_tbind: Arc<wgpu::BindGroup>,
        path_or_uuid: &str,
    ) -> Result<GltfSceneAsset, String> {
        let file_path = self.resolve_path_from_meta_source(path_or_uuid);
        let id_str = if let Some(id) = self.get_uuid(&file_path) {
            id.to_string()
        } else {
            file_path.clone()
        };

        let (document, buffers, images) = gltf::import(&file_path)
            .map_err(|e| format!("GLTF dosyasi yuklenemedi ({}). Hata: {}", file_path, e))?;
        self.load_gltf_from_import(
            device,
            queue,
            texture_bind_group_layout,
            default_tbind,
            &id_str,
            document,
            buffers,
            images,
        )
    }

    /// `gltf::import` ana iş parçacığı dışında çalıştırıldıktan sonra GPU yüklemesi için.
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
        // --- 1. RESİMLERİ TEXTURE & BINDGROUP YAPMA ---
        let mut gltf_textures = Vec::new();

        for (i, image) in images.iter().enumerate() {
            let (width, height) = (image.width, image.height);
            let texture_size = wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            };

            let (img_data, format, bytes_per_row) = match image.format {
                gltf::image::Format::R8G8B8A8 => (
                    image.pixels.clone(),
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                    4 * width,
                ),
                gltf::image::Format::R8G8B8 => {
                    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
                    for chunk in image.pixels.chunks_exact(3) {
                        rgba.extend_from_slice(&[chunk[0], chunk[1], chunk[2], 255]);
                    }
                    (rgba, wgpu::TextureFormat::Rgba8UnormSrgb, 4 * width)
                }
                gltf::image::Format::R8G8 => {
                    // Luminance + Alpha converts to R=Lum, G=Lum, B=Lum, A=Alpha
                    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
                    for chunk in image.pixels.chunks_exact(2) {
                        rgba.extend_from_slice(&[chunk[0], chunk[0], chunk[0], chunk[1]]);
                    }
                    (rgba, wgpu::TextureFormat::Rgba8UnormSrgb, 4 * width)
                }
                gltf::image::Format::R8 => {
                    // Luminance converts to R=Lum, G=Lum, B=Lum, A=255
                    let mut rgba = Vec::with_capacity((width * height * 4) as usize);
                    for &lum in &image.pixels {
                        rgba.extend_from_slice(&[lum, lum, lum, 255]);
                    }
                    (rgba, wgpu::TextureFormat::Rgba8UnormSrgb, 4 * width)
                }
                _ => {
                    eprintln!("[GLTF WARN] Bilinmeyen piksel formatı (image idx={}), RGBA8 fallback kullanılıyor. Boyut: {}x{}, Pixel len: {}",
                        i, width, height, image.pixels.len());
                    // Fallback to RGBA8 padding if length doesn't match standard
                    let mut rgba = vec![255; (width * height * 4) as usize];
                    // At least prevent WGPU out-of-bounds panic
                    let copy_len = image.pixels.len().min(rgba.len());
                    rgba[..copy_len].copy_from_slice(&image.pixels[..copy_len]);
                    (rgba, wgpu::TextureFormat::Rgba8UnormSrgb, 4 * width)
                }
            };

            let texture = device.create_texture(&wgpu::TextureDescriptor {
                size: texture_size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                label: Some(&format!("{}_tex_{}", file_path, i)),
                view_formats: &[],
            });

            queue.write_texture(
                wgpu::ImageCopyTexture {
                    texture: &texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &img_data,
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
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
                label: Some(&format!("{}_bg_{}", file_path, i)),
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
            
            let tex_source = format!("gltf_tex_{}_{}", file_path, i);
            self.texture_cache.insert(tex_source.clone(), bg.clone());
            gltf_textures.push((bg, tex_source));
        }

        // --- 2. MATERYALLERİ OLUŞTURMA ---
        let mut gltf_materials = Vec::new();
        for material in document.materials() {
            let pbr = material.pbr_metallic_roughness();
            let base_color = pbr.base_color_factor();

            let mut mat = if let Some(tex_info) = pbr.base_color_texture() {
                let tex_idx = tex_info.texture().source().index();
                if let Some((bg, src)) = gltf_textures.get(tex_idx) {
                    let mut m = Material::new(bg.clone());
                    m.texture_source = Some(src.clone());
                    m
                } else {
                    Material::new(default_tbind.clone())
                }
            } else {
                Material::new(default_tbind.clone())
            };
            mat.albedo =
                gizmo_math::Vec4::new(base_color[0], base_color[1], base_color[2], base_color[3]);
            mat.metallic = pbr.metallic_factor(); // Araba kaportası gibi yansıyan nesneler simsiyah kalmasın diye kısıtlamayı kaldırdık, çünkü artık shader'da Fake IBL var
            mat.roughness = pbr.roughness_factor(); 
                                                             // Varsayılan: PBR açık (unlit=0.0). GLTF modelleri artık ışıklandırma alacak.
            mat.unlit = 0.0;

            if material.alpha_mode() == gltf::material::AlphaMode::Blend
                || material.alpha_mode() == gltf::material::AlphaMode::Mask
            {
                mat.is_transparent = true;
            }
            if material.double_sided() {
                mat.is_double_sided = true;
            }

            gltf_materials.push(mat);
        }

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

        let mut animations = Vec::new();
        for anim in document.animations() {
            let mut transl = Vec::new();
            let mut rot = Vec::new();
            let mut scl = Vec::new();

            for channel in anim.channels() {
                let target_node = channel.target().node().index();
                let reader = channel.reader(|b| Some(&buffers[b.index()]));

                if let Some(inputs) = reader.read_inputs() {
                    let times: Vec<f32> = inputs.collect();

                        let interp_mode = match channel.sampler().interpolation() {
                            gltf::animation::Interpolation::Step => crate::animation::InterpolationMode::Step,
                            gltf::animation::Interpolation::CubicSpline => crate::animation::InterpolationMode::CubicSpline,
                            _ => crate::animation::InterpolationMode::Linear,
                        };

                        if let Some(outputs) = reader.read_outputs() {
                            match outputs {
                                gltf::animation::util::ReadOutputs::Translations(tr) => {
                                    let mut kfs = Vec::new();
                                    for (time, val) in times.iter().zip(tr) {
                                        kfs.push(Keyframe {
                                            time: *time,
                                            value: Vec3::new(val[0], val[1], val[2]),
                                        });
                                    }
                                    transl.push(Track {
                                        target_node,
                                        interpolation: interp_mode,
                                        keyframes: kfs,
                                    });
                                }
                            gltf::animation::util::ReadOutputs::Rotations(rt) => {
                                let mut kfs = Vec::new();
                                for (time, val) in times.iter().zip(rt.into_f32()) {
                                    kfs.push(Keyframe {
                                        time: *time,
                                        value: Quat::from_xyzw(val[0], val[1], val[2], val[3]),
                                    });
                                }
                                    rot.push(Track {
                                        target_node,
                                        interpolation: interp_mode,
                                        keyframes: kfs,
                                    });
                            }
                            gltf::animation::util::ReadOutputs::Scales(sc) => {
                                let mut kfs = Vec::new();
                                for (time, val) in times.iter().zip(sc) {
                                    kfs.push(Keyframe {
                                        time: *time,
                                        value: Vec3::new(val[0], val[1], val[2]),
                                    });
                                }
                                    scl.push(Track {
                                        target_node,
                                        interpolation: interp_mode,
                                        keyframes: kfs,
                                    });
                            }
                            _ => {} // Morph targets vb. goz ardi edildi
                        }
                    }
                }
            }

            animations.push(AnimationClip {
                name: anim.name().unwrap_or("unnamed_anim").to_string(),
                duration: 0.0, // Hesaplamamiz lazim (track'lerin son keyframe time'larinin max'i)
                translations: transl,
                rotations: rot,
                scales: scl,
            });
        }

        // Sure hesaplama sonradan yapilabilir
        for anim in &mut animations {
            let mut max_t = 0.0f32;
            for t in &anim.translations {
                if let Some(k) = t.keyframes.last() {
                    max_t = max_t.max(k.time);
                }
            }
            for t in &anim.rotations {
                if let Some(k) = t.keyframes.last() {
                    max_t = max_t.max(k.time);
                }
            }
            for t in &anim.scales {
                if let Some(k) = t.keyframes.last() {
                    max_t = max_t.max(k.time);
                }
            }
            anim.duration = max_t;
        }

        let mut node_parents = std::collections::HashMap::new();
        for node in document.nodes() {
            for child in node.children() {
                node_parents.insert(child.index(), node.index());
            }
        }

        let mut skeletons = Vec::new();
        for skin in document.skins() {
            let reader = skin.reader(|b| Some(&buffers[b.index()]));
            let ibm: Vec<[[f32; 4]; 4]> = reader
                .read_inverse_bind_matrices()
                .map(|v| v.collect())
                .unwrap_or_else(|| {
                    vec![
                        [
                            [1.0, 0., 0., 0.],
                            [0., 1., 0., 0.],
                            [0., 0., 1., 0.],
                            [0., 0., 0., 1.]
                        ];
                        skin.joints().count()
                    ]
                });

            let mut node_to_bone = std::collections::HashMap::new();
            for (bone_idx, node) in skin.joints().enumerate() {
                node_to_bone.insert(node.index(), bone_idx);
            }

            let mut joints = Vec::new();
            for (bone_idx, joint_node) in skin.joints().enumerate() {
                let inverse_bind_matrix = gizmo_math::Mat4::from_cols_array_2d(&ibm[bone_idx]);

                let parent_index = node_parents
                    .get(&joint_node.index())
                    .and_then(|p| node_to_bone.get(p).copied());

                let (t, r, s) = joint_node.transform().decomposed();
                let loc_t = gizmo_math::Mat4::from_translation(Vec3::new(t[0], t[1], t[2]));
                let loc_r = gizmo_math::Mat4::from_quat(Quat::from_array(r));
                let loc_s = gizmo_math::Mat4::from_scale(Vec3::new(s[0], s[1], s[2]));
                let local_bind_transform = loc_t * loc_r * loc_s;

                joints.push(SkeletonJoint {
                    name: joint_node.name().unwrap_or("bone").to_string(),
                    node_index: joint_node.index(),
                    inverse_bind_matrix,
                    parent_index,
                    local_bind_transform,
                });
            }

            skeletons.push(SkeletonHierarchy { joints });
        }

        Ok(GltfSceneAsset {
            roots,
            animations,
            skeletons,
        })
    }

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
        if let Some(_mesh) = node.mesh() {
            for (prim_i, primitive) in node.mesh().unwrap().primitives().enumerate() {
                let reader = primitive.reader(|buffer| Some(&buffers[buffer.index()]));

                let positions = reader
                    .read_positions()
                    .map(|v| v.collect::<Vec<_>>())
                    .unwrap_or_default();
                let normals = reader
                    .read_normals()
                    .map(|v| v.collect::<Vec<_>>())
                    .unwrap_or_else(|| vec![[0.0, 1.0, 0.0]; positions.len()]);
                let tex_coords = reader
                    .read_tex_coords(0)
                    .map(|v| v.into_f32().collect::<Vec<_>>())
                    .unwrap_or_else(|| vec![[0.0, 0.0]; positions.len()]);

                let joints = reader
                    .read_joints(0)
                    .map(|v| v.into_u16().collect::<Vec<_>>());
                let weights = reader
                    .read_weights(0)
                    .map(|v| v.into_f32().collect::<Vec<_>>());

                let get_vertex = |i: usize, pos: [f32; 3]| -> Vertex {
                    let j = if let Some(ref js) = joints {
                        if i < js.len() {
                            [
                                js[i][0] as u32,
                                js[i][1] as u32,
                                js[i][2] as u32,
                                js[i][3] as u32,
                            ]
                        } else {
                            [0; 4]
                        }
                    } else {
                        [0; 4]
                    };

                    let w = if let Some(ref ws) = weights {
                        if i < ws.len() {
                            ws[i]
                        } else {
                            [0.0; 4]
                        }
                    } else {
                        [0.0; 4]
                    };

                    Vertex {
                        position: pos,
                        normal: normals[i],
                        tex_coords: tex_coords[i],
                        color: [1.0, 1.0, 1.0],
                        joint_indices: j,
                        joint_weights: w,
                    }
                };

                let mut all_vertices = Vec::new();
                let mut aabb = gizmo_math::Aabb::empty();

                if let Some(indices) = reader.read_indices() {
                    let indices_u32: Vec<u32> = indices.into_u32().collect();
                    for idx in indices_u32 {
                        let i = idx as usize;
                        if i < positions.len() {
                            let pos = positions[i];
                            aabb.extend(Vec3::new(pos[0], pos[1], pos[2]));
                            all_vertices.push(get_vertex(i, pos));
                        }
                    }
                } else {
                    for i in 0..positions.len() {
                        let pos = positions[i];
                        aabb.extend(Vec3::new(pos[0], pos[1], pos[2]));
                        all_vertices.push(get_vertex(i, pos));
                    }
                }

                // EĞER GLTF DOSYASINDA NORMALLER YOKSA VEYA BOZUKSA (HEPSİ [0,1,0] İSE)
                // Kötü gözükmemesi için Flat Normalleri kendimiz hesaplayalım.
                let has_real_normals = reader.read_normals().is_some();
                if !has_real_normals {
                    for chunk in all_vertices.chunks_exact_mut(3) {
                        let p0 = chunk[0].position;
                        let p1 = chunk[1].position;
                        let p2 = chunk[2].position;
                        let v0 = Vec3::new(p0[0], p0[1], p0[2]);
                        let v1 = Vec3::new(p1[0], p1[1], p1[2]);
                        let v2 = Vec3::new(p2[0], p2[1], p2[2]);

                        let norm = (v1 - v0).cross(v2 - v0);
                        let final_norm = if norm.length_squared() > 1e-6 {
                            norm.normalize()
                        } else {
                            Vec3::new(0.0, 1.0, 0.0)
                        };
                        let n_arr = [final_norm.x, final_norm.y, final_norm.z];

                        chunk[0].normal = n_arr;
                        chunk[1].normal = n_arr;
                        chunk[2].normal = n_arr;
                    }
                }

                let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(&format!("GLTF VBuf: {}_prim{}", file_name, prim_i)),
                    contents: bytemuck::cast_slice(&all_vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });

                let mesh_source = format!("gltf_mesh_{}_{:?}_p{}", file_name, node.name(), prim_i);
                let mesh_comp = Mesh::new(
                    Arc::new(vbuf),
                    all_vertices.len() as u32,
                    Vec3::ZERO,
                    mesh_source.clone(),
                    aabb,
                );

                self.mesh_cache.insert(mesh_source, mesh_comp.clone());


                let mat_opt = primitive
                    .material()
                    .index()
                    .and_then(|idx| materials.get(idx).cloned());
                primitives.push((mesh_comp, mat_opt));
            }
        }

        let mut children = Vec::new();
        for child in node.children() {
            children.push(self.parse_gltf_node(device, &child, buffers, materials, file_name));
        }

        GltfNodeData {
            index: node.index(),
            name: node.name().map(|n| n.to_string()),
            translation,
            rotation,
            scale,
            primitives,
            children,
        }
    }
}

pub struct GltfNodeData {
    pub index: usize,
    pub name: Option<String>,
    pub translation: [f32; 3],
    pub rotation: [f32; 4],
    pub scale: [f32; 3],
    pub primitives: Vec<(Mesh, Option<Material>)>,
    pub children: Vec<GltfNodeData>,
}

pub struct GltfSceneAsset {
    pub roots: Vec<GltfNodeData>,
    pub animations: Vec<AnimationClip>,
    pub skeletons: Vec<SkeletonHierarchy>,
}

