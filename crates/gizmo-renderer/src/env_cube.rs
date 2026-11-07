//! The environment as a cube with one roughness per mip, which the forward-only path's lit shader reads.
//!
//! The presets and [`Sky`](crate::frame_uniforms::Sky) are functions of a direction and a roughness, and
//! what they depend on stays the same for a whole frame, usually for a whole game. The forward shader
//! worked them out at every fragment all the same, twice, for the light and for the reflection, and on an
//! Adreno 618 with a floor filling the screen that was most of the frame: 63 fps with them, about 140
//! with a constant in their place. So on the forward-only path they are drawn into this cube when what
//! they depend on changes, mip `k` at roughness `k / (LEVELS - 1)`, and the shader samples it (`ENV_CUBE`
//! in `shader.wgsl`). The desktop path keeps working them out per pixel, next to its reflection probes.

use bytemuck::{Pod, Zeroable};

use crate::gpu_types::SceneUniforms;

/// The sharpest mip's edge, in texels.
pub(crate) const BASE_SIZE: u32 = 128;
/// Mips down to 8 texels. A roughness of 1 is smooth across a face, and 8 is enough to follow it.
pub(crate) const LEVELS: u32 = 5;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// What the cube shows, as `env_bake.wgsl` reads it. Two equal ones draw the same cube.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub(crate) struct EnvInputs {
    zenith: [f32; 4],
    horizon: [f32; 4],
    ground: [f32; 4],
    sun_direction: [f32; 4],
    /// Preset, preset blended towards, [`BASE_SIZE`], [`LEVELS`].
    params: [u32; 4],
    /// x = how far the blend is.
    blend: [f32; 4],
}

impl EnvInputs {
    /// The parts of a frame's uniforms that the environment depends on.
    pub(crate) fn of(u: &SceneUniforms) -> Self {
        Self {
            zenith: u.sky_zenith,
            horizon: u.sky_horizon,
            ground: u.sky_ground,
            sun_direction: u.sun_direction,
            params: [u.environment_preset, u.environment_preset_2, BASE_SIZE, LEVELS],
            blend: [u.environment_blend_t, 0.0, 0.0, 0.0],
        }
    }
}

/// See the [module docs](self).
pub(crate) struct EnvironmentCube {
    /// What the views look into. Only the tests' readback reads it.
    #[cfg_attr(not(test), allow(dead_code))]
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    /// One per mip and face, mip-major: target `i` is face `i % 6` of mip `i / 6`.
    targets: Vec<wgpu::TextureView>,
    inputs: wgpu::Buffer,
    /// What it was last drawn from; `None` until the first time.
    drawn: Option<EnvInputs>,
    /// Built the first time it draws, so a path that never samples the cube never compiles its shader.
    bake: Option<(wgpu::RenderPipeline, wgpu::BindGroup)>,
}

impl EnvironmentCube {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("environment cube"),
            size: wgpu::Extent3d { width: BASE_SIZE, height: BASE_SIZE, depth_or_array_layers: 6 },
            mip_level_count: LEVELS,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("environment cube"),
            dimension: Some(wgpu::TextureViewDimension::Cube),
            ..Default::default()
        });
        let targets = (0..LEVELS * 6)
            .map(|i| {
                texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("environment cube face"),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_mip_level: i / 6,
                    mip_level_count: Some(1),
                    base_array_layer: i % 6,
                    array_layer_count: Some(1),
                    ..Default::default()
                })
            })
            .collect();
        let inputs = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("environment cube inputs"),
            size: std::mem::size_of::<EnvInputs>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self { texture, view, targets, inputs, drawn: None, bake: None }
    }

    /// The whole cube, every mip, for group 0.
    pub(crate) fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// Draws the cube again if what it shows has changed since it was last drawn. True if it did.
    pub(crate) fn update(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        uniforms: &SceneUniforms,
    ) -> bool {
        let inputs = EnvInputs::of(uniforms);
        if self.drawn == Some(inputs) {
            return false;
        }
        queue.write_buffer(&self.inputs, 0, bytemuck::bytes_of(&inputs));
        let buffer = &self.inputs;
        let (pipeline, group) = self.bake.get_or_insert_with(|| bake_pipeline(device, buffer));
        for (i, target) in self.targets.iter().enumerate() {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("environment cube"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                ..Default::default()
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &*group, &[]);
            let i = i as u32;
            pass.draw(0..3, i..i + 1);
        }
        self.drawn = Some(inputs);
        true
    }
}

fn bake_pipeline(device: &wgpu::Device, inputs: &wgpu::Buffer) -> (wgpu::RenderPipeline, wgpu::BindGroup) {
    let source = include_str!("shaders/env_bake.wgsl");
    #[cfg(not(gizmo_forward_only))]
    let shader = crate::pipeline::shaders::load_shader_composed(
        device,
        "demo/assets/shaders/env_bake.wgsl",
        source,
        "Environment Cube Shader",
    );
    #[cfg(gizmo_forward_only)]
    let shader = crate::pipeline::shaders::load_shader_composed_web(device, source, "Environment Cube Shader");
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("environment cube"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("environment cube"),
        layout: &layout,
        entries: &[wgpu::BindGroupEntry { binding: 0, resource: inputs.as_entire_binding() }],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("environment cube"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("environment cube"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState { format: FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState { topology: wgpu::PrimitiveTopology::TriangleList, ..Default::default() },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    (pipeline, group)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uniforms() -> SceneUniforms {
        let mut u: SceneUniforms = Zeroable::zeroed();
        u.sky_zenith = [1.0, 0.0, 0.0, 1.0];
        u.sky_horizon = [0.0, 1.0, 0.0, 0.0];
        u.sky_ground = [0.0, 0.0, 1.0, 0.0];
        u.sun_direction = [0.0, -1.0, 0.0, 1.0];
        u
    }

    /// The parts of a frame the environment does not depend on leave the cube as it is; any it does
    /// depend on redraws it.
    #[test]
    fn only_what_the_environment_depends_on_redraws_it() {
        let a = uniforms();
        let mut moved = a;
        moved.camera_pos = [9.0, 9.0, 9.0, 1.0];
        moved.num_lights = 3;
        assert_eq!(EnvInputs::of(&a), EnvInputs::of(&moved), "a camera or a light is not the sky");
        for change in [
            |u: &mut SceneUniforms| u.sky_horizon[0] = 0.5,
            |u: &mut SceneUniforms| u.sun_direction[0] = 0.3,
            |u: &mut SceneUniforms| u.environment_preset = 2,
            |u: &mut SceneUniforms| u.environment_preset_2 = 1,
            |u: &mut SceneUniforms| u.environment_blend_t = 0.5,
        ] {
            let mut b = a;
            change(&mut b);
            assert_ne!(EnvInputs::of(&a), EnvInputs::of(&b));
        }
    }

    /// Reads one texel of a mip and face back, as linear RGB.
    fn texel(device: &wgpu::Device, queue: &wgpu::Queue, cube: &EnvironmentCube, level: u32, face: u32, x: u32, y: u32) -> [f32; 3] {
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 256,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &cube.texture,
                mip_level: level,
                origin: wgpu::Origin3d { x, y, z: face },
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &staging,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256), rows_per_image: Some(1) },
            },
            wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        );
        queue.submit(Some(encoder.finish()));
        let slice = staging.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        let bytes = slice.get_mapped_range().expect("mapped").to_vec();
        let half = |i: usize| half_to_f32(u16::from_le_bytes([bytes[i * 2], bytes[i * 2 + 1]]));
        [half(0), half(1), half(2)]
    }

    fn half_to_f32(h: u16) -> f32 {
        let sign = if h & 0x8000 != 0 { -1.0 } else { 1.0 };
        let exp = i32::from((h >> 10) & 0x1f);
        let frac = f32::from(h & 0x3ff);
        sign * match exp {
            0 => frac * 2f32.powi(-24),
            31 => f32::INFINITY,
            _ => (1.0 + frac / 1024.0) * 2f32.powi(exp - 15),
        }
    }

    /// **Each face looks where a cube is sampled, and each mip is its roughness.** A sky that is red
    /// straight up, green at the horizon and blue straight down. The sharpest mip's +Y face is red at its
    /// centre, −Y blue, and +X green where it meets the horizon, at its middle row. The blurriest mip is a
    /// roughness of 1, where `sky_gradient` blends halfway: +Y is half red and half green, not red.
    #[test]
    #[ignore = "requires a GPU adapter"]
    fn the_cube_shows_the_sky_by_direction_and_roughness() {
        let _gpu = crate::test_gpu::gpu_lock();
        pollster::block_on(async {
            let Some((device, queue)) = crate::test_gpu::headless_device().await else {
                eprintln!("skipping: no usable GPU adapter");
                return;
            };
            let mut cube = EnvironmentCube::new(&device);
            let u = uniforms();
            let mut encoder = device.create_command_encoder(&Default::default());
            assert!(cube.update(&device, &queue, &mut encoder, &u), "the first frame draws it");
            queue.submit(Some(encoder.finish()));
            let mut encoder = device.create_command_encoder(&Default::default());
            assert!(!cube.update(&device, &queue, &mut encoder, &u), "the same sky does not draw it again");
            queue.submit(Some(encoder.finish()));

            let mid = BASE_SIZE / 2;
            let close = |got: [f32; 3], want: [f32; 3], what: &str| {
                assert!(got.iter().zip(want).all(|(g, w)| (g - w).abs() < 0.1), "{what}: {got:?}, wanted {want:?}");
            };
            close(texel(&device, &queue, &cube, 0, 2, mid, mid), [1.0, 0.0, 0.0], "+Y, sharp");
            close(texel(&device, &queue, &cube, 0, 3, mid, mid), [0.0, 0.0, 1.0], "-Y, sharp");
            close(texel(&device, &queue, &cube, 0, 0, mid, mid), [0.0, 1.0, 0.0], "+X at the horizon, sharp");
            // Near the top of +X the sky is mostly red, at its bottom mostly blue: `t` runs downwards.
            let top = texel(&device, &queue, &cube, 0, 0, mid, 2);
            let bottom = texel(&device, &queue, &cube, 0, 0, mid, BASE_SIZE - 3);
            assert!(top[0] > top[2] && bottom[2] > bottom[0], "+X is upside down: top {top:?}, bottom {bottom:?}");

            let small = (BASE_SIZE >> (LEVELS - 1)) / 2;
            let rough = texel(&device, &queue, &cube, LEVELS - 1, 2, small, small);
            close(rough, [0.5, 0.5, 0.0], "+Y at a roughness of 1");
        });
    }
}
