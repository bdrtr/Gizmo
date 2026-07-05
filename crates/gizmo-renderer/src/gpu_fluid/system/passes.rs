use super::*;

impl GpuFluidSystem {
    pub fn compute_pass(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        queue: &wgpu::Queue,
        update_grid: bool,
        active_particles: u32,
    ) {
        if active_particles == 0 {
            return;
        }
        let workgroups_parts = active_particles.div_ceil(64);

        // Keep the shader's `num_particles` guard (params offset 28) in sync with
        // the active LOD set. It is otherwise never updated, so under LOD < 1.0 the
        // hash pass (dispatched over `num_elements`) treats particles in
        // [active, N) as REAL and inserts them into the grid, but `grid_offsets`
        // only runs over `active` — so those particles get no cell offsets and are
        // silently dropped from every neighbor scan (density/lambda/viscosity),
        // corrupting incompressibility. Writing `active` here makes exactly the
        // active set be hashed, integrated, and offset-mapped as one population.
        queue.write_buffer(
            &self.params_buffer,
            28,
            bytemuck::cast_slice(&[active_particles]),
        );

        // 1. PBF PREDICT PASS
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Fluid Predict Pass"),
                timestamp_writes: None,
            });
            cpass.set_bind_group(0, &self.pipelines.compute_bind_group, &[0]);
            cpass.set_pipeline(&self.pipelines.pipeline_predict);
            cpass.dispatch_workgroups(workgroups_parts, 1, 1);
        }

        // 2. SPATIAL HASHING (Based on predicted positions)
        if update_grid {
            {
                let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Fluid Clear Pass"),
                    timestamp_writes: None,
                });
                cpass.set_bind_group(0, &self.pipelines.compute_bind_group, &[0]);
                cpass.set_pipeline(&self.pipelines.pipeline_clear);
                cpass.dispatch_workgroups(self.total_cells.div_ceil(64), 1, 1);
            }

            let num_elements = active_particles.next_power_of_two();

            {
                let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Fluid Hash Pass"),
                    timestamp_writes: None,
                });
                cpass.set_bind_group(0, &self.pipelines.compute_bind_group, &[0]);
                cpass.set_pipeline(&self.pipelines.pipeline_hash);
                // MUST run over num_elements so padded elements get their hashes set to 0xFFFFFFFF
                cpass.dispatch_workgroups(num_elements.div_ceil(64), 1, 1);
            }

            // O(log^2 N) bitonic sort passes
            let mut offset_idx = 0;
            let mut k = 2u32;
            while k <= num_elements {
                let mut j = k >> 1;
                while j > 0 {
                    let offset = offset_idx * 256;
                    let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                        label: Some("Fluid Sort Pass"),
                        timestamp_writes: None,
                    });
                    cpass.set_bind_group(
                        0,
                        &self.pipelines.compute_bind_group,
                        &[offset as wgpu::DynamicOffset],
                    );
                    cpass.set_pipeline(&self.pipelines.pipeline_sort);
                    cpass.dispatch_workgroups(num_elements.div_ceil(64), 1, 1);

                    offset_idx += 1;
                    j >>= 1;
                }
                k <<= 1;
            }

            {
                let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Fluid Offsets Pass"),
                    timestamp_writes: None,
                });
                cpass.set_bind_group(0, &self.pipelines.compute_bind_group, &[0]);
                cpass.set_pipeline(&self.pipelines.pipeline_offsets);
                cpass.dispatch_workgroups(workgroups_parts, 1, 1);
            }
        }

        // 3. PBF SOLVER ITERATIONS (AAA: increased from 4 to 6 for better convergence)
        for _ in 0..6 {
            {
                let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Fluid Calc Lambda"),
                    timestamp_writes: None,
                });
                cpass.set_bind_group(0, &self.pipelines.compute_bind_group, &[0]);
                cpass.set_pipeline(&self.pipelines.pipeline_calc_lambda);
                cpass.dispatch_workgroups(workgroups_parts, 1, 1);
            }
            {
                let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: Some("Fluid Apply Delta P"),
                    timestamp_writes: None,
                });
                cpass.set_bind_group(0, &self.pipelines.compute_bind_group, &[0]);
                cpass.set_pipeline(&self.pipelines.pipeline_apply_delta_p);
                cpass.dispatch_workgroups(workgroups_parts, 1, 1);
            }
        }

        // 4. AAA: VORTICITY CONFINEMENT — ω = ∇ × v (curl of velocity)
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Fluid Compute Vorticity"),
                timestamp_writes: None,
            });
            cpass.set_bind_group(0, &self.pipelines.compute_bind_group, &[0]);
            cpass.set_pipeline(&self.pipelines.pipeline_compute_vorticity);
            cpass.dispatch_workgroups(workgroups_parts, 1, 1);
        }

        // 5. AAA: UPDATE VELOCITY — Vorticity Confinement + Surface Tension + Viscosity
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Fluid Update Velocity Pass"),
                timestamp_writes: None,
            });
            cpass.set_bind_group(0, &self.pipelines.compute_bind_group, &[0]);
            cpass.set_pipeline(&self.pipelines.pipeline_update_velocity);
            cpass.dispatch_workgroups(workgroups_parts, 1, 1);
        }

        // 6. AAA: CLASSIFY PARTICLES — Foam / Spray / Droplet detection
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("Fluid Classify Particles"),
                timestamp_writes: None,
            });
            cpass.set_bind_group(0, &self.pipelines.compute_bind_group, &[0]);
            cpass.set_pipeline(&self.pipelines.pipeline_classify);
            cpass.dispatch_workgroups(workgroups_parts, 1, 1);
        }
    }

    pub fn render_pass<'a>(
        &'a self,
        _rpass: &mut wgpu::RenderPass<'a>,
        _global_scene_bind_group: &'a wgpu::BindGroup,
    ) {
        // Fallback for compatibility, not used directly by SSFR loop
    }
    pub fn render_ssfr(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        target_texture: &wgpu::Texture,
        target_view: &wgpu::TextureView,
        scene_depth_view: &wgpu::TextureView,
        global_scene_bind_group: &wgpu::BindGroup,
        active_particles: u32,
    ) {
        if active_particles == 0 {
            return;
        }
        // Copy the opaque background before rendering fluid on top
        encoder.copy_texture_to_texture(
            wgpu::TexelCopyTextureInfo {
                texture: target_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyTextureInfo {
                texture: &self.opaque_bg_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::Extent3d {
                width: self.opaque_bg_texture.width(),
                height: self.opaque_bg_texture.height(),
                depth_or_array_layers: 1,
            },
        );

        // 1. Depth Pass
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("SSFR Depth"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.raw_depth_texture_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 1.0,
                            g: 1.0,
                            b: 1.0,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth_texture_view,
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
            rpass.set_pipeline(&self.pipelines.pipeline_depth);
            rpass.set_bind_group(0, global_scene_bind_group, &[]);
            rpass.set_bind_group(1, &self.ssfr_particle_bg, &[]);
            rpass.draw(0..4, 0..active_particles);
        }

        // 2. Blur Pass (Ping-Pong)
        {
            let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("SSFR Blur"),
                timestamp_writes: None,
            });
            cpass.set_pipeline(&self.pipelines.pipeline_blur);

            let target_width = target_texture.width();
            let target_height = target_texture.height();

            // X Pass
            cpass.set_bind_group(0, &self.ssfr_blur_x_bg, &[]);
            cpass.dispatch_workgroups(target_width.div_ceil(16), target_height.div_ceil(16), 1);

            // Y Pass
            cpass.set_bind_group(0, &self.ssfr_blur_y_bg, &[]);
            cpass.dispatch_workgroups(target_width.div_ceil(16), target_height.div_ceil(16), 1);
        }

        // 3. Thickness Pass
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("SSFR Thickness"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.thickness_texture_view,
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
            rpass.set_pipeline(&self.pipelines.pipeline_thickness);
            rpass.set_bind_group(0, global_scene_bind_group, &[]);
            rpass.set_bind_group(1, &self.ssfr_particle_bg, &[]);
            rpass.draw(0..4, 0..active_particles);
        }

        // 4. Composite Pass
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("SSFR Composite"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: scene_depth_view,
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
            rpass.set_pipeline(&self.pipelines.pipeline_composite);
            rpass.set_bind_group(0, global_scene_bind_group, &[]);
            rpass.set_bind_group(1, &self.ssfr_composite_bg, &[]);
            rpass.draw(0..3, 0..1); // Fullscreen triangle
        }

        // 5. AAA: Foam/Spray/Droplet Render Pass
        {
            let mut rpass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("SSFR Foam/Spray"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load, // Preserve composite result
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: scene_depth_view,
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
            rpass.set_pipeline(&self.pipelines.pipeline_foam);
            rpass.set_bind_group(0, global_scene_bind_group, &[]);
            rpass.set_bind_group(1, &self.ssfr_particle_bg, &[]);
            rpass.draw(0..4, 0..active_particles); // Only foam/spray survive in vertex shader
        }
    }
}
