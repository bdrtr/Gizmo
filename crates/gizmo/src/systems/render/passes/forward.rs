use super::super::*;

// Forward pipeline bind-group indeksleri hedefe göre değişir: tarayıcı WebGPU
// maxBindGroups=4 olduğundan web şemasında shadow grubu atılır ve skeleton /
// instance bir kayar (bkz. gizmo_renderer::pipeline shaders.rs `load_shader_web`).
//   Native: 0=global 1=texture 2=shadow 3=skeleton 4=instance
//   WASM:   0=global 1=texture 2=skeleton 3=instance
#[cfg(not(target_arch = "wasm32"))]
const BG_SKELETON: u32 = 3;
#[cfg(not(target_arch = "wasm32"))]
const BG_INSTANCE: u32 = 4;
#[cfg(target_arch = "wasm32")]
const BG_SKELETON: u32 = 2;
#[cfg(target_arch = "wasm32")]
const BG_INSTANCE: u32 = 3;

pub fn record_forward_and_fluid(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &Renderer,
    world: &World,
    draw_items: &[DrawItem],
    uploaded_instances: u32,
    particle_lod: f32,
    fluid_lod: f32,
    // Inverse of the (unjittered) camera view-projection — the volumetric smoke raymarch
    // reconstructs rays with this instead of the buggy WGSL inverse_mat4.
    inv_view_proj: [[f32; 4]; 4],
) {
    // ── Forward pass (unlit / skybox / GPU subsystems; PBR skipped if deferred) ──
    {
        let hdr_load = if renderer.deferred.is_some() {
            wgpu::LoadOp::Load
        } else {
            wgpu::LoadOp::Clear(wgpu::Color {
                r: 0.4,
                g: 0.6,
                b: 0.9,
                a: 1.0,
            })
        };
        let depth_load = if renderer.deferred.is_some() {
            wgpu::LoadOp::Load
        } else {
            wgpu::LoadOp::Clear(1.0)
        };
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Default Engine Render Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &renderer.post.hdr_texture_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: hdr_load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &renderer.depth_texture_view,
                depth_ops: Some(wgpu::Operations {
                    load: depth_load,
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        render_pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
        // Web şemasında shadow grubu pipeline layout'unda yok (4-grup limiti).
        #[cfg(not(target_arch = "wasm32"))]
        render_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
        render_pass.set_bind_group(BG_INSTANCE, &renderer.scene.instance_bind_group, &[]);

        let show_wireframes = world.get_resource::<WireframeConfig>().map(|c| c.global).unwrap_or(false);

        for item in draw_items {
            let mut draw_solid = false;
            let draw_wire = show_wireframes && !item.is_skybox;

            if item.is_skybox || item.unlit || renderer.deferred.is_none() || item.is_transparent {
                draw_solid = true;
            }

            // Main pass: yalnız kamera-görünür instance'lar. camera_count==0 ise
            // (tüm batch shadow-only / kamera-dışı) aralık BOŞ olur — 0-instance
            // draw'ı hem boşa komut, hem tarayıcıda "instance count of 0 is
            // unusual" uyarısı üretir → tamamen atla.
            let inst_start = item.first_instance;
            let inst_end = (item.first_instance + item.camera_count)
                .min(uploaded_instances)
                .max(item.first_instance);
            if inst_end <= inst_start {
                continue;
            }

            let skel_bg = item
                .skeleton_bind_group
                .as_ref()
                .unwrap_or(&renderer.scene.dummy_skeleton_bind_group);

            if draw_solid {
                let pipeline = if item.is_skybox {
                    &renderer.scene.sky_pipeline
                } else if item.unlit {
                    &renderer.scene.unlit_pipeline
                } else if item.is_transparent {
                    &renderer.scene.transparent_pipeline
                } else {
                    &renderer.scene.render_pipeline
                };
                render_pass.set_pipeline(pipeline);
                render_pass.set_bind_group(1, &*item.bind_group, &[]);
                render_pass.set_bind_group(BG_SKELETON, skel_bg.as_ref(), &[]);
                render_pass.set_vertex_buffer(0, item.vbuf.slice(..));
                render_pass.draw(0..item.vertex_count, inst_start..inst_end);
            }

            if draw_wire {
                render_pass.set_pipeline(&renderer.scene.wireframe_pipeline);
                render_pass.set_bind_group(1, &*item.bind_group, &[]);
                render_pass.set_bind_group(BG_SKELETON, skel_bg.as_ref(), &[]);
                render_pass.set_vertex_buffer(0, item.vbuf.slice(..));
                render_pass.draw(0..item.vertex_count, inst_start..inst_end);
            }
        }

        // Draw GPU Physics Spheres!
        if let Some(physics) = &renderer.gpu_physics {
            physics.render_pass(&mut render_pass, &renderer.scene.global_bind_group);
            physics.debug_render_pass(&mut render_pass, &renderer.scene.global_bind_group);
        }

        // Draw SPH fluid
        if let Some(fluid) = &renderer.gpu_fluid {
            fluid.render_pass(&mut render_pass, &renderer.scene.global_bind_group);
        }

        // (GPU Particles artık AYRI bir pass'te — aşağıda — soft-particle derinlik örneklemesi için.)
    }

    // ── GPU Particles (SOFT PARTICLES) ──────────────────────────────────────────
    // Forward pass'ten AYRI çizilir: parçacık FS'i sahne DERİNLİĞİNİ örneklemeli (geometriye
    // sert girmesin), ama bir doku aynı pass'te hem depth-attachment hem sampled olamaz.
    // Bu pass'te depth ATTACHMENT YOK → depth sampled bağlanır; occlusion + yumuşak-kaybolma
    // FS'te sahne derinliğinden manuel yapılır. Renk hedefi HDR (Load, önceki sonucun üstüne).
    if let Some(particles) = &renderer.gpu_particles {
        let active_parts = (particles.max_particles as f32 * particle_lod) as u32;
        if active_parts > 0 {
            let depth_bg =
                particles.create_depth_bind_group(&renderer.device, &renderer.depth_texture_view);
            let mut ppass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Particle Soft Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &renderer.post.hdr_texture_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            particles.render_pass(
                &mut ppass,
                &renderer.scene.global_bind_group,
                &depth_bg,
                active_parts,
            );
        }
    }

    if let Some(fluid) = &renderer.gpu_fluid {
        let active_fluid = (fluid.num_particles as f32 * fluid_lod) as u32;
        fluid.render_ssfr(
            encoder,
            &renderer.post.hdr_texture,
            &renderer.post.hdr_texture_view,
            &renderer.depth_texture_view,
            &renderer.scene.global_bind_group,
            active_fluid,
        );
    }

    // ── Volumetrik duman (T6): sahnenin üstüne HDR'ye raymarch (post-process ÖNCESİ) ──
    if let Some(smoke) = &renderer.smoke {
        let (time, dt) = world
            .get_resource::<gizmo_core::time::Time>()
            .map(|t| (t.elapsed() as f32, t.dt()))
            .unwrap_or((0.0, 1.0 / 60.0));
        smoke.render(
            encoder,
            &renderer.device,
            &renderer.queue,
            &renderer.scene.global_bind_group,
            &renderer.post.hdr_texture_view,
            &renderer.depth_texture_view,
            time,
            dt,
            inv_view_proj,
        );
    }
}
