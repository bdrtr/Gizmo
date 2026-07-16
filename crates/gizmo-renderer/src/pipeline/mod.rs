//! Sahne render pipeline'larının kurulumu: layout'lar, shadow kaynakları, core/shadow
//! pipeline'ları ve global bind group'lar. Alt modüllere bölünmüştür; genel API
//! (SceneState, build_scene_pipelines, rebuild_pipelines, load_shader) bu modülden
//! değişmeden yeniden ihraç edilir.

mod layouts;
mod pipelines;
mod shaders;
mod uniforms;

pub use shaders::load_shader;
pub use shaders::load_shader_composed;
#[cfg(target_arch = "wasm32")]
pub use shaders::load_shader_composed_web;

use layouts::{build_layouts, LayoutRefs};
use pipelines::{build_core_pipelines, build_shadow_pipeline};
use uniforms::{build_global_uniforms, build_shadow_resources};

use crate::gpu_types::ShadowVsUniform;
use std::sync::Arc;
use wgpu::util::DeviceExt;

/// Sahne render durumu — pipeline'lar, shadow, skeleton ve global bind group'lar
pub struct SceneState {
    pub render_pipeline: wgpu::RenderPipeline,
    pub render_double_sided_pipeline: wgpu::RenderPipeline,
    pub unlit_pipeline: wgpu::RenderPipeline,
    pub sky_pipeline: wgpu::RenderPipeline,
    pub water_pipeline: wgpu::RenderPipeline,
    pub shadow_pipeline: wgpu::RenderPipeline,
    pub wireframe_pipeline: wgpu::RenderPipeline,
    pub transparent_pipeline: wgpu::RenderPipeline,
    pub grid_pipeline: wgpu::RenderPipeline,
    pub global_uniform_buffer: wgpu::Buffer,
    pub global_bind_group_layout: wgpu::BindGroupLayout,
    pub global_bind_group: wgpu::BindGroup,
    pub shadow_bind_group_layout: wgpu::BindGroupLayout,
    pub shadow_bind_group: wgpu::BindGroup,
    /// Depth `texture_2d_array` (all CSM layers) for comparison sampling in lit shaders.
    pub shadow_texture_view: wgpu::TextureView,
    /// One 2D depth view per cascade for shadow map rendering passes.
    pub shadow_cascade_layer_views: [wgpu::TextureView; 4],
    pub shadow_depth_texture: wgpu::Texture,
    pub point_shadow_depth_texture: wgpu::Texture,
    pub point_shadow_cube_view: wgpu::TextureView,
    pub point_shadow_face_views: [wgpu::TextureView; 6],
    pub shadow_pass_bind_group_layout: wgpu::BindGroupLayout,
    /// One uniform buffer + bind group per CSM cascade (avoids per-pass overwrite races on the queue).
    pub shadow_cascade_uniform_buffers: [wgpu::Buffer; 4],
    pub shadow_pass_bind_groups: [wgpu::BindGroup; 4],
    pub point_shadow_uniform_buffers: [wgpu::Buffer; 6],
    pub point_shadow_pass_bind_groups: [wgpu::BindGroup; 6],
    pub texture_bind_group_layout: wgpu::BindGroupLayout,
    pub skeleton_bind_group_layout: wgpu::BindGroupLayout,
    pub dummy_skeleton_bind_group: Arc<wgpu::BindGroup>,
    pub instance_bind_group_layout: wgpu::BindGroupLayout,
    pub instance_buffer: wgpu::Buffer,
    pub instance_bind_group: wgpu::BindGroup,
    /// Current capacity (number of InstanceRaw items) of `instance_buffer`.
    pub instance_capacity: usize,
}

impl SceneState {
    pub fn ensure_instance_capacity(&mut self, device: &wgpu::Device, needed: usize) -> bool {
        if needed <= self.instance_capacity {
            return false;
        }

        let new_capacity = if self.instance_capacity == 0 {
            needed.max(8_192)
        } else {
            needed.max(self.instance_capacity + self.instance_capacity / 2).max(self.instance_capacity + 4096)
        };
        let new_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Instance Buffer (grown)"),
            size: (new_capacity * std::mem::size_of::<crate::InstanceRaw>()) as u64,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let new_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &self.instance_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: new_buffer.as_entire_binding(),
            }],
            label: Some("instance_bind_group (grown)"),
        });

        self.instance_buffer = new_buffer;
        self.instance_bind_group = new_bind_group;
        self.instance_capacity = new_capacity;
        true
    }
}

// ------------------------------------------------------------------
// ANA YÖNETİCİ METOTLAR
// ------------------------------------------------------------------

#[tracing::instrument(skip_all)]
pub fn build_scene_pipelines(device: &wgpu::Device) -> SceneState {
    let global_uniform_buffer = build_global_uniforms(device);
    let (
        shadow_depth_texture,
        shadow_texture_view,
        shadow_cascade_layer_views,
        shadow_sampler,
        point_shadow_depth_texture,
        point_shadow_cube_view,
        point_shadow_face_views,
    ) = build_shadow_resources(device);
    let layouts = build_layouts(device);

    let global_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: &layouts.global,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: global_uniform_buffer.as_entire_binding(),
        }],
        label: Some("global_bind_group"),
    });

    let shadow_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: &layouts.shadow,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&shadow_texture_view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&shadow_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(&point_shadow_cube_view),
            },
        ],
        label: Some("shadow_bind_group"),
    });

    let id4 = [
        [1.0f32, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let shadow_cascade_uniform_buffers = std::array::from_fn(|i| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("Shadow cascade VS uniform {i}")),
            contents: bytemuck::bytes_of(&ShadowVsUniform {
                light_view_proj: id4,
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    });

    let shadow_pass_bind_groups = std::array::from_fn(|i| {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &layouts.shadow_pass,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: shadow_cascade_uniform_buffers[i].as_entire_binding(),
            }],
            label: Some(&format!("shadow_pass_bind_group_{i}")),
        })
    });

    let point_shadow_uniform_buffers = std::array::from_fn(|i| {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("Point shadow VS uniform {i}")),
            contents: bytemuck::bytes_of(&ShadowVsUniform {
                light_view_proj: id4,
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    });

    let point_shadow_pass_bind_groups = std::array::from_fn(|i| {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &layouts.shadow_pass, // Reusing same layout as directional shadows
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: point_shadow_uniform_buffers[i].as_entire_binding(),
            }],
            label: Some(&format!("point_shadow_pass_bind_group_{i}")),
        })
    });

    let dummy_identity: [[f32; 4]; 4] = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ];
    let dummy_skeleton_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Dummy Skeleton Buffer"),
        contents: bytemuck::cast_slice(&[dummy_identity; 128]),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    });
    let dummy_skeleton_bind_group =
        Arc::new(device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &layouts.skeleton,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: dummy_skeleton_buffer.as_entire_binding(),
            }],
            label: Some("dummy_skeleton_bind_group"),
        }));

    let initial_capacity: usize = 8_192;
    let instance_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Instance Buffer"),
        size: (initial_capacity * std::mem::size_of::<crate::InstanceRaw>()) as u64,
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let instance_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: &layouts.instance,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: instance_buffer.as_entire_binding(),
        }],
        label: Some("instance_bind_group"),
    });

    let layout_refs = LayoutRefs {
        global: &layouts.global,
        shadow: &layouts.shadow,
        shadow_pass: &layouts.shadow_pass,
        texture: &layouts.texture,
        skeleton: &layouts.skeleton,
        instance: &layouts.instance,
    };
    let core_pipelines = build_core_pipelines(device, &layout_refs);
    let shadow_pipeline = build_shadow_pipeline(device, &layout_refs);

    tracing::info!(
        initial_instance_capacity = initial_capacity,
        "[Pipeline] scene render pipelines built (core + shadow + global bind groups)"
    );

    SceneState {
        render_pipeline: core_pipelines.render,
        render_double_sided_pipeline: core_pipelines.render_double_sided,
        wireframe_pipeline: core_pipelines.wireframe,
        unlit_pipeline: core_pipelines.unlit,
        sky_pipeline: core_pipelines.sky,
        water_pipeline: core_pipelines.water,
        transparent_pipeline: core_pipelines.transparent,
        grid_pipeline: core_pipelines.grid,
        shadow_pipeline,
        global_uniform_buffer,
        global_bind_group_layout: layouts.global,
        global_bind_group,
        shadow_bind_group_layout: layouts.shadow,
        shadow_bind_group,
        shadow_texture_view,
        shadow_cascade_layer_views,
        shadow_depth_texture,
        point_shadow_depth_texture,
        point_shadow_cube_view,
        point_shadow_face_views,
        shadow_pass_bind_group_layout: layouts.shadow_pass,
        shadow_cascade_uniform_buffers,
        shadow_pass_bind_groups,
        point_shadow_uniform_buffers,
        point_shadow_pass_bind_groups,
        texture_bind_group_layout: layouts.texture,
        skeleton_bind_group_layout: layouts.skeleton,
        dummy_skeleton_bind_group,
        instance_bind_group_layout: layouts.instance,
        instance_buffer,
        instance_bind_group,
        instance_capacity: initial_capacity,
    }
}

#[tracing::instrument(skip_all)]
pub fn rebuild_pipelines(renderer: &mut crate::Renderer) {
    let device = &renderer.device;
    let post_shader = load_shader(
        device,
        "demo/assets/shaders/post_process.wgsl",
        include_str!("../shaders/post_process.wgsl"),
        "Post-Processing Shader",
    );

    // Geçici LayoutRefs tutucusu, render pipeline'ı için mevcut layoutları referans alır
    let layouts = LayoutRefs {
        global: &renderer.scene.global_bind_group_layout,
        shadow: &renderer.scene.shadow_bind_group_layout,
        shadow_pass: &renderer.scene.shadow_pass_bind_group_layout,
        texture: &renderer.scene.texture_bind_group_layout,
        skeleton: &renderer.scene.skeleton_bind_group_layout,
        instance: &renderer.scene.instance_bind_group_layout,
    };

    let core_pipelines = build_core_pipelines(device, &layouts);
    let shadow_pipeline = build_shadow_pipeline(device, &layouts);

    renderer.scene.render_pipeline = core_pipelines.render;
    renderer.scene.render_double_sided_pipeline = core_pipelines.render_double_sided;
    renderer.scene.wireframe_pipeline = core_pipelines.wireframe;
    renderer.scene.unlit_pipeline = core_pipelines.unlit;
    renderer.scene.sky_pipeline = core_pipelines.sky;
    renderer.scene.water_pipeline = core_pipelines.water;
    renderer.scene.transparent_pipeline = core_pipelines.transparent;
    renderer.scene.grid_pipeline = core_pipelines.grid;
    renderer.scene.shadow_pipeline = shadow_pipeline;

    crate::post_process::rebuild_post_pipelines(renderer, &post_shader);
    tracing::info!("[Pipeline] core + shadow + post-process pipelines rebuilt");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Çekirdek WGSL shader'ları headless device'ta naga ile derlenebilmeli.
    /// shader.wgsl/gbuffer.wgsl düzenlemelerinin (skinned-normal inverse-transpose vb.)
    /// WGSL'i geçersiz kılmadığını doğrular. GPU adapter yoksa graceful atlanır.
    #[test]
    fn core_shaders_compile() {
        pollster::block_on(async {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::all(),
                flags: wgpu::InstanceFlags::default(),
                memory_budget_thresholds: Default::default(),
                backend_options: Default::default(),
                display: None,
            });
            let Ok(adapter) = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::LowPower,
                    ..Default::default()
                })
                .await
            else {
                tracing::info!("No GPU adapter; skipping core_shaders_compile.");
                return;
            };
            let Ok((device, _queue)) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    // shader.wgsl group(4) kullanıyor → renderer'ın gerçek limitiyle eşle.
                    required_limits: wgpu::Limits {
                        max_bind_groups: 6,
                        ..wgpu::Limits::default()
                    },
                    ..Default::default()
                })
                .await
            else {
                return;
            };

            // Motorun standalone modül olarak yüklediği render/post-process shader'ları.
            // (fluid_*/kernels/fluid_bindings concatenate edilen fragmanlar — hariç.)
            let shaders: &[(&str, &str)] = &[
                ("shader.wgsl", include_str!("../shaders/shader.wgsl")),
                ("gbuffer.wgsl", include_str!("../shaders/gbuffer.wgsl")),
                ("deferred_lighting.wgsl", include_str!("../shaders/deferred_lighting.wgsl")),
                ("post_process.wgsl", include_str!("../shaders/post_process.wgsl")),
                ("ssao.wgsl", include_str!("../shaders/ssao.wgsl")),
                ("ssao_blur.wgsl", include_str!("../shaders/ssao_blur.wgsl")),
                ("ssao_apply.wgsl", include_str!("../shaders/ssao_apply.wgsl")),
                ("ssr.wgsl", include_str!("../shaders/ssr.wgsl")),
                ("ssr_apply.wgsl", include_str!("../shaders/ssr_apply.wgsl")),
                ("ssgi.wgsl", include_str!("../shaders/ssgi.wgsl")),
                ("ssgi_blur.wgsl", include_str!("../shaders/ssgi_blur.wgsl")),
                ("ssgi_apply.wgsl", include_str!("../shaders/ssgi_apply.wgsl")),
                ("taa.wgsl", include_str!("../shaders/taa.wgsl")),
                ("fxaa.wgsl", include_str!("../shaders/fxaa.wgsl")),
                ("volumetric.wgsl", include_str!("../shaders/volumetric.wgsl")),
                ("volumetric_apply.wgsl", include_str!("../shaders/volumetric_apply.wgsl")),
                ("sky.wgsl", include_str!("../shaders/sky.wgsl")),
                ("unlit.wgsl", include_str!("../shaders/unlit.wgsl")),
                ("grid.wgsl", include_str!("../shaders/grid.wgsl")),
                ("water.wgsl", include_str!("../shaders/water.wgsl")),
                ("shadow.wgsl", include_str!("../shaders/shadow.wgsl")),
                ("point_shadow.wgsl", include_str!("../shaders/point_shadow.wgsl")),
                ("decal.wgsl", include_str!("../shaders/decal.wgsl")),
                ("debug_lines.wgsl", include_str!("../shaders/debug_lines.wgsl")),
                ("mipmap.wgsl", include_str!("../shaders/mipmap.wgsl")),
                // Self-contained compute shaders (own bindings inline). The fluid
                // shaders (spatial_hash/fluid_compute) share bindings via
                // fluid_bindings.wgsl so they are validated by the gpu_fluid
                // dispatch test instead; fem_compute/particle_compute by their own
                // GPU tests.
                ("physics_compute.wgsl", include_str!("../shaders/physics_compute.wgsl")),
                ("physics_culling.wgsl", include_str!("../shaders/physics_culling.wgsl")),
                ("physics_debug.wgsl", include_str!("../shaders/physics_debug.wgsl")),
                // Loaded via create_shader_module in gpu_physics/gpu_particles/gpu_cull (not
                // in the golden render test's pipelines), so validate their naga_oil
                // composition here — this test auto-composes any src containing `#import`.
                ("physics_render.wgsl", include_str!("../shaders/physics_render.wgsl")),
                ("particle_render.wgsl", include_str!("../shaders/particle_render.wgsl")),
                ("mesh_cull.wgsl", include_str!("../shaders/mesh_cull.wgsl")),
            ];

            // Shaders that go through the wasm `load_shader_composed_web` path: validate BOTH
            // the native and web shader-def variants here so the web build (no browser in CI)
            // is verified — the web variant strips `#ifdef SHADOWS` and remaps
            // `@group(#{SKELETON_GROUP/INSTANCE_GROUP})`, which is exactly where a bad #ifdef
            // (e.g. a shadow binding used outside the guard) would surface as an undefined id.
            let web_path = ["shader.wgsl", "unlit.wgsl", "water.wgsl", "sky.wgsl", "grid.wgsl"];

            let mut failures: Vec<String> = Vec::new();
            for (name, src) in shaders {
                // Shaders that `#import gizmo::common` (or use `#ifdef`/`#{...}`) are
                // naga_oil-composed before validation; new migrations are picked up
                // automatically. Compose under the NATIVE defs, and additionally under the WEB
                // defs for shaders on the web path.
                let mut variants: Vec<(&str, String)> = Vec::new();
                if src.contains("#import") || src.contains("#ifdef") || src.contains("#{") {
                    variants.push(("native", shaders::compose_wgsl(src, name, shaders::native_render_defs())));
                    if web_path.contains(name) {
                        variants.push(("web", shaders::compose_wgsl(src, name, shaders::web_render_defs())));
                    }
                } else {
                    variants.push(("raw", src.to_string()));
                }
                for (variant, final_src) in &variants {
                    let scope = device.push_error_scope(wgpu::ErrorFilter::Validation);
                    let _module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                        label: Some(name),
                        source: wgpu::ShaderSource::Wgsl(final_src.as_str().into()),
                    });
                    if let Some(err) = scope.pop().await {
                        failures.push(format!("{name} [{variant}]: {err:?}"));
                    }
                }
            }
            assert!(
                failures.is_empty(),
                "WGSL doğrulaması başarısız shader('lar):\n{}",
                failures.join("\n")
            );
        });
    }

    #[test]
    fn test_dynamic_instance_buffer_resize() {
        pollster::block_on(async {
            let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                backends: wgpu::Backends::all(),
                flags: wgpu::InstanceFlags::default(),
                memory_budget_thresholds: Default::default(),
                backend_options: Default::default(),
                display: None,
            });

            let adapter = instance
                .request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::LowPower,
                    ..Default::default()
                })
                .await;

            let adapter = match adapter {
                Ok(a) => a,
                Err(_) => {
                    tracing::info!(
                        "No suitable GPU adapter found for headless test. Skipping wgpu test."
                    );
                    return;
                }
            };

            // Wireframe pipeline requires POLYGON_MODE_LINE
            let adapter_features = adapter.features();
            if !adapter_features.contains(wgpu::Features::POLYGON_MODE_LINE) {
                tracing::info!(
                    "GPU adapter does not support POLYGON_MODE_LINE. Skipping pipeline test."
                );
                return;
            }

            let (device, _) = adapter
                .request_device(&wgpu::DeviceDescriptor {
                    required_features: wgpu::Features::POLYGON_MODE_LINE,
                    required_limits: wgpu::Limits {
                        max_bind_groups: 6,
                        ..wgpu::Limits::default()
                    },
                    label: None,
                    experimental_features: wgpu::ExperimentalFeatures::default(),
                    memory_hints: wgpu::MemoryHints::default(),
                    trace: wgpu::Trace::Off,
                })
                .await
                .unwrap();

            // Sahnemizi kur, default capacity 8_192 olmali!
            let mut scene_state = build_scene_pipelines(&device);
            assert_eq!(scene_state.instance_capacity, 8_192);

            // Daha kucuk bir obje listesi istenirse buyumez.
            let grew = scene_state.ensure_instance_capacity(&device, 100);
            assert!(!grew, "Buffer should not grow if capacity is enough");
            assert_eq!(scene_state.instance_capacity, 8_192);

            // Mevcudun disine ciktiginda (Ornegin 10_000) 1.5 katina grow eder.
            let grew2 = scene_state.ensure_instance_capacity(&device, 10_000);
            assert!(grew2, "Buffer should grow since needed > capacity");
            assert_eq!(scene_state.instance_capacity, 12_288);

            // Gercek byte miktarinin da artmis oldugundan emin olalim.
            let expected_bytes = (12_288 * std::mem::size_of::<crate::InstanceRaw>()) as u64;
            assert_eq!(scene_state.instance_buffer.size(), expected_bytes);

            // Yeniden mevcut sinirlar icinde kaldiginda grow etmez
            let grew3 = scene_state.ensure_instance_capacity(&device, 12_000);
            assert!(!grew3, "Buffer should not grow if capacity is enough");
            assert_eq!(scene_state.instance_capacity, 12_288);
        });
    }
}
