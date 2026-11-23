use gizmo_math::Vec3;
use gizmo_physics_core::BodyHandle;

use crate::coupling::{RigidImpulse, RigidReaction};
use crate::soft_body::SoftBodyMesh;

/// The elastic forces of tetrahedral FEM soft bodies, computed on the GPU.
///
/// Compiled only under this crate's `gpu_physics` feature, which is off by default. The engine's
/// own driver [`crate::system::soft_body_step_system`] steps on the CPU, so this is reached by
/// building one and calling [`GpuCompute::step_soft_bodies`].
///
/// **What runs where.** The GPU computes what a CPU step spends its time on: every element's
/// Neo-Hookean stress and the four nodal forces it gives, one invocation per element, each with
/// its own body's material. The rest is [`SoftBodyMesh::step_coupled`]'s own code. The forces are
/// read back and summed into their nodes in element order, and the nodes are integrated, damped
/// and collided by the function a CPU step ends in. So a GPU step is a CPU step with one part
/// computed elsewhere, and differs from it only by that part's rounding — `tests/gpu_step.rs`
/// holds the two together.
///
/// Until 2026-11-23 the shader had its own integrator, a copy of the CPU's that had stopped
/// following it:
/// - every body took the first body's `mu`, `lambda` and `damping`;
/// - the shader had a floor built in at y = 0.1 m;
/// - an element stopped pushing at J < 0.05, where the CPU's limit is 10⁻⁴;
/// - a massless node still fell;
/// - a node that crept into a collider was never put back on its surface;
/// - the forces were summed as fixed-point integers, which overflowed past about 2 × 10⁵ N;
/// - every step started a thread to poll the device.
pub struct GpuCompute {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    /// `None` until the first step; grown, never shrunk, as the bodies grow.
    buffers: Option<Buffers>,
}

/// The buffers of one capacity, and the bind group over them.
struct Buffers {
    node_capacity: usize,
    element_capacity: usize,
    positions: wgpu::Buffer,
    elements: wgpu::Buffer,
    params: wgpu::Buffer,
    /// Four `vec4` per element: the forces the shader writes.
    forces: wgpu::Buffer,
    /// What `forces` is copied into, to be read back.
    staging: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

/// The size of one element's forces in the `forces` buffer: four `vec4<f32>`.
const FORCES_PER_ELEMENT: usize = 4 * std::mem::size_of::<[f32; 4]>();

impl GpuCompute {
    /// Builds the compute path on a device of its own, from the system's high-performance adapter.
    ///
    /// # Errors
    ///
    /// [`crate::SoftBodyError::NoCompatibleAdapter`] if no adapter could be acquired, and
    /// [`crate::SoftBodyError::DeviceRequestFailed`] if requesting the device failed.
    pub async fn new() -> Result<Self, crate::SoftBodyError> {
        let instance = wgpu::Instance::default();
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
                // wgpu 30. Limit bucketing is a browser-fingerprinting defence; this is a native
                // compute path talking to its own hardware, and bucketing would cost real limits.
                apply_limit_buckets: false,
            })
            .await
            .map_err(|_| crate::SoftBodyError::NoCompatibleAdapter)?;
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("Gizmo Physics Compute Device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                experimental_features: wgpu::ExperimentalFeatures::default(),
                memory_hints: wgpu::MemoryHints::default(),
                trace: wgpu::Trace::Off,
            })
            .await
            .map_err(|e| crate::SoftBodyError::DeviceRequestFailed(e.to_string()))?;
        Ok(Self::with_device(device, queue))
    }

    /// Builds the compute path on a device the caller already has — a renderer's, say.
    pub fn with_device(device: wgpu::Device, queue: wgpu::Queue) -> Self {
        let shader = device.create_shader_module(wgpu::include_wgsl!("soft_body.wgsl"));
        let entry = |binding: u32, ty: wgpu::BufferBindingType| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: false, min_binding_size: None },
            count: None,
        };
        let read = wgpu::BufferBindingType::Storage { read_only: true };
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Soft Body Bind Group Layout"),
            entries: &[
                entry(0, read),
                entry(1, read),
                entry(2, wgpu::BufferBindingType::Uniform),
                entry(3, wgpu::BufferBindingType::Storage { read_only: false }),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Soft Body Pipeline Layout"),
            bind_group_layouts: &[Some(&bind_group_layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("Soft Body Element Forces"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: Default::default(),
            cache: None,
        });
        Self { device, queue, pipeline, bind_group_layout, buffers: None }
    }

    /// The buffers, grown to hold `nodes` nodes and `elements` elements: to the larger of what is
    /// needed, twice what was there, and 1024.
    fn buffers_for(&mut self, nodes: usize, elements: usize) -> &Buffers {
        let fits = self.buffers.as_ref().is_some_and(|b| b.node_capacity >= nodes && b.element_capacity >= elements);
        if !fits {
            let (old_nodes, old_elements) = self.buffers.as_ref().map_or((0, 0), |b| (b.node_capacity, b.element_capacity));
            let node_capacity = nodes.max(old_nodes * 2).max(1024);
            let element_capacity = elements.max(old_elements * 2).max(1024);
            let buffer = |label: &str, size: usize, usage: wgpu::BufferUsages| {
                self.device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: size as u64, usage, mapped_at_creation: false })
            };
            let positions = buffer(
                "Soft Body Positions",
                node_capacity * std::mem::size_of::<[f32; 4]>(),
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            );
            let elements = buffer(
                "Soft Body Elements",
                element_capacity * std::mem::size_of::<GpuTetrahedron>(),
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            );
            let params = buffer(
                "Soft Body Parameters",
                std::mem::size_of::<GpuParameters>(),
                wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            );
            let forces = buffer(
                "Soft Body Element Forces",
                element_capacity * FORCES_PER_ELEMENT,
                wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
            );
            let staging = buffer(
                "Soft Body Force Readback",
                element_capacity * FORCES_PER_ELEMENT,
                wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            );
            let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Soft Body Bind Group"),
                layout: &self.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: positions.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: elements.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 2, resource: params.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 3, resource: forces.as_entire_binding() },
                ],
            });
            self.buffers = Some(Buffers { node_capacity, element_capacity, positions, elements, params, forces, staging, bind_group });
        }
        self.buffers.as_ref().expect("just made")
    }

    /// Advances every soft body by one step of `dt` seconds under `gravity` (m/s²), against
    /// `rigid_colliders` as immovable geometry: [`SoftBodyMesh::step`], with the element forces
    /// computed on the GPU. Each body keeps its own material and damping.
    ///
    /// The meshes are in world space, so the [`BodyHandle`] and `Transform` carried with each are
    /// not read.
    ///
    /// # Errors
    ///
    /// [`crate::SoftBodyError::NodeOffsetOverflow`] if the bodies hold more than `u32::MAX` nodes
    /// together, and [`crate::SoftBodyError::ReadbackFailed`] if the forces could not be read back
    /// (the device was lost, say). Either way the meshes are left as they were.
    pub fn step_soft_bodies(
        &mut self,
        soft_bodies: &mut [(BodyHandle, SoftBodyMesh, gizmo_physics_core::components::Transform)],
        rigid_colliders: &[(BodyHandle, gizmo_physics_core::components::Transform, gizmo_physics_core::components::Collider)],
        dt: f32,
        gravity: Vec3,
    ) -> Result<(), crate::SoftBodyError> {
        let mut sink = Vec::new();
        self.step_soft_bodies_coupled(soft_bodies, rigid_colliders, &[], &mut sink, dt, gravity)?;
        debug_assert!(sink.is_empty(), "an empty reaction table can only produce immovable contacts");
        Ok(())
    }

    /// [`step_soft_bodies`](Self::step_soft_bodies) with two-way coupling:
    /// [`SoftBodyMesh::step_coupled`]'s step, `reactions` and `out_impulses` meaning what they
    /// mean there.
    ///
    /// # Errors
    ///
    /// As [`step_soft_bodies`](Self::step_soft_bodies).
    pub fn step_soft_bodies_coupled(
        &mut self,
        soft_bodies: &mut [(BodyHandle, SoftBodyMesh, gizmo_physics_core::components::Transform)],
        rigid_colliders: &[(BodyHandle, gizmo_physics_core::components::Transform, gizmo_physics_core::components::Collider)],
        reactions: &[(BodyHandle, RigidReaction)],
        out_impulses: &mut Vec<RigidImpulse>,
        dt: f32,
        gravity: Vec3,
    ) -> Result<(), crate::SoftBodyError> {
        // 1. Every body's nodes and elements in one array each, the elements' corners rebased
        //    onto the node array and each carrying its body's material.
        let mut positions: Vec<[f32; 4]> = Vec::new();
        let mut elements: Vec<GpuTetrahedron> = Vec::new();
        for (_, sb, _) in soft_bodies.iter() {
            let base = u32::try_from(positions.len()).map_err(|_| crate::SoftBodyError::NodeOffsetOverflow)?;
            base.checked_add(u32::try_from(sb.nodes.len()).map_err(|_| crate::SoftBodyError::NodeOffsetOverflow)?)
                .ok_or(crate::SoftBodyError::NodeOffsetOverflow)?;
            positions.extend(sb.nodes.iter().map(|n| [n.position.x, n.position.y, n.position.z, 0.0]));
            elements.extend(sb.elements.iter().map(|e| GpuTetrahedron {
                indices: e.node_indices.map(|i| i + base),
                inv_rest_matrix_col0: e.inv_rest_matrix.x_axis.into(),
                pad0: 0.0,
                inv_rest_matrix_col1: e.inv_rest_matrix.y_axis.into(),
                pad1: 0.0,
                inv_rest_matrix_col2: e.inv_rest_matrix.z_axis.into(),
                pad2: 0.0,
                rest_volume: e.rest_volume,
                mu: sb.mu,
                lambda: sb.lambda,
                pad3: 0.0,
            }));
        }

        // 2. The forces, on the GPU.
        let forces = if elements.is_empty() { Vec::new() } else { self.element_forces(&positions, &elements)? };

        // 3. The rest of each body's step, on the CPU, as a CPU step takes it.
        let mut first = 0;
        for (_, sb, _) in soft_bodies.iter_mut() {
            let mine = &forces[first * 4..(first + sb.elements.len()) * 4];
            first += sb.elements.len();
            let mut node_forces = sb.weights(gravity);
            let pushed = sb.elements.iter().zip(mine.as_chunks::<4>().0).map(|(e, f)| {
                if f[0][3] == 0.0 {
                    return None; // crushed past `J_EPS`
                }
                let force = |k: usize| Vec3::new(f[k][0], f[k][1], f[k][2]);
                crate::soft_body::finite_forces(e.node_indices.map(|i| i as usize), [force(0), force(1), force(2), force(3)])
            });
            crate::soft_body::add_element_forces(&mut node_forces, pushed);
            sb.advance_nodes(&node_forces, dt, rigid_colliders, reactions, out_impulses);
        }
        Ok(())
    }

    /// Uploads the nodes and elements, runs the force pass and reads its result back: four
    /// `[x, y, z, pushes]` per element.
    fn element_forces(&mut self, positions: &[[f32; 4]], elements: &[GpuTetrahedron]) -> Result<Vec<[f32; 4]>, crate::SoftBodyError> {
        let params = GpuParameters { num_elements: elements.len() as u32, pad: [0; 3] };
        let pipeline = self.pipeline.clone();
        let (device, queue) = (self.device.clone(), self.queue.clone());
        let buffers = self.buffers_for(positions.len(), elements.len());
        queue.write_buffer(&buffers.positions, 0, bytemuck::cast_slice(positions));
        queue.write_buffer(&buffers.elements, 0, bytemuck::cast_slice(elements));
        queue.write_buffer(&buffers.params, 0, bytemuck::bytes_of(&params));
        let bytes = (elements.len() * FORCES_PER_ELEMENT) as u64;
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("Soft Body Forces") });
        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("Soft Body Forces"), timestamp_writes: None });
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &buffers.bind_group, &[]);
            pass.dispatch_workgroups(params.num_elements.div_ceil(64), 1, 1);
        }
        encoder.copy_buffer_to_buffer(&buffers.forces, 0, &buffers.staging, 0, bytes);
        let submitted = queue.submit(Some(encoder.finish()));

        // 4. Wait for it here, on this thread: the copy has to finish before the mapping can.
        let slice = buffers.staging.slice(0..bytes);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        device
            .poll(wgpu::PollType::Wait { submission_index: Some(submitted), timeout: None })
            .map_err(|e| crate::SoftBodyError::ReadbackFailed(e.to_string()))?;
        match rx.recv() {
            Ok(Ok(())) => {}
            Ok(Err(e)) => return Err(crate::SoftBodyError::ReadbackFailed(e.to_string())),
            Err(_) => return Err(crate::SoftBodyError::ReadbackFailed("the mapping never answered".to_string())),
        }
        let forces = {
            let mapped = slice.get_mapped_range().map_err(|e| crate::SoftBodyError::ReadbackFailed(e.to_string()))?;
            bytemuck::cast_slice::<u8, [f32; 4]>(&mapped).to_vec()
        };
        buffers.staging.unmap();
        Ok(forces)
    }
}

/// One tetrahedral element as the `Tetrahedron` struct of `soft_body.wgsl` lays it out: 80 bytes,
/// the three columns of `Dm⁻¹` each in a 16-byte slot, then `rest_volume`, `mu` and `lambda`.
///
/// The `pad*` words carry nothing. `pad0`–`pad2` close the columns' slots, as WGSL's `vec3<f32>`
/// alignment asks; `pad3` brings the struct to 80. Removing one shifts every field after it, and
/// `tests/wgsl_layout.rs` checks the shader's side with naga.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuTetrahedron {
    /// The four corner nodes, as indices into all the bodies' nodes together: each body's own
    /// [`crate::soft_body::Tetrahedron::node_indices`] plus where its nodes start.
    pub indices: [u32; 4],
    /// First column of the inverse rest shape matrix `Dm⁻¹`, in 1/m.
    pub inv_rest_matrix_col0: [f32; 3],
    /// Unused; puts the next column at byte 32.
    pub pad0: f32,
    /// Second column of `Dm⁻¹`, in 1/m.
    pub inv_rest_matrix_col1: [f32; 3],
    /// Unused; puts the next column at byte 48.
    pub pad1: f32,
    /// Third column of `Dm⁻¹`, in 1/m.
    pub inv_rest_matrix_col2: [f32; 3],
    /// Unused; puts `rest_volume` at byte 64.
    pub pad2: f32,
    /// The element's rest volume `V₀`, in m³: it scales the forces it gives.
    pub rest_volume: f32,
    /// Its body's shear modulus μ ([`SoftBodyMesh::mu`]), in pascals.
    pub mu: f32,
    /// Its body's first Lamé parameter λ ([`SoftBodyMesh::lambda`]), in pascals.
    pub lambda: f32,
    /// Unused; brings the struct to 80 bytes.
    pub pad3: f32,
}

/// The pass's uniform block, the `Parameters` struct of `soft_body.wgsl`: 16 bytes.
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuParameters {
    /// How many elements the pass computes. The element buffer is larger; the invocations past
    /// this return at once.
    pub num_elements: u32,
    /// Unused; a uniform block is a multiple of 16 bytes.
    pub pad: [u32; 3],
}
