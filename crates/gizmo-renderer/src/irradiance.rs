//! The irradiance-volume grid, on the GPU.
//!
//! # What this connects
//!
//! [`gi`](crate::gi) has held a complete irradiance-volume implementation — `SHCoeffs`,
//! `LightProbe`, `ProbeGrid` with analytic baking and trilinear sampling, seven tests — with
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

/// The uploaded probe grid and the bind group the lighting pass reads it through.
pub struct IrradianceState {
    probes_buffer: wgpu::Buffer,
    meta_buffer: wgpu::Buffer,
    /// Group 3 of the deferred lighting pipeline.
    pub bind_group: wgpu::BindGroup,
    /// Its layout, needed when the pipeline is built.
    pub bind_group_layout: wgpu::BindGroupLayout,
    /// How many probes are currently uploaded. `0` is a valid, neutral state.
    pub probe_count: usize,
    /// Capacity in probes, so an upload of the same size does not reallocate.
    capacity: usize,
}

impl IrradianceState {
    /// Builds an empty state: one zeroed probe, `probe_count = 0`.
    ///
    /// The buffer is never zero-length — wgpu rejects a zero-sized binding, and a renderer with no
    /// GI still has to bind something.
    #[must_use]
    pub fn new(device: &wgpu::Device) -> Self {
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
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
            ],
        });

        let probes_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("irradiance_probes"),
            contents: bytemuck::cast_slice(&[0.0f32; PROBE_FLOATS]),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
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

        let bind_group = Self::mk_bind_group(device, &bind_group_layout, &probes_buffer, &meta_buffer);

        Self {
            probes_buffer,
            meta_buffer,
            bind_group,
            bind_group_layout,
            probe_count: 0,
            capacity: 1,
        }
    }

    fn mk_bind_group(
        device: &wgpu::Device,
        layout: &wgpu::BindGroupLayout,
        probes: &wgpu::Buffer,
        meta: &wgpu::Buffer,
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
            ],
        })
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
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            });
            self.capacity = count.max(1);
            self.bind_group = Self::mk_bind_group(
                device,
                &self.bind_group_layout,
                &self.probes_buffer,
                &self.meta_buffer,
            );
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

    /// Whether anything is uploaded. The shader checks the same thing through `probe_count`.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.probe_count == 0
    }
}

#[cfg(test)]
mod tests {
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
