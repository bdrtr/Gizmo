use super::*;

// default_render_pass'ten çıkarılan render geçişleri (Tier 3 round-2: mega-fn
// bölmesi). Hepsi yan-etki-only: encoder'a komut kaydeder, çıktı döndürmez.

pub(super) fn record_shadow_passes(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &Renderer,
    draw_items: &[DrawItem],
    uploaded_instances: u32,
) {
    // CSM shadow passes — one depth-only pass per cascade.
    for i in 0..crate::renderer::CASCADE_COUNT {
        let mut shadow_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Shadow Pass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &renderer.scene.shadow_cascade_layer_views[i],
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        shadow_pass.set_pipeline(&renderer.scene.shadow_pipeline);
        shadow_pass.set_bind_group(0, &renderer.scene.shadow_pass_bind_groups[i], &[]);
        shadow_pass.set_bind_group(2, &renderer.scene.instance_bind_group, &[]);
        for item in draw_items {
            if item.unlit || item.is_transparent {
                continue;
            }
            let skel_bg = item
                .skeleton_bind_group
                .as_ref()
                .unwrap_or(&renderer.scene.dummy_skeleton_bind_group);
            shadow_pass.set_bind_group(1, skel_bg.as_ref(), &[]);
            shadow_pass.set_vertex_buffer(0, item.vbuf.slice(..));
            shadow_pass.draw(
                0..item.vertex_count,
                // Shadow passes draw the FULL range (camera-visible + off-screen casters),
                // clamped to what was uploaded.
                item.first_instance
                    ..(item.first_instance + item.instance_count)
                        .min(uploaded_instances)
                        .max(item.first_instance),
            );
        }
    }

    // Point Light Shadow Passes — 6 faces
    for i in 0..6 {
        let mut shadow_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Point Shadow Pass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &renderer.scene.point_shadow_face_views[i],
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        // We reuse the directional shadow pipeline because it only does position transformation
        shadow_pass.set_pipeline(&renderer.scene.shadow_pipeline);
        shadow_pass.set_bind_group(0, &renderer.scene.point_shadow_pass_bind_groups[i], &[]);
        shadow_pass.set_bind_group(2, &renderer.scene.instance_bind_group, &[]);
        for item in draw_items {
            if item.unlit || item.is_transparent { continue; }
            let skel_bg = item
                .skeleton_bind_group
                .as_ref()
                .unwrap_or(&renderer.scene.dummy_skeleton_bind_group);
            shadow_pass.set_bind_group(1, skel_bg.as_ref(), &[]);
            shadow_pass.set_vertex_buffer(0, item.vbuf.slice(..));
            shadow_pass.draw(
                0..item.vertex_count,
                // Shadow passes draw the FULL range (camera-visible + off-screen casters),
                // clamped to what was uploaded.
                item.first_instance
                    ..(item.first_instance + item.instance_count)
                        .min(uploaded_instances)
                        .max(item.first_instance),
            );
        }
    }

}

pub(super) fn record_deferred_geometry(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &Renderer,
    world: &World,
    draw_items: &[DrawItem],
    uploaded_instances: u32,
) {

    // ── Z-Prepass (Depth Only) ────────────────────────────────────────────────
    if let Some(ref def) = renderer.deferred {
        let mut z_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Z-Prepass"),
            color_attachments: &[], // No color targets
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
            multiview_mask: None,
        });
        z_pass.set_pipeline(&def.z_prepass_pipeline);
        z_pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
        z_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
        z_pass.set_bind_group(4, &renderer.scene.instance_bind_group, &[]);
        for item in draw_items {
            if item.unlit || item.is_skybox || item.is_transparent {
                continue;
            }
            let skel_bg = item
                .skeleton_bind_group
                .as_ref()
                .unwrap_or(&renderer.scene.dummy_skeleton_bind_group);
            z_pass.set_bind_group(3, skel_bg.as_ref(), &[]);
            z_pass.set_bind_group(1, &*item.bind_group, &[]);
            z_pass.set_vertex_buffer(0, item.vbuf.slice(..));
            z_pass.draw(
                0..item.vertex_count,
                // Main pass: camera-visible instances only.
                item.first_instance
                    ..(item.first_instance + item.camera_count)
                        .min(uploaded_instances)
                        .max(item.first_instance),
            );
        }
    }

    // ── G-buffer pass (PBR geometry → albedo / normal / world-position) ─────
    if let Some(ref def) = renderer.deferred {
        let mut gbuf_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("G-Buffer Pass"),
            color_attachments: &[
                Some(wgpu::RenderPassColorAttachment {
                    view: &def.albedo_metallic_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: &def.normal_roughness_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: &def.world_position_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                }),
                Some(wgpu::RenderPassColorAttachment {
                    view: &def.world_tangent_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                }),
            ],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &renderer.depth_texture_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Load, // Z-Prepass populated this!
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        gbuf_pass.set_pipeline(&def.gbuffer_pipeline);
        gbuf_pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
        gbuf_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
        gbuf_pass.set_bind_group(4, &renderer.scene.instance_bind_group, &[]);
        for item in draw_items {
            if item.unlit || item.is_transparent {
                continue;
            }
            let skel_bg = item
                .skeleton_bind_group
                .as_ref()
                .unwrap_or(&renderer.scene.dummy_skeleton_bind_group);
            gbuf_pass.set_bind_group(3, skel_bg.as_ref(), &[]);
            gbuf_pass.set_bind_group(1, &*item.bind_group, &[]);
            gbuf_pass.set_vertex_buffer(0, item.vbuf.slice(..));
            gbuf_pass.draw(
                0..item.vertex_count,
                // Main pass: camera-visible instances only.
                item.first_instance
                    ..(item.first_instance + item.camera_count)
                        .min(uploaded_instances)
                        .max(item.first_instance),
            );
        }
    }

    // ── Decal Pass (Blend into G-buffer) ──────────────────────────
    let mut decal_draws = Vec::new();
    if let Some(ref decal_state) = renderer.decal {
        let decals = world.borrow::<crate::renderer::components::Decal>();
        let transforms = world.borrow::<gizmo_physics_core::Transform>();
        let mut uniform_data = Vec::new();

        for (id, decal) in decals.iter() {
            if let Some(trans) = transforms.get(id) {
                let model = trans.local_matrix;
                let inv_model = model.inverse();

                uniform_data.push(crate::renderer::decal::DecalUniforms {
                    inv_model: inv_model.to_cols_array(),
                    model: model.to_cols_array(),
                    albedo_color: [decal.color.x, decal.color.y, decal.color.z, decal.color.w],
                    _pad: [0.0; 28],
                });

                decal_draws.push(decal.bind_group.clone());
                if uniform_data.len() >= 1024 {
                    break;
                } // Max 1024 decals limit
            }
        }

        if !uniform_data.is_empty() {
            renderer.queue.write_buffer(
                &decal_state.uniform_buffer,
                0,
                bytemuck::cast_slice(&uniform_data),
            );
        }
    }

    if !decal_draws.is_empty() {
        if let (Some(ref decal_state), Some(ref def)) = (&renderer.decal, &renderer.deferred) {
            let mut decal_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Decal Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &def.albedo_metallic_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None, // No depth testing needed
                timestamp_writes: None,
                occlusion_query_set: None,
            multiview_mask: None,
            });

            decal_pass.set_pipeline(&decal_state.pipeline);
            decal_pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
            decal_pass.set_bind_group(1, &decal_state.world_pos_bg, &[]);
            decal_pass.set_vertex_buffer(0, decal_state.vertex_buffer.slice(..));

            for (i, bind_group) in decal_draws.iter().enumerate() {
                let offset = (i * 256) as u32;
                decal_pass.set_bind_group(2, bind_group.as_ref(), &[]);
                decal_pass.set_bind_group(3, &decal_state.decal_uniform_bg, &[offset]);
                decal_pass.draw(0..36, 0..1);
            }
        }
    }

    // ── Deferred lighting pass (G-buffers → HDR) ──────────────────────────
    if let Some(ref def) = renderer.deferred {
        let mut light_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Deferred Lighting Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &renderer.post.hdr_texture_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.4, g: 0.6, b: 0.9, a: 1.0 }),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        light_pass.set_pipeline(&def.lighting_pipeline);
        light_pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
        light_pass.set_bind_group(1, &renderer.scene.shadow_bind_group, &[]);
        light_pass.set_bind_group(2, &def.gbuffer_bind_group, &[]);
        light_pass.draw(0..3, 0..1);
    }

}

pub(super) fn record_ssao(encoder: &mut wgpu::CommandEncoder, renderer: &Renderer) {
    // ── SSAO: hemisphere sampling → raw AO texture ────────────────────────────
    if let Some(ref ssao) = renderer.ssao {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("SSAO Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &ssao.ao_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&ssao.ssao_pipeline);
        pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
        pass.set_bind_group(1, &ssao.ssao_gbuf_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    // ── SSAO blur: 5×5 box filter → blurred AO texture ───────────────────────
    if let Some(ref ssao) = renderer.ssao {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("SSAO Blur Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &ssao.ao_blurred_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&ssao.blur_pipeline);
        pass.set_bind_group(0, &ssao.blur_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    // ── SSAO apply: multiply AO into HDR (multiply blend) ─────────────────────
    if let Some(ref ssao) = renderer.ssao {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("SSAO Apply Pass"),
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
        pass.set_pipeline(&ssao.apply_pipeline);
        pass.set_bind_group(0, &ssao.apply_bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

}

pub(super) fn record_forward_and_fluid(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &Renderer,
    world: &World,
    draw_items: &[DrawItem],
    uploaded_instances: u32,
    particle_lod: f32,
    fluid_lod: f32,
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
        render_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
        render_pass.set_bind_group(4, &renderer.scene.instance_bind_group, &[]);

        let show_wireframes = world.get_resource::<WireframeConfig>().map(|c| c.global).unwrap_or(false);

        for item in draw_items {
            let mut draw_solid = false;
            let draw_wire = show_wireframes && !item.is_skybox;

            if item.is_skybox || item.unlit || renderer.deferred.is_none() || item.is_transparent {
                draw_solid = true;
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
                render_pass.set_bind_group(3, skel_bg.as_ref(), &[]);
                render_pass.set_vertex_buffer(0, item.vbuf.slice(..));
                render_pass.draw(
                    0..item.vertex_count,
                    // Main pass: camera-visible instances only.
                    item.first_instance
                        ..(item.first_instance + item.camera_count)
                            .min(uploaded_instances)
                            .max(item.first_instance),
                );
            }

            if draw_wire {
                render_pass.set_pipeline(&renderer.scene.wireframe_pipeline);
                render_pass.set_bind_group(1, &*item.bind_group, &[]);
                render_pass.set_bind_group(3, skel_bg.as_ref(), &[]);
                render_pass.set_vertex_buffer(0, item.vbuf.slice(..));
                render_pass.draw(
                    0..item.vertex_count,
                    // Main pass: camera-visible instances only.
                    item.first_instance
                        ..(item.first_instance + item.camera_count)
                            .min(uploaded_instances)
                            .max(item.first_instance),
                );
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

        // Draw GPU Particles
        if let Some(particles) = &renderer.gpu_particles {
            let active_parts = (particles.max_particles as f32 * particle_lod) as u32;
            particles.render_pass(
                &mut render_pass,
                &renderer.scene.global_bind_group,
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

}

pub(super) fn record_screen_space_effects(encoder: &mut wgpu::CommandEncoder, renderer: &Renderer) {


    // ── SSR: Screen Space Reflections ───────────────────────────────────────────
    if let Some(ref ssr) = renderer.ssr {
        // Pass 1: SSR Raymarch
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("SSR Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &ssr.ssr_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            multiview_mask: None,
            });
            pass.set_pipeline(&ssr.ssr_pipeline);
            pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
            pass.set_bind_group(1, &ssr.ssr_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        // Pass 2: SSR Apply (Additive blend into HDR texture)
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("SSR Apply Pass"),
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
            pass.set_pipeline(&ssr.apply_pipeline);
            pass.set_bind_group(0, &ssr.apply_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    // ── SSGI: Screen Space Global Illumination ────────────────────────────────
    if let Some(ref ssgi) = renderer.ssgi {
        // Pass 1: SSGI Raymarch
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("SSGI Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &ssgi.ssgi_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            multiview_mask: None,
            });
            pass.set_pipeline(&ssgi.ssgi_pipeline);
            pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
            pass.set_bind_group(1, &ssgi.ssgi_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        // Pass 2: SSGI Blur
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("SSGI Blur Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &ssgi.ssgi_blurred_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            multiview_mask: None,
            });
            pass.set_pipeline(&ssgi.blur_pipeline);
            pass.set_bind_group(0, &ssgi.blur_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        // Pass 3: SSGI Apply (Additive blend into HDR texture)
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("SSGI Apply Pass"),
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
            pass.set_pipeline(&ssgi.apply_pipeline);
            pass.set_bind_group(0, &ssgi.apply_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    // ── Volumetric Lighting (God Rays) ──────────────────────────────────────────
    if let Some(ref vol) = renderer.volumetric {
        // Pass 1: Volumetric Raymarch
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Volumetric Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &vol.volumetric_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            multiview_mask: None,
            });
            pass.set_pipeline(&vol.volumetric_pipeline);
            pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
            pass.set_bind_group(1, &renderer.scene.shadow_bind_group, &[]);
            pass.set_bind_group(2, &vol.volumetric_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        // Pass 2: Volumetric Apply (Additive blend into HDR texture)
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Volumetric Apply Pass"),
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
            pass.set_pipeline(&vol.apply_pipeline);
            pass.set_bind_group(0, &vol.apply_bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }

}

pub(super) fn record_taa_and_overlays(
    encoder: &mut wgpu::CommandEncoder,
    renderer: &mut Renderer,
    world: &mut World,
) {
    // ── TAA resolve: blend jittered HDR with clamped history ─────────────────
    if let Some(ref taa) = renderer.taa {
        if taa.enabled {
            let (resolve_bg, output_view) = taa.current_resolve_inputs_output();
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("TAA Resolve Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: output_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            multiview_mask: None,
            });
            pass.set_pipeline(&taa.resolve_pipeline);
            pass.set_bind_group(0, resolve_bg, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    // ── TAA blit: copy stabilized history output back into HDR texture ───────
    if let Some(ref taa) = renderer.taa {
        if taa.enabled {
            let blit_bg = taa.current_blit_bg();
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("TAA Blit Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &renderer.post.hdr_texture_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            multiview_mask: None,
            });
            pass.set_pipeline(&taa.blit_pipeline);
            pass.set_bind_group(0, &taa.empty_bg, &[]);
            pass.set_bind_group(1, blit_bg, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    // Render Gizmos AFTER TAA to avoid ghosting/washing out dynamic overlays
    if let Some(gizmos) = world.get_resource::<crate::renderer::Gizmos>() {
        if let Some(debug_renderer) = &mut renderer.debug_renderer {
            debug_renderer.update(&renderer.queue, &gizmos);
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Gizmo Render Pass (Post-TAA)"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &renderer.post.hdr_texture_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load, // Keep TAA-stabilized geometry scene
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &renderer.depth_texture_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            multiview_mask: None,
            });
            debug_renderer.render(
                &mut pass,
                &renderer.scene.global_bind_group,
                gizmos.depth_test,
            );
        }
    }

    // Auto-clear gizmos for the next frame ONLY if a physics step occurred
    let mut should_clear = true;
    let current_step = world.get_resource::<gizmo_core::time::PhysicsTime>()
        .map(|pt| pt.step_count());

    if let Some(current_step) = current_step {
        struct GizmosLastStepCount(u64);
        
        let has_resource = world.get_resource::<GizmosLastStepCount>().is_some();
        if has_resource {
            if let Some(mut last_step) = world.get_resource_mut::<GizmosLastStepCount>() {
                if last_step.0 == current_step {
                    should_clear = false;
                } else {
                    last_step.0 = current_step;
                }
            }
        } else {
            world.insert_resource(GizmosLastStepCount(current_step));
        }
    }

    if should_clear {
        if let Some(mut gizmos) = world.get_resource_mut::<crate::renderer::Gizmos>() {
            gizmos.clear();
        }
    }

    // Advance TAA ping-pong and frame counter
    if let Some(ref mut taa) = renderer.taa {
        if taa.enabled {
            taa.advance_frame();
        }
    }
}
