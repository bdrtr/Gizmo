//! Rain falling, and splashing where it lands (`Renderer::rainfall`).
//!
//! What [`Renderer::wetness`](crate::Renderer::wetness) does to surfaces — darker, smoother, puddles —
//! this does to the air: streaks falling round the camera and rings on the ground where they land. Nothing
//! is simulated (`shaders/rain.wgsl`). A drop is its instance index: a point of a box round the camera,
//! moved down with the clock and wrapped into the box, so it stands still in the world but for its fall.
//! It is a streak a few millimetres wide along where it went relative to the camera while a shutter was
//! open — its fall, and the camera's own motion, which at a car's speed draws the near drops out into long
//! lines streaming past —, at least a pixel and a half wide on screen with
//! what it covers of that as its strength, lit by its cluster's lights and scattered towards the camera —
//! forward the most, as drops throw light: a street's rain shows in its lamps and headlights, and hardly
//! in the dark between them. Where the rain map ([`crate::rain_map`]) has something above a drop it is
//! under cover and is not drawn; and a splash is a cell of a grid round the camera, a crown of water
//! thrown up and falling back again and again, standing on the surface the rain map says the rain falls on
//! there — a road, a pavement, a car's roof — and turned to the camera.
//!
//! Drawn into the HDR target after the forward pass, added to it, behind what the frame drew there. Without
//! a rain map (a frame with no [`Wetness`](crate::frame_uniforms::Wetness)) it falls everywhere and does
//! not splash.

use gizmo_math::Vec3;
use wgpu::util::DeviceExt;

/// A camera moving faster than this, m/s, between two frames did not move: it cut. Its rain is drawn as if
/// it stood still that frame, not as a screen of lines from where it was to where it is.
pub const CAMERA_CUT_SPEED: f32 = 150.0;

/// How it rains. [`Default`] is a steady rain.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rainfall {
    /// How many drops are in the box round the camera at once.
    pub drops: u32,
    /// The box round the camera they fall in, metres: across, up, along.
    pub box_size: [f32; 3],
    /// How fast they fall, m/s.
    pub speed: f32,
    /// The wind, m/s, added to their fall: it slants them.
    pub wind: [f32; 3],
    /// How long the camera's shutter stays open, seconds: a streak is where its drop went, relative to the
    /// camera, while it was open — its fall, and the camera's motion.
    pub shutter: f32,
    /// A drop's width, metres.
    pub drop_width: f32,
    /// How bright the rain is: a factor on what the lights give it.
    pub brightness: f32,
    /// What the air lends a drop where no light reaches it, before `brightness`.
    pub ambient: f32,
    /// Splash cells a side of the grid round the camera; 0 for none.
    pub splash_cells: u32,
    /// A splash cell's side, metres.
    pub splash_cell: f32,
    /// How high and wide a splash's crown is thrown, metres.
    pub splash_size: f32,
    /// How long a splash lasts, seconds.
    pub splash_life: f32,
}

impl Default for Rainfall {
    fn default() -> Self {
        Self {
            drops: 20_000,
            box_size: [40.0, 24.0, 40.0],
            speed: 9.0,
            wind: [0.0, 0.0, 0.0],
            shutter: 0.03,
            drop_width: 0.0015,
            brightness: 1.0,
            ambient: 0.002,
            splash_cells: 96,
            splash_cell: 0.5,
            splash_size: 0.12,
            splash_life: 0.3,
        }
    }
}

/// The uniform `rain.wgsl` reads (`RainParams`).
#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
struct RainParams {
    box_size: [f32; 4],
    fall: [f32; 4],
    look: [f32; 4],
    map: [f32; 4],
    camera_velocity: [f32; 4],
}

impl RainParams {
    fn new(rain: &Rainfall, camera_velocity: Vec3) -> Self {
        let [wx, wy, wz] = rain.wind;
        Self {
            box_size: [rain.box_size[0].max(1.0), rain.box_size[1].max(1.0), rain.box_size[2].max(1.0), rain.drops as f32],
            fall: [wx, wy - rain.speed.max(0.0), wz, rain.shutter.max(0.0)],
            look: [rain.drop_width.max(0.0), rain.brightness.max(0.0), rain.splash_cells as f32, rain.splash_size.max(0.0)],
            map: [crate::rain_map::RAIN_MAP_DEPTH, rain.splash_cell.max(0.01), rain.splash_life.max(0.01), rain.ambient.max(0.0)],
            camera_velocity: camera_velocity.extend(0.0).to_array(),
        }
    }
}

/// Where a view's camera was at a clock, and how fast it was going then.
struct CameraTrack {
    view: Option<usize>,
    camera: Vec3,
    clock: f32,
    velocity: Vec3,
}

/// Each view's camera at its last frame (`SceneState::active_view`), for its velocity: two views of a frame
/// are two cameras, and one's velocity measured from the other's place would be nonsense.
#[derive(Default)]
struct CameraTracks(Vec<CameraTrack>);

impl CameraTracks {
    /// The velocity of `view`'s camera, now at `camera` with the clock at `clock`: from where it was at the
    /// view's last clock. The same for every pass of one frame; nothing on a view's first frame, after a
    /// pause of more than a quarter second, or across a cut ([`CAMERA_CUT_SPEED`]).
    fn velocity(&mut self, view: Option<usize>, camera: Vec3, clock: f32) -> Vec3 {
        let Some(track) = self.0.iter_mut().find(|t| t.view == view) else {
            // A handful of views at most; a view no longer drawn makes room.
            if self.0.len() >= 8 {
                self.0.remove(0);
            }
            self.0.push(CameraTrack { view, camera, clock, velocity: Vec3::ZERO });
            return Vec3::ZERO;
        };
        if clock != track.clock {
            let dt = clock - track.clock;
            let velocity = (camera - track.camera) / dt;
            track.velocity = if dt > 0.0 && dt <= 0.25 && velocity.length() < CAMERA_CUT_SPEED { velocity } else { Vec3::ZERO };
            track.camera = camera;
            track.clock = clock;
        }
        track.velocity
    }
}

/// The pipelines and the uniform. Built the first frame it rains (`Renderer::record_rain`).
pub struct RainState {
    drop_pipeline: wgpu::RenderPipeline,
    splash_pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    params: wgpu::Buffer,
    /// A one-texel rain map that says "nothing below": what is bound when the frame has none.
    no_map: wgpu::TextureView,
    /// Each view's camera at its last frame, for its velocity.
    tracks: CameraTracks,
}

impl RainState {
    /// Builds the pipelines over the scene's global layout, for an HDR target of `format`.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, global_layout: &wgpu::BindGroupLayout, format: wgpu::TextureFormat) -> Self {
        let shader = crate::pipeline::load_shader_composed(device, "demo/assets/shaders/rain.wgsl", include_str!("shaders/rain.wgsl"), "Rain Shader");
        let depth = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                multisampled: false,
                view_dimension: wgpu::TextureViewDimension::D2,
                sample_type: wgpu::TextureSampleType::Depth,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("rain_layout"),
            entries: &[
                depth(0),
                depth(1),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("rain_pipeline_layout"),
            bind_group_layouts: &[Some(global_layout), Some(&layout)],
            immediate_size: 0,
        });
        // Added to the frame: rain is light, scattered towards the camera.
        let add = wgpu::BlendState {
            color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
            alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
        };
        let pipeline = |entry: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState { module: &shader, entry_point: Some(entry), compilation_options: Default::default(), buffers: &[] },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_rain"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend: Some(add), write_mask: wgpu::ColorWrites::ALL })],
                }),
                primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, cull_mode: None, ..Default::default() },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let drop_pipeline = pipeline("vs_drop");
        let splash_pipeline = pipeline("vs_splash");
        let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("rain_params"),
            contents: bytemuck::bytes_of(&RainParams::new(&Rainfall::default(), Vec3::ZERO)),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        // A depth of 1 everywhere: nothing below, so nothing covers a drop and nothing splashes.
        let no_map_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("rain_no_map"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let no_map = no_map_texture.create_view(&Default::default());
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("rain_no_map_clear") });
        drop(encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("rain_no_map_clear"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &no_map,
                depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        }));
        queue.submit(Some(encoder.finish()));
        Self { drop_pipeline, splash_pipeline, layout, params, no_map, tracks: CameraTracks::default() }
    }

    /// Records this frame's rain into `hdr`, behind what `depth` holds: `rain`'s drops, then its splashes
    /// on `rain_map` (the frame's, or none), lit through the spots' `cookies`, streaked by the camera's
    /// `camera_velocity` as well as by their fall.
    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        global: &wgpu::BindGroup,
        depth: &wgpu::TextureView,
        rain_map: Option<&wgpu::TextureView>,
        hdr: &wgpu::TextureView,
        cookies: &crate::cookies::CookieTextures,
        rain: &Rainfall,
        camera_velocity: Vec3,
    ) {
        queue.write_buffer(&self.params, 0, bytemuck::bytes_of(&RainParams::new(rain, camera_velocity)));
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("rain"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(depth) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(rain_map.unwrap_or(&self.no_map)) },
                wgpu::BindGroupEntry { binding: 2, resource: self.params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&cookies.view) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&cookies.sampler) },
            ],
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Rain Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: hdr,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(0, global, &[]);
        pass.set_bind_group(1, &group, &[]);
        if rain.drops > 0 {
            pass.set_pipeline(&self.drop_pipeline);
            pass.draw(0..6, 0..rain.drops);
        }
        if rain.splash_cells > 0 && rain_map.is_some() {
            pass.set_pipeline(&self.splash_pipeline);
            pass.draw(0..6, 0..rain.splash_cells * rain.splash_cells);
        }
    }
}

impl crate::Renderer {
    /// Draws this frame's rain ([`Self::rainfall`]) into the HDR target, behind what the frame drew: after
    /// the forward pass, before post-processing, for the view whose camera stands at `camera` with the
    /// frame's clock at `clock` (the scene block's). Nothing while it is not raining. Builds the pipelines
    /// the first time.
    pub fn record_rain(&mut self, encoder: &mut wgpu::CommandEncoder, camera: Vec3, clock: f32) {
        let Some(rain) = self.rainfall else { return };
        if self.rain.is_none() {
            let format = self.post.hdr_texture.format();
            self.rain = Some(RainState::new(&self.device, &self.queue, &self.scene.global_bind_group_layout, format));
        }
        let view = self.scene.active_view;
        let Some(state) = self.rain.as_mut() else { return };
        let camera_velocity = state.tracks.velocity(view, camera, clock);
        state.record(
            &self.device,
            &self.queue,
            encoder,
            self.scene.view_bind_group(),
            &self.depth_texture_view,
            self.scene.shadow_rain_layer_view.as_ref(),
            &self.post.hdr_texture_view,
            &self.scene.cookies,
            &rain,
            camera_velocity,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **A view's camera velocity is its own, steady through a frame, and nothing across a cut or a pause.**
    /// The rain streaks the drops by it: a car at 30 m/s draws the near ones out into lines streaming past,
    /// and a camera cut must not draw a screen of lines from where the camera was to where it is.
    #[test]
    fn a_views_camera_velocity_is_its_own_and_nothing_across_a_cut_or_a_pause() {
        let mut tracks = CameraTracks::default();
        let near = |a: Vec3, b: Vec3| (a - b).length() < 1e-3;
        // Its first frame: nothing to measure from.
        assert_eq!(tracks.velocity(None, Vec3::ZERO, 1.0), Vec3::ZERO);
        // 3 m along x in a tenth of a second: 30 m/s — and the same for a second pass of the frame.
        assert!(near(tracks.velocity(None, Vec3::new(3.0, 0.0, 0.0), 1.1), Vec3::new(30.0, 0.0, 0.0)));
        assert!(near(tracks.velocity(None, Vec3::new(3.0, 0.0, 0.0), 1.1), Vec3::new(30.0, 0.0, 0.0)));
        // A second view, standing still far away, is its own camera, and the first keeps its speed.
        assert_eq!(tracks.velocity(Some(0), Vec3::new(500.0, 0.0, 0.0), 1.1), Vec3::ZERO);
        assert_eq!(tracks.velocity(Some(0), Vec3::new(500.0, 0.0, 0.0), 1.2), Vec3::ZERO);
        assert!(near(tracks.velocity(None, Vec3::new(6.0, 0.0, 0.0), 1.2), Vec3::new(30.0, 0.0, 0.0)));
        // A cut: 100 m in a sixtieth of a second is not a camera moving.
        assert_eq!(tracks.velocity(None, Vec3::new(106.0, 0.0, 0.0), 1.2 + 1.0 / 60.0), Vec3::ZERO);
        // And moving on from there, it is measured from where it cut to.
        assert!(near(tracks.velocity(None, Vec3::new(106.0, 0.0, 1.0), 1.3 + 1.0 / 60.0), Vec3::new(0.0, 0.0, 10.0)));
        // A pause of a second: nothing, rather than a second's motion as a frame's.
        assert_eq!(tracks.velocity(None, Vec3::new(106.0, 0.0, 2.0), 2.3 + 1.0 / 60.0), Vec3::ZERO);
    }
}
