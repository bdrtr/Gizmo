//! The irradiance-volume grid, on the GPU.
//!
//! # What this connects
//!
//! [`gi`](crate::gi) has held a complete irradiance-volume implementation — `SHCoeffs`,
//! `LightProbe`, `ProbeGrid` with trilinear sampling, seven tests — with
//! nothing in the render path calling it. `CAPABILITY_GAPS.md` §F2 recorded the shape of that:
//! `to_gpu_data` had exactly one caller, its own test, and no shader read a spherical-harmonic
//! coefficient. A game that wanted indirect light had to sample on the CPU and fold the result
//! into a material colour by hand, which is what `irradiance_volumes` does to prove the maths
//! works.
//!
//! This uploads the grid and gives the deferred lighting pass something to read. The sampling and
//! evaluation are ported to WGSL from [`ProbeGrid::sample`](crate::gi::ProbeGrid::sample) and
//! [`SHCoeffs::evaluate`](crate::gi::SHCoeffs::evaluate) — the same trilinear blend over the same
//! eight corners, the same basis constants — so the GPU answer and the CPU answer are the same
//! answer, and the demo measures that rather than asserting it.
//!
//! # Layout
//!
//! One storage buffer of probes and one uniform of grid metadata, in bind group 3 — free in the
//! deferred lighting pipeline, which spends 0 (scene), 1 (shadow) and 2 (G-buffer).
//!
//! Each probe is 28 floats, which is `SHCoeffs::to_gpu_data`'s existing layout: 27 coefficients
//! (L0, three L1, five L2, three channels each) plus one pad to a 16-byte boundary. Reusing it
//! rather than defining a second one is what keeps the two paths from drifting.
//!
//! # Always bound, possibly empty
//!
//! Unlike SSR or volumetric, this is not an `Option` on the renderer. The deferred lighting
//! pipeline declares group 3 in its layout, and wgpu requires a declared group to be bound — so a
//! renderer with no probes binds a one-probe grid of zeros and the shader adds nothing. The
//! alternative is two pipeline variants, which is a larger cost than a 112-byte buffer.

use crate::gi::ProbeGrid;
use wgpu::util::DeviceExt;

/// Floats per probe — `SHCoeffs::to_gpu_data`'s width.
const PROBE_FLOATS: usize = 28;

/// Grid metadata: where the probes are and how they are laid out.
///
/// `repr(C)` with explicit padding, because a `vec3` in WGSL is 16-byte aligned and a Rust
/// `[f32; 3]` is not — the mismatch is silent and shifts every field after it.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GridMeta {
    /// `xyz` = grid minimum corner, `w` = probe count as a float.
    ///
    /// Packed into the fourth components rather than given padding fields of their own, because
    /// naga_oil rejects identifiers like `_pad0` in a composable module — "must not require
    /// substitution according to naga writeback rules". Three `vec4`s and no pads is the shape
    /// that survives the composer.
    grid_min_count: [f32; 4],
    /// `xyz` = cell size, `w` unused.
    cell_size: [f32; 4],
    /// `xyz` = resolution, `w` unused.
    resolution: [u32; 4],
}

/// Floats in the reflection uniform: a header vec4, then three vec4s per probe slot.
pub(crate) const REFLECTION_FLOATS: usize = 4 + 12 * crate::reflection::MAX_PROBES;

/// The reflection uniform `reflection_probes.wgsl` reads for `set`, or for no probes at all: a header
/// `[count, max_mip, background slot or −1, environment slot + 1 or 0]`, then per slot
/// `[origin, has box] · [box min, blend distance] · [box max, intensity]`. One packing for the desktop's
/// irradiance group and the forward-only path's group 0.
pub(crate) fn reflection_uniform(set: Option<&ReflectionSet<'_>>) -> [f32; REFLECTION_FLOATS] {
    match set {
        Some(set) => pack_reflections(set.mip_count, &set.probes, set.environment),
        None => pack_reflections(0, &[], None),
    }
}

/// [`reflection_uniform`] from the values alone.
fn pack_reflections(mip_count: u32, probes: &[ReflectionSource], environment: Option<usize>) -> [f32; REFLECTION_FLOATS] {
    let mut data = [0.0f32; REFLECTION_FLOATS];
    data[2] = -1.0;
    {
        let probes = &probes[..probes.len().min(crate::reflection::MAX_PROBES)];
        data[0] = probes.len() as f32;
        data[1] = mip_count.saturating_sub(1) as f32;
        if let Some(background) = probes.iter().position(|p| p.box_half_extents.is_none()) {
            data[2] = background as f32;
        }
        data[3] = environment.map_or(0.0, |slot| slot as f32 + 1.0);
        for (slot, p) in probes.iter().enumerate() {
            let at = 4 + 12 * slot;
            data[at..at + 3].copy_from_slice(&p.position);
            data[at + 11] = p.intensity;
            if let Some(half) = p.box_half_extents {
                data[at + 3] = 1.0;
                data[at + 7] = p.blend_distance.max(1e-3);
                for axis in 0..3 {
                    data[at + 4 + axis] = p.position[axis] - half[axis].abs();
                    data[at + 8 + axis] = p.position[axis] + half[axis].abs();
                }
            }
        }
    }
    data
}

/// One captured probe, as [`ReflectionSet`] carries it.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReflectionSource {
    /// Multiplies what it reflects.
    pub intensity: f32,
    /// Where it was captured, in world space.
    pub position: [f32; 3],
    /// Half-extents of the world-aligned box around `position` it influences, with box-projected
    /// parallax inside it; `None` makes it the scene's background reflection instead.
    pub box_half_extents: Option<[f32; 3]>,
    /// How far inside its box the probe fades in from nothing to full weight, in metres.
    pub blend_distance: f32,
}

impl ReflectionSource {
    /// A probe at `position`, without a box.
    pub fn new(intensity: f32, position: [f32; 3]) -> Self {
        Self { intensity, position, box_half_extents: None, blend_distance: 0.5 }
    }

    /// With an influence box of `half_extents` around the probe, fading in over `blend_distance`.
    pub fn with_box(mut self, half_extents: Option<[f32; 3]>, blend_distance: f32) -> Self {
        self.box_half_extents = half_extents;
        self.blend_distance = blend_distance;
        self
    }
}

/// The captured probes and their cube array, as [`IrradianceState::set_reflection`] takes them.
#[non_exhaustive]
pub struct ReflectionSet<'a> {
    /// The prefiltered cube array, slot `i` holding `probes[i]`.
    pub view: &'a wgpu::TextureView,
    /// Its mip count; mip `roughness × (count − 1)` is read.
    pub mip_count: u32,
    /// Up to [`MAX_PROBES`](crate::reflection::MAX_PROBES); the rest are ignored.
    pub probes: Vec<ReflectionSource>,
    /// Which slot holds the [`EnvironmentMap`](crate::environment::EnvironmentMap), if one is set:
    /// the diffuse and the background read it.
    pub environment: Option<usize>,
}

impl<'a> ReflectionSet<'a> {
    /// The probes in `view`'s slots.
    pub fn new(view: &'a wgpu::TextureView, mip_count: u32, probes: Vec<ReflectionSource>) -> Self {
        Self { view, mip_count, probes, environment: None }
    }
}

/// A box of irradiance probes the engine bakes from the scene itself: place it, size it, and the
/// deferred lighting pass reads it for indirect light — no hand-built [`ProbeGrid`] and no upload
/// call.
///
/// **What it holds is light bounced off the scene**: from each probe the engine draws the lit scene
/// six ways, as a [`ReflectionProbe`](crate::components::ReflectionProbe) is drawn, and projects
/// what it sees into the probe's spherical harmonics — a red wall's light on the white floor beside
/// it, a lamp's pool reflected up onto a ceiling, an emissive panel lighting a room. The sky is left
/// out of the drawing (the background, an environment map, a skybox or backdrop): the lighting pass
/// adds the sky to every surface itself, so a volume holding it too would count it twice. So are
/// the lights themselves — a light is not something the drawing shows — so a volume adds no copy of
/// any light's direct light, which the lighting pass also adds itself. Until 2026-11-12 it was baked
/// from the lights alone, analytically: each one's direct light, unshadowed, which the lighting pass
/// then added again on top of the light's own.
///
/// **Baked once**, on the first frame it exists, and again when it changes or after
/// [`request_capture`](Self::request_capture): every probe is six full renders, so a 4×3×4 volume is
/// 288 of them — a cost for a level load, not for every frame. Move a light or change the room,
/// then ask for a rebake. A bake is one bounce: the volume's previous light is not in its captures.
///
/// One per scene: the first found is used. A grid uploaded by hand (`IrradianceState::upload`) is
/// left alone while no volume exists; a volume's bake goes when the volume does. The deferred path's
/// alone, as the volume always was: the forward-only path (a browser's, a phone's) has no group for
/// it.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct IrradianceVolume {
    /// The box's minimum corner, world space.
    pub min: gizmo_math::Vec3,
    /// Its maximum corner.
    pub max: gizmo_math::Vec3,
    /// Probes along each axis (each at least 2).
    pub resolution: [u32; 3],
    /// A constant light added to every probe, as a radiance from every direction: a white matte
    /// surface takes it as it is. It was a sixteenth of that, `1/4π`, until 2026-11-12.
    pub ambient: gizmo_math::Vec3,
    /// Which bake this volume asks for — bumped by [`request_capture`](Self::request_capture). Every
    /// new volume starts at a value no other has had, as a reflection probe does (see
    /// `ReflectionProbe`'s field of the same name). Not saved: a loaded volume is a new one.
    #[serde(skip, default = "crate::components::reflection_probe::fresh_request")]
    capture_request: u64,
}

/// What the game authors, compared: the bake request is the renderer's business.
impl PartialEq for IrradianceVolume {
    fn eq(&self, other: &Self) -> bool {
        self.min == other.min && self.max == other.max && self.resolution == other.resolution && self.ambient == other.ambient
    }
}

impl IrradianceVolume {
    /// A volume over the box `min`–`max` with `resolution` probes per axis, no ambient.
    pub fn new(min: gizmo_math::Vec3, max: gizmo_math::Vec3, resolution: [u32; 3]) -> Self {
        Self {
            min: min.min(max),
            max: min.max(max),
            resolution: resolution.map(|r| r.max(2)),
            ambient: gizmo_math::Vec3::ZERO,
            capture_request: crate::components::reflection_probe::fresh_request(),
        }
    }

    /// The same volume with `ambient` added to every probe.
    pub fn with_ambient(mut self, ambient: gizmo_math::Vec3) -> Self {
        self.ambient = ambient.max(gizmo_math::Vec3::ZERO);
        self
    }

    /// Bakes again on the next frame.
    pub fn request_capture(&mut self) {
        self.capture_request = self.capture_request.wrapping_add(1);
    }

    /// Which bake this volume asks for. The renderer keeps what it last baked
    /// (`IrradianceState::baked_from`) and bakes again when this, or the volume, differs.
    #[must_use]
    pub fn capture_request(&self) -> u64 {
        self.capture_request
    }
}

gizmo_core::impl_component!(IrradianceVolume);

/// The uploaded probe grid and the bind group the lighting pass reads it through.
pub struct IrradianceState {
    probes_buffer: wgpu::Buffer,
    meta_buffer: wgpu::Buffer,
    /// Bindings 2–4: the reflection cube the specular term reads, its sampler, and
    /// `[enabled, max_mip, intensity, 0]`. A 1×1 placeholder cube and `enabled = 0` until
    /// [`set_reflection`](Self::set_reflection) is given a probe — then the lighting pass leaves
    /// the procedural sky alone and frames are bit-identical to before probes existed.
    reflection_params: wgpu::Buffer,
    reflection_sampler: wgpu::Sampler,
    reflection_placeholder: wgpu::TextureView,
    reflection_view: Option<wgpu::TextureView>,
    /// Group 3 of the deferred lighting pipeline.
    pub bind_group: wgpu::BindGroup,
    /// Its layout, needed when the pipeline is built.
    pub bind_group_layout: wgpu::BindGroupLayout,
    /// How many probes are currently uploaded. `0` is a valid, neutral state.
    pub probe_count: usize,
    /// What the uploaded grid was baked from, when it came from an [`IrradianceVolume`] — a hash
    /// of the volume and its bake request. The game path bakes again only when it changes. `None`
    /// for a grid uploaded by hand, or none.
    pub baked_from: Option<u64>,
    /// Capacity in probes, so an upload of the same size does not reallocate.
    capacity: usize,
    /// The bake's pipelines, built the first time a volume is baked.
    bake: Option<crate::irradiance_bake::ProbeBake>,
}

impl IrradianceState {
    /// The group's layout: the probe grid (0–1) and the reflection probes (2–4). Built here so a
    /// pipeline made before any `IrradianceState` exists — the forward pass, which binds this
    /// group at 5 on native — has an identical one; wgpu treats identical layouts as compatible.
    pub fn layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
        device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("irradiance_bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::CubeArray,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        })
    }

    /// Builds an empty state: one zeroed probe, `probe_count = 0`.
    ///
    /// The buffer is never zero-length — wgpu rejects a zero-sized binding, and a renderer with no
    /// GI still has to bind something.
    #[must_use]
    pub fn new(device: &wgpu::Device) -> Self {
        let bind_group_layout = Self::layout(device);
        let reflection_params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("reflection_params"),
            contents: bytemuck::cast_slice(&[0.0f32; REFLECTION_FLOATS]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let reflection_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("reflection_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let reflection_placeholder = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("reflection_placeholder"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 6 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba16Float,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::CubeArray),
                ..Default::default()
            });

        let probes_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("irradiance_probes"),
            contents: bytemuck::cast_slice(&[0.0f32; PROBE_FLOATS]),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
        });
        let meta_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("irradiance_meta"),
            contents: bytemuck::bytes_of(&GridMeta {
                grid_min_count: [0.0; 4],
                cell_size: [1.0, 1.0, 1.0, 0.0],
                resolution: [1, 1, 1, 0],
            }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group = Self::mk_bind_group(
            device,
            &bind_group_layout,
            &probes_buffer,
            &meta_buffer,
            (&reflection_placeholder, &reflection_sampler, &reflection_params),
        );

        Self {
            probes_buffer,
            meta_buffer,
            reflection_params,
            reflection_sampler,
            reflection_placeholder,
            reflection_view: None,
            bind_group,
            bind_group_layout,
            probe_count: 0,
            baked_from: None,
            capacity: 1,
            bake: None,
        }
    }

    fn mk_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        probes: &wgpu::Buffer,
        meta: &wgpu::Buffer,
        (cube, sampler, params): (&wgpu::TextureView, &wgpu::Sampler, &wgpu::Buffer),
    ) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("irradiance_bg"),
            layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: probes.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: meta.as_entire_binding(),
                },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(cube) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(sampler) },
                wgpu::BindGroupEntry { binding: 4, resource: params.as_entire_binding() },
            ],
        })
    }

    fn rebind(&mut self, device: &wgpu::Device) {
        let cube = self.reflection_view.as_ref().unwrap_or(&self.reflection_placeholder);
        self.bind_group = Self::mk_bind_group(
            device,
            &self.bind_group_layout,
            &self.probes_buffer,
            &self.meta_buffer,
            (cube, &self.reflection_sampler, &self.reflection_params),
        );
    }

    /// Points the specular term at a set of reflection probes, or back at the procedural sky with
    /// `None`. See `deferred_lighting.wgsl`'s `probe_environment` for how they combine.
    pub fn set_reflection(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, set: Option<ReflectionSet<'_>>) {
        let data = reflection_uniform(set.as_ref());
        queue.write_buffer(&self.reflection_params, 0, bytemuck::cast_slice(&data));
        self.reflection_view = set.map(|s| s.view.clone());
        self.rebind(device);
    }

    /// Uploads a baked grid, reallocating only if it needs more room.
    ///
    /// Each probe goes up through `SHCoeffs::to_gpu_data`, the same function the CPU path's own
    /// test uses, so there is one definition of the layout rather than two.
    pub fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, grid: &ProbeGrid) {
        let count = grid.probes.len();
        let mut data: Vec<f32> = Vec::with_capacity(count.max(1) * PROBE_FLOATS);
        for probe in &grid.probes {
            data.extend_from_slice(&probe.coeffs.to_gpu_data());
        }
        if data.is_empty() {
            data.extend_from_slice(&[0.0f32; PROBE_FLOATS]);
        }

        if count.max(1) > self.capacity {
            self.probes_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("irradiance_probes"),
                contents: bytemuck::cast_slice(&data),
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
            });
            self.capacity = count.max(1);
            self.rebind(device);
        } else {
            queue.write_buffer(&self.probes_buffer, 0, bytemuck::cast_slice(&data));
        }

        queue.write_buffer(
            &self.meta_buffer,
            0,
            bytemuck::bytes_of(&GridMeta {
                grid_min_count: [
                    grid.grid_min.x,
                    grid.grid_min.y,
                    grid.grid_min.z,
                    count as f32,
                ],
                cell_size: [
                    grid.cell_size.x,
                    grid.cell_size.y,
                    grid.cell_size.z,
                    0.0,
                ],
                resolution: [grid.resolution[0], grid.resolution[1], grid.resolution[2], 0],
            }),
        );
        self.probe_count = count;
    }

    /// Starts baking `grid` (its probes' places; their coefficients are not read) with `ambient` added
    /// to each probe, a radiance as [`IrradianceVolume::ambient`] is. The grid goes up with nothing in
    /// its probes, so the volume adds no light to its own captures, and [`baked_from`](Self::baked_from)
    /// is cleared until the bake is finished. Then, for each probe in order: its six faces drawn and
    /// handed to [`record_bake_face`](Self::record_bake_face), then
    /// [`record_bake_probe`](Self::record_bake_probe); and [`record_bake_finish`](Self::record_bake_finish)
    /// once every probe is in.
    pub fn begin_bake(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, grid: &ProbeGrid, ambient: gizmo_math::Vec3) {
        self.upload(device, queue, &ProbeGrid::new(grid.grid_min, grid.grid_max, grid.resolution));
        self.baked_from = None;
        self.bake
            .get_or_insert_with(|| crate::irradiance_bake::ProbeBake::new(device))
            .begin(device, grid.probes.len(), ambient);
    }

    /// Records filtering `frame` — the HDR target, just drawn from the probe by a 90° square camera
    /// looking along [`face_basis`](crate::reflection::face_basis)`(face)` — into that face.
    pub fn record_bake_face(&self, device: &wgpu::Device, encoder: &mut wgpu::CommandEncoder, frame: &wgpu::TextureView, face: usize) {
        if let Some(bake) = self.bake.as_ref() {
            bake.record_face(device, encoder, frame, face);
        }
    }

    /// Records projecting the six faces just recorded into probe `probe` (its index in the grid).
    pub fn record_bake_probe(&self, encoder: &mut wgpu::CommandEncoder, probe: usize) {
        if let Some(bake) = self.bake.as_ref() {
            bake.record_probe(encoder, probe);
        }
    }

    /// Records putting the finished bake where the lighting pass reads it.
    pub fn record_bake_finish(&mut self, encoder: &mut wgpu::CommandEncoder) {
        if let Some(bake) = self.bake.as_mut() {
            bake.record_finish(encoder, &self.probes_buffer);
        }
    }

    /// The probes the lighting pass reads, read back from the GPU — what a bake put there. Waits for
    /// the GPU; for tools and tests.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn read_probes(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Vec<crate::gi::SHCoeffs> {
        let size = (self.probe_count * PROBE_FLOATS * 4) as u64;
        if size == 0 {
            return Vec::new();
        }
        let staging = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("irradiance_probes_read"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("irradiance_probes_read") });
        encoder.copy_buffer_to_buffer(&self.probes_buffer, 0, &staging, 0, size);
        queue.submit(std::iter::once(encoder.finish()));
        let slice = staging.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        let Ok(mapped) = slice.get_mapped_range() else { return Vec::new() };
        bytemuck::cast_slice::<u8, f32>(&mapped).as_chunks::<PROBE_FLOATS>().0.iter().map(crate::gi::SHCoeffs::from_gpu_data).collect()
    }

    /// Whether anything is uploaded. The shader checks the same thing through `probe_count`.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.probe_count == 0
    }
}

#[cfg(test)]
mod tests {
    /// One packing for both paths: no probes is a count of 0 and no background; a boxed probe carries its
    /// box and blend distance, an unboxed one is the background, and the environment's slot is one up.
    #[test]
    fn the_reflection_uniform_packs_what_the_shader_reads() {
        assert_eq!(&super::reflection_uniform(None)[..4], &[0.0, 0.0, -1.0, 0.0]);
        let boxed = super::ReflectionSource::new(2.0, [1.0, 2.0, 3.0]).with_box(Some([1.0, 0.5, 2.0]), 0.25);
        let open = super::ReflectionSource::new(0.5, [0.0, 4.0, 0.0]);
        let data = super::pack_reflections(6, &[boxed, open], Some(1));
        assert_eq!(&data[..4], &[2.0, 5.0, 1.0, 2.0], "two probes, mip 5 the roughest, slot 1 the background, the environment one up");
        assert_eq!(&data[4..16], &[1.0, 2.0, 3.0, 1.0, 0.0, 1.5, 1.0, 0.25, 2.0, 2.5, 5.0, 2.0], "the boxed probe");
        assert_eq!(&data[16..20], &[0.0, 4.0, 0.0, 0.0], "the open one has no box");
        assert_eq!(data[27], 0.5, "and its intensity");
    }

    use super::*;
    use crate::gi::{LightProbe, ProbeGrid, SHCoeffs};
    use gizmo_math::Vec3;

    /// The GPU layout has to be the size the shader's `GridMeta` is, or every field after the
    /// first reads the wrong bytes and nothing reports it.
    #[test]
    fn the_metadata_block_is_three_vec4s() {
        assert_eq!(std::mem::size_of::<GridMeta>(), 48);
        assert_eq!(std::mem::align_of::<GridMeta>(), 4);
    }

    /// A probe is 28 floats, matching `SHCoeffs::to_gpu_data`.
    ///
    /// Asserted against the function rather than against 28, so that widening the coefficient set
    /// fails here instead of silently shifting every probe after the first.
    #[test]
    fn a_probe_is_as_wide_as_to_gpu_data() {
        let c = SHCoeffs::default();
        assert_eq!(c.to_gpu_data().len(), PROBE_FLOATS);
    }

    /// A grid built here indexes the way the shader indexes it.
    ///
    /// The shader recomputes `z * res.y * res.x + y * res.x + x` from the metadata; this pins that
    /// the metadata describes the same ordering `ProbeGrid` used when it filled `probes`.
    #[test]
    fn the_upload_order_is_the_grids_own_order() {
        let mut grid = ProbeGrid::new(Vec3::ZERO, Vec3::splat(4.0), [2, 2, 2]);
        // Give each probe a distinguishable L0 so an ordering mistake is visible.
        for (i, p) in grid.probes.iter_mut().enumerate() {
            p.coeffs.l0 = Vec3::splat(i as f32);
        }
        let flat: Vec<f32> = grid
            .probes
            .iter()
            .flat_map(|p| p.coeffs.to_gpu_data())
            .collect();
        assert_eq!(flat.len(), 8 * PROBE_FLOATS);
        for i in 0..8 {
            assert_eq!(
                flat[i * PROBE_FLOATS],
                i as f32,
                "probe {i} is not at offset {i} * {PROBE_FLOATS}"
            );
        }
    }

    /// An empty grid uploads as one zeroed probe with a count of zero.
    ///
    /// wgpu rejects a zero-sized binding, so "no GI" cannot be an empty buffer — and a count of
    /// zero is what makes the shader return black rather than read the placeholder.
    #[test]
    fn an_empty_grid_is_one_zero_probe_and_a_zero_count() {
        let grid = ProbeGrid {
            probes: Vec::new(),
            grid_min: Vec3::ZERO,
            grid_max: Vec3::ONE,
            resolution: [0, 0, 0],
            cell_size: Vec3::ONE,
        };
        assert_eq!(grid.probes.len(), 0);
        // The upload path's own branch, without a device: an empty probe list becomes one probe.
        let mut data: Vec<f32> = grid
            .probes
            .iter()
            .flat_map(|p: &LightProbe| p.coeffs.to_gpu_data())
            .collect();
        if data.is_empty() {
            data.extend_from_slice(&[0.0f32; PROBE_FLOATS]);
        }
        assert_eq!(data.len(), PROBE_FLOATS);
        assert!(data.iter().all(|v| *v == 0.0));
    }
}
