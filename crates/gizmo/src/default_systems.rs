use crate::core::World;
use crate::math::{Mat4, Vec3};
use crate::physics::Transform;
use crate::renderer::{
    components::{Camera, Material, Mesh, MeshRenderer},
    Renderer,
};
use bytemuck;
use wgpu;

pub struct DrawItem {
    vbuf: std::sync::Arc<wgpu::Buffer>,
    vertex_count: u32,
    bind_group: std::sync::Arc<wgpu::BindGroup>,
    unlit: bool,
}

/// Bevy'nin DefaultPlugins davranisini taklit eden, sadece modelleri
/// isiklandirip hizlica ekrana basmaya yarayan kutudan cikmis Render Motoru.
/// Yeni acilan `tut` gibi bos projelerde yuzlerce satir kod yazmamak icin kullanilir.
pub fn default_render_pass(
    world: &World,
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    renderer: &mut Renderer,
) {
    let aspect = if renderer.size.height > 0 {
        renderer.size.width as f32 / renderer.size.height as f32
    } else {
        1.0
    };
    let mut proj = Mat4::perspective_rh(std::f32::consts::FRAC_PI_4, aspect, 0.1, 2000.0);
    let mut view_mat = Mat4::from_translation(Vec3::ZERO);
    let mut cam_pos = Vec3::ZERO;
    let mut cam_forward = Vec3::new(0.0, 0.0, -1.0);

    // TODO: Bütün nesnelerin (özellikle kamera ve çizilecek objelerin) global matrix'leri
    // bu pass çağrılmadan hemen önce bir `update_transforms(world)` sistemiyle güncellenmiş olmalıdır.

    if let (Some(cameras), Some(transforms)) =
        (world.borrow::<Camera>().expect("ECS Aliasing Error"), world.borrow::<Transform>().expect("ECS Aliasing Error"))
    {
        // TODO: Aktif kamera için `ActiveCamera` tarzı bir marker bileşeni kullanılmalı.
        // ECS array sırası stabil değildir. Şimdilik geçici çözüm olarak ilki alınıyor.
        if let Some((active_cam, _)) = cameras.iter().next() {
            if let (Some(cam), Some(trans)) =
                (cameras.get(active_cam), transforms.get(active_cam))
            {
                proj = cam.get_projection(aspect);
                view_mat = cam.get_view(trans.position);
                cam_pos = trans.position;
                cam_forward = trans.rotation * Vec3::new(0.0, 0.0, -1.0);
            }
        }
    }

    let view_proj = proj * view_mat;
    let frustum = crate::renderer::Frustum::from_matrix(&view_proj);

    let id = Mat4::IDENTITY.to_cols_array_2d();
    let scene_uniform_data = crate::renderer::gpu_types::SceneUniforms {
        view_proj: view_proj.to_cols_array_2d(),
        camera_pos: [cam_pos.x, cam_pos.y, cam_pos.z, 1.0],
        sun_direction: [0.0, -1.0, 0.0, 1.0],
        sun_color: [1.0, 1.0, 1.0, 1.0],
        lights: [crate::renderer::gpu_types::LightData {
            position: [0.0; 4],
            color: [0.0; 4],
            direction: [0.0, -1.0, 0.0, 0.0],
            params: [0.0; 4],
        }; 10],
        light_view_proj: [id; 4],
        cascade_splits: [10.0, 50.0, 200.0, 2000.0],
        camera_forward: [cam_forward.x, cam_forward.y, cam_forward.z, 0.0],
        cascade_params: [0.1, 1.0 / crate::renderer::SHADOW_MAP_RES as f32, 0.0, 0.0],
        num_lights: 0,
        // _align_pad ve _pad_scene: wgpu Uniform alignment kuralları gereği son paddingler
        _align_pad: [0; 3],
        _pad_scene: [0; 3],
        _end_pad: 0,
    };
    renderer.queue.write_buffer(
        &renderer.scene.global_uniform_buffer,
        0,
        bytemuck::cast_slice(&[scene_uniform_data]),
    );

    let renderers = world.borrow::<MeshRenderer>().expect("ECS Aliasing Error");
    let mut instances = Vec::new();
    let mut draw_items = Vec::new();
    if let Some(mut q) = world.query::<(&Mesh, &Transform, &Material)>() {
        for (e, (mesh, trans, mat)) in q.iter_mut() {
            if renderers.as_ref().map_or(true, |r| r.get(e).is_none()) {
                continue;
            }

            let center_mat = Mat4::from_translation(mesh.center_offset);
            let model = trans.global_matrix * center_mat;
            if !crate::renderer::visible_in_frustum(&frustum, &model, mesh.bounds) {
                continue;
            }
            let instance_data = crate::renderer::gpu_types::InstanceRaw {
                model: model.to_cols_array_2d(),
                albedo_color: [mat.albedo.x, mat.albedo.y, mat.albedo.z, mat.albedo.w],
                roughness: mat.roughness,
                metallic: mat.metallic,
                unlit: mat.unlit,
                _padding: 0.0,
            };
            instances.push(instance_data);
            draw_items.push(DrawItem {
                vbuf: mesh.vbuf.clone(),
                vertex_count: mesh.vertex_count,
                bind_group: mat.bind_group.clone(),
                unlit: mat.unlit > 0.5,
            });
        }
    }

    // Instance limiti kontrolü (Taşmaları önlemek için capaciteyi zorla)
    // TODO: Eğer needed > capacity ise çalışma zamanı pipeline re-allocation yapılmalı.
    let max_instances = renderer.scene.instance_capacity as usize;
    let instances: Vec<_> = instances.into_iter().take(max_instances).collect();

    if !instances.is_empty() {
        renderer.queue.write_buffer(
            &renderer.scene.instance_buffer,
            0,
            bytemuck::cast_slice(&instances),
        );
    }

    {
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Default Engine Render Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &renderer.post.hdr_texture_view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color {
                        r: 0.1,
                        g: 0.1,
                        b: 0.15,
                        a: 1.0,
                    }),
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
        // TODO: Shadow map pass eksik, render_pass öncesinde directional shadow mapping calistirilmali!

        // Draw loop disina sabit BindGroupLayout lari aliyoruz (Performans optimizasyonu)
        render_pass.set_bind_group(0, &renderer.scene.global_bind_group, &[]);
        render_pass.set_bind_group(2, &renderer.scene.shadow_bind_group, &[]);
        render_pass.set_bind_group(3, &renderer.scene.dummy_skeleton_bind_group, &[]);
        render_pass.set_bind_group(4, &renderer.scene.instance_bind_group, &[]);

        for (i, item) in draw_items.iter().enumerate() {
            let pipeline = if item.unlit {
                &renderer.scene.unlit_pipeline
            } else {
                &renderer.scene.render_pipeline
            };
            render_pass.set_pipeline(pipeline);
            render_pass.set_bind_group(1, &item.bind_group, &[]);
            render_pass.set_vertex_buffer(0, item.vbuf.slice(..));
            render_pass.draw(0..item.vertex_count, (i as u32)..((i as u32) + 1));
        }
    }

    renderer.run_post_processing(encoder, view);
}
