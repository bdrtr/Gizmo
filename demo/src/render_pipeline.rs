use gizmo::prelude::*;
use crate::state::GameState;

pub fn execute_render_pipeline(world: &mut World, state: &GameState, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView, renderer: &mut gizmo::renderer::Renderer, _light_time: f32) {
        let aspect = if renderer.size.height > 0 {
            renderer.size.width as f32 / renderer.size.height as f32
        } else {
            1.0
        };

        let mut proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, aspect, 0.1, 2000.0);
        let mut view_mat = Mat4::from_translation(Vec3::ZERO);
        let mut cam_pos = Vec3::ZERO;

        if let (Some(cameras), Some(mut transforms)) = (world.borrow::<Camera>(), world.borrow_mut::<Transform>()) {
            if let (Some(cam), Some(trans)) = (cameras.get(state.player_id), transforms.get(state.player_id)) {
                proj = cam.get_projection(aspect);
                view_mat = cam.get_view(trans.position);
                cam_pos = trans.position;
            }
            // Skybox her zaman Kamerayla aynı yerde durarak sonsuzluk hissi yaratır.
            if let Some(sky_t) = transforms.get_mut(state.skybox_id) {
                sky_t.position = cam_pos;
            }
        }
        
        let view_proj = proj * view_mat;

        // Event: Spawning moved to spawner_update_system.
        // Event: Texture Loading moved to main render loop pass before execute_render_pipeline.

        // --- SKELETAL ANIMATION UPDATE ---
        let delta_time = 1.0 / (state.current_fps.max(1.0));
        
        if let Some(mut q) = world.query_mut_mut::<gizmo::renderer::components::AnimationPlayer, gizmo::renderer::components::Skeleton>() {
            for (_e, anim_player, skeleton) in q.iter_mut() {
                if anim_player.animations.is_empty() { continue; }
                
                let active_idx = anim_player.active_animation.min(anim_player.animations.len() - 1);
                let anim = &anim_player.animations[active_idx];
                
                // Zamanı ilerlet
                anim_player.current_time += delta_time;
                if anim_player.current_time > anim.duration {
                    if anim_player.loop_anim {
                        anim_player.current_time %= anim.duration.max(0.001); // 0 div fix
                    } else {
                        anim_player.current_time = anim.duration;
                    }
                }
                
                let time = anim_player.current_time;
                
                // 1) Local Poses hesapla (Sadece animasyondan gelenleri ez, geri kalanı orijinal local_bind kalsın)
                let hierarchy = &skeleton.hierarchy;
                let mut local_poses = vec![Mat4::IDENTITY; hierarchy.joints.len()];
                
                for (b_idx, joint) in hierarchy.joints.iter().enumerate() {
                    let (mut s, mut r, mut t) = joint.local_bind_transform.to_scale_rotation_translation();
                    
                    if let Some(track) = anim.translations.iter().find(|tr| tr.target_node == joint.node_index) {
                        if let Some(val) = track.get_interpolated(time, |a, b, lerp_t| a.lerp(b, lerp_t)) {
                            t = val;
                        }
                    }
                    
                    if let Some(track) = anim.rotations.iter().find(|tr| tr.target_node == joint.node_index) {
                        if let Some(val) = track.get_interpolated(time, |a, b, lerp_t| a.slerp(b, lerp_t)) {
                            r = Quat::from_xyzw(val.x, val.y, val.z, val.w);
                        }
                    }
                    
                    if let Some(track) = anim.scales.iter().find(|tr| tr.target_node == joint.node_index) {
                        if let Some(val) = track.get_interpolated(time, |a, b, lerp_t| a.lerp(b, lerp_t)) {
                            s = val;
                        }
                    }
                    
                    local_poses[b_idx] = Mat4::from_scale_rotation_translation(s, r, t);
                }

                // 2) Global matrisleri hesapla (Hierarchy)
                let globals = hierarchy.calculate_global_matrices(&local_poses);
                
                // 3) Inverse Bind Matrices ile çarpıp Skeleton'un local_poses alanına yaz (ki shader bilsin)
                skeleton.local_poses.clear();
                for (i, global_mat) in globals.iter().enumerate() {
                    let final_mat = *global_mat * hierarchy.joints[i].inverse_bind_matrix;
                    skeleton.local_poses.push(final_mat);
                }
                
                // 4) GPU'ya gönder! (En faza 64 kemik)
                let mut gpu_data = [[[0.0f32; 4]; 4]; 64];
                for i in 0..skeleton.local_poses.len().min(64) {
                    gpu_data[i] = skeleton.local_poses[i].to_cols_array_2d();
                }
                renderer.queue.write_buffer(&skeleton.buffer, 0, bytemuck::cast_slice(&gpu_data));
            }
        }

        // Işık kaynaklarını topla (Maksimum 10)
        let mut lights_data = [gizmo::renderer::renderer::LightData { position: [0.0; 4], color: [0.0; 4] }; 10];
        let mut num_lights = 0;
        
        if let Some(q) = world.query_ref_ref::<PointLight, Transform>() {
            for (_e, l, t) in q.iter() {
                if num_lights >= 10 { break; }
                lights_data[num_lights as usize] = gizmo::renderer::renderer::LightData {
                    position: [t.position.x, t.position.y, t.position.z, l.intensity],
                    color: [l.color.x, l.color.y, l.color.z, 0.0],
                };
                num_lights += 1;
            }
        }

        // --- Directional Light (Güneş) Taraması ---
        let mut sun_dir = [0.0, -1.0, 0.0, 0.0];
        let mut sun_col = [0.0, 0.0, 0.0, 0.0];
        
        if let Some(q) = world.query_ref_ref::<gizmo::renderer::components::DirectionalLight, Transform>() {
            for (_e, dl, t) in q.iter() {
                if dl.is_sun {
                    // Transform'un rotasyonundan ileri vektörü hesapla (Güneşin baktığı yön)
                    // Standartlara göre ışık '-Z' ye bakar
                    let forward = t.rotation.mul_vec3(Vec3::new(0.0, 0.0, -1.0)).normalize();
                    sun_dir = [forward.x, forward.y, forward.z, 1.0]; // w=1.0: güneş tanımlı
                    sun_col = [dl.color.x, dl.color.y, dl.color.z, dl.intensity];
                    break; // Sadece ilk ana güneşi al
                }
            }
        }

        // Shadow Mapping İçin Dinamik Ana Işık Kamerası Hazırla
        let mut light_view_proj = Mat4::IDENTITY;
        if sun_dir[3] > 0.5 {
            // Dinamik Frustum: Gölge kamerasını oyuncunun (cam_pos) tam üstüne/arkasına kilitleriz.
            let light_direction = Vec3::new(sun_dir[0], sun_dir[1], sun_dir[2]).normalize();
            // Güneşi kameranın uzağına yerleştirip, kameranın baktığı yeri aydınlatmasını sağla
            let light_pos = cam_pos - light_direction * 40.0; 
            
            let light_view = Mat4::look_at_rh(light_pos, cam_pos, Vec3::new(0.0, 1.0, 0.0));
            // Daha sıkı (tight) frustum kullanarak gölge kalitesini artırıyoruz (25 metre)
            let light_proj = Mat4::orthographic_rh(-25.0, 25.0, -25.0, 25.0, 0.1, 150.0);
            light_view_proj = light_proj * light_view;
        } else if num_lights > 0 {
            // Fallback: PointLight taklidi
            let l_pos = Vec3::new(lights_data[0].position[0], lights_data[0].position[1], lights_data[0].position[2]);
            let light_view = Mat4::look_at_rh(l_pos, Vec3::ZERO, Vec3::new(0.0, 1.0, 0.0));
            let light_proj = Mat4::orthographic_rh(-10.0, 10.0, -10.0, 10.0, 0.1, 100.0);
            light_view_proj = light_proj * light_view;
        }

        // Global Uniforms (Her frame sadece 1 kere gönderilir)
        let scene_uniform_data = gizmo::renderer::renderer::SceneUniforms {
            view_proj: view_proj.to_cols_array_2d(),
            camera_pos: [cam_pos.x, cam_pos.y, cam_pos.z, 1.0],
            sun_direction: sun_dir,
            sun_color: sun_col,
            lights: lights_data,
            light_view_proj: light_view_proj.to_cols_array_2d(),
            num_lights,
            _padding: [0; 3],
        };
        renderer.queue.write_buffer(&renderer.global_uniform_buffer, 0, bytemuck::cast_slice(&[scene_uniform_data]));

        // --- BATCHING (INSTANCING) HAZIRLIĞI VE FRUSTUM CULLING ---
        use gizmo::renderer::renderer::InstanceRaw;

        let frustum = gizmo::math::frustum::Frustum::from_matrix(&view_proj);

        struct BatchData {
            vbuf: std::sync::Arc<wgpu::Buffer>,
            vertex_count: u32,
            bind_group: std::sync::Arc<wgpu::BindGroup>,
            skeleton_bg: std::sync::Arc<wgpu::BindGroup>,
            instances: Vec<InstanceRaw>,
        }

        let mut batches: std::collections::HashMap<(*const wgpu::Buffer, *const wgpu::BindGroup, *const wgpu::BindGroup), BatchData> = std::collections::HashMap::new();

        let renderers = world.borrow::<gizmo::renderer::components::MeshRenderer>();
        let skeletons = world.borrow::<gizmo::renderer::components::Skeleton>();
        let lod_groups = world.borrow::<gizmo::renderer::components::LodGroup>();
        
        if let Some(q) = world.query_ref_ref_ref::<Mesh, Transform, Material>() {
            for (e, mesh, trans, mat) in q.iter() {
                // Sadece MeshRenderer tagli olanları çiz:
                if let Some(r) = &renderers {
                    if r.get(e).is_none() { continue; }
                } else { continue; }
                // --- GLOBAL TRANSFORM HESAPLAMA ---
                // transform_hierarchy_system() daha önce tüm hiyerarşiyi t.global_matrix'te çözdüğü için 
                // doğrudan global_matrix'i kullanmamız yeterlidir. Çift çarpım yapmıyoruz!
                let global_model = trans.global_matrix;
                
                let center_mat = Mat4::from_translation(mesh.center_offset);
                let model = global_model * center_mat;

                // Frustum Culling (Görüş açısı dışındakileri atla)
                if e != state.skybox_id && e != state.gizmo_x && e != state.gizmo_y && e != state.gizmo_z {
                    let world_aabb = mesh.bounds.transform(&model);
                    if !frustum.contains_aabb(&world_aabb) {
                        continue;
                    }
                }

                // --- LOD (Level of Detail) SEÇİMİ ---
                // Eğer entity'de LodGroup varsa, kameraya mesafeye göre düşük/yüksek detay mesh seç
                let active_mesh = if let Some(lods) = &lod_groups {
                    if let Some(lod) = lods.get(e) {
                        let world_pos = Vec3::new(model.w_axis.x, model.w_axis.y, model.w_axis.z);
                        let dist = cam_pos.distance(world_pos);
                        lod.select_mesh(dist).unwrap_or(mesh)
                    } else {
                        mesh
                    }
                } else {
                    mesh
                };

                let instance_data = InstanceRaw {
                    model: model.to_cols_array_2d(),
                    albedo_color: [mat.albedo.x, mat.albedo.y, mat.albedo.z, mat.albedo.w],
                    roughness: mat.roughness,
                    metallic: mat.metallic,
                    unlit: mat.unlit,
                    _padding: 0.0,
                };

                // --- SKELETON (KEMİK) ARAMASI ---
                // Yalnızca child meshin değil, atalarından (Root) herhangi birisinde Skeleton var mı diye tırman:
                let mut skel_bg = renderer.dummy_skeleton_bind_group.clone();
                if let Some(skels) = &skeletons {
                    if let Some(s) = skels.get(e) {
                         skel_bg = s.bind_group.clone();
                    } else if let Some(parents) = world.borrow::<Parent>() {
                         let mut curr = e;
                         while let Some(p) = parents.get(curr) {
                             if let Some(s) = skels.get(p.0) {
                                 skel_bg = s.bind_group.clone();
                                 break;
                             }
                             curr = p.0;
                         }
                    }
                }

                let vbuf_ptr = std::sync::Arc::as_ptr(&active_mesh.vbuf);
                let bg_ptr = std::sync::Arc::as_ptr(&mat.bind_group);
                let skel_ptr = std::sync::Arc::as_ptr(&skel_bg);

                let batch = batches.entry((vbuf_ptr, bg_ptr, skel_ptr)).or_insert_with(|| BatchData {
                    vbuf: active_mesh.vbuf.clone(),
                    vertex_count: active_mesh.vertex_count,
                    bind_group: mat.bind_group.clone(),
                    skeleton_bg: skel_bg,
                    instances: Vec::new(),
                });
                
                batch.instances.push(instance_data);
            }
        }

        // Batch'ler için GPU tarafında geçici instancing buffer'ı oluştur
        let mut gpu_batches = Vec::new();
        use wgpu::util::DeviceExt;
        for (_, batch) in batches {
            let instance_buf = renderer.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Instance Buffer"),
                contents: bytemuck::cast_slice(&batch.instances),
                usage: wgpu::BufferUsages::VERTEX,
            });
            gpu_batches.push((batch, instance_buf));
        }

        // --- 1. GÖLGE PASS (Shadow Pass) ---
        {
            let mut shadow_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Shadow Pass"),
                color_attachments: &[], // Shadow pass sadece Depth'e çizer
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &renderer.shadow_texture_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            shadow_pass.set_pipeline(&renderer.shadow_pipeline);

            // Tıpkı main render gibi gruplanmış nesneleri tek draw çağrısıyla bas
            for (batch, instance_buf) in &gpu_batches {
                shadow_pass.set_bind_group(0, &renderer.global_bind_group, &[]);
                shadow_pass.set_bind_group(1, &batch.skeleton_bg, &[]);
                shadow_pass.set_vertex_buffer(0, batch.vbuf.slice(..));
                shadow_pass.set_vertex_buffer(1, instance_buf.slice(..));
                shadow_pass.draw(0..batch.vertex_count, 0..batch.instances.len() as u32);
            }
        }

        // --- 2. ANA RENDER PASS (HDR Offscreen Texture'a çiz) ---
        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Main Render Pass (HDR)"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &renderer.hdr_texture_view, // Artık ekran yerine HDR texture'a çiziyoruz!
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.1, g: 0.15, b: 0.20, a: 1.0 }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &renderer.depth_texture_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            render_pass.set_pipeline(&renderer.render_pipeline);

            for (batch, instance_buf) in &gpu_batches {
                render_pass.set_bind_group(0, &renderer.global_bind_group, &[]);
                render_pass.set_bind_group(1, &batch.bind_group, &[]);
                render_pass.set_bind_group(2, &renderer.shadow_bind_group, &[]);
                render_pass.set_bind_group(3, &batch.skeleton_bg, &[]);
                render_pass.set_vertex_buffer(0, batch.vbuf.slice(..));
                render_pass.set_vertex_buffer(1, instance_buf.slice(..));
                render_pass.draw(0..batch.vertex_count, 0..batch.instances.len() as u32);
            }
        }

        // --- 3. POST-PROCESSING (Bloom + Tone Mapping → Ekrana Yaz) ---
        renderer.run_post_processing(encoder, view);

}
