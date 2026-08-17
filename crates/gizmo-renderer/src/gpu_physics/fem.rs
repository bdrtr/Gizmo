use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
/// One node of a tetrahedral soft body, as the FEM compute shader reads it.
pub struct GpuSoftBodyNode {
    /// `xyz` = world position, `w` = mass.
    pub position_mass: [f32; 4],
    /// `xyz` = velocity, `w` != 0 pins the node in place (an anchor).
    pub velocity_fixed: [f32; 4],
    /// The accumulated force, as **fixed-point integers**.
    ///
    /// Integers because every tetrahedron touching a node adds to it concurrently, and only the
    /// integer atomics are available in WGSL — a float accumulation would need a lock the GPU does
    /// not have. The shader scales by a fixed factor on the way in and out.
    pub forces: [i32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
/// One tetrahedral element, with its rest shape precomputed.
///
/// The inverse rest matrix is stored rather than the rest positions because that is what the
/// deformation gradient needs — `F = D_current · D_rest⁻¹` — and inverting a 3×3 per element per
/// step, for every element, is exactly the work this precomputation removes.
pub struct GpuTetrahedron {
    /// The four node indices.
    pub indices: [u32; 4],
    /// Column 0 of the inverse rest-shape matrix.
    pub inv_rest_col0: [f32; 4],
    /// Column 1.
    pub inv_rest_col1: [f32; 4],
    /// Column 2.
    pub inv_rest_col2: [f32; 4],
    /// `x` = the element's rest volume, the rest padding.
    pub rest_volume_pad: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
/// An obstacle the soft body is kept out of.
pub struct GpuFemCollider {
    /// Which shape: 0 = sphere, 1 = plane.
    pub shape_type: u32,
    /// The sphere's radius. Read only when [`Self::shape_type`] is 0.
    pub radius: f32,
    /// Padding.
    pub _pad0: u32,
    /// Padding, completing the 16-byte slot before the vectors.
    pub _pad1: u32,
    /// The sphere's centre, or a point on the plane.
    pub position: [f32; 4],
    /// The plane's normal. Read only when [`Self::shape_type`] is 1.
    pub normal: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
/// The FEM solver's per-step parameters.
pub struct GpuFemParams {
    /// `[dt, μ, λ, damping]` — the timestep, the two Lamé parameters that define the material's
    /// stiffness, and velocity damping.
    pub properties: [f32; 4],
    /// `xyz` = gravity in m/s², `w` unused.
    pub gravity: [f32; 4],
    /// `[nodes, elements, colliders, _]` — how much of each buffer is live.
    pub counts: [u32; 4],
}

/// A GPU FEM soft body: a tetrahedral mesh whose deformation is solved on the GPU.
///
/// Three dispatches per step — clear the force accumulator, compute each element's stress, then
/// integrate — for the same reason the rigid solver is staged: each reads what the previous wrote
/// across the whole buffer.
pub struct GpuFemSystem {
    /// The nodes.
    pub nodes_buffer: wgpu::Buffer,
    /// The tetrahedra.
    pub elements_buffer: wgpu::Buffer,
    /// The per-step parameters.
    pub params_buffer: wgpu::Buffer,
    /// The obstacles.
    pub colliders_buffer: wgpu::Buffer,

    /// All four buffers, as the compute passes read them.
    pub compute_bind_group: wgpu::BindGroup,
    /// Stage 1: zero the fixed-point force accumulators.
    pub pipeline_clear: wgpu::ComputePipeline,
    /// Stage 2: per element, compute the deformation gradient and scatter its forces to the four
    /// nodes.
    pub pipeline_stress: wgpu::ComputePipeline,
    /// Stage 3: per node, integrate those forces and resolve collisions.
    pub pipeline_integrate: wgpu::ComputePipeline,

    /// How many nodes are live.
    pub num_nodes: u32,
    /// How many tetrahedra are live.
    pub num_elements: u32,
}

impl GpuFemSystem {
    /// Uploads a tetrahedral mesh, precomputing each element's inverse rest matrix, and builds the
    /// three pipelines.
    pub fn new(
        device: &wgpu::Device,
        nodes: &[GpuSoftBodyNode],
        elements: &[GpuTetrahedron],
        colliders: &[GpuFemCollider],
        params: &GpuFemParams,
    ) -> Self {
        let nodes_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("FEM Nodes Buffer"),
            contents: bytemuck::cast_slice(nodes),
            usage: wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::VERTEX
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
        });

        let elements_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("FEM Elements Buffer"),
            contents: bytemuck::cast_slice(elements),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        let params_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("FEM Params Buffer"),
            contents: bytemuck::cast_slice(&[*params]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Ensure we always have at least a dummy collider to satisfy binding rules
        let dummy_collider = [GpuFemCollider {
            shape_type: 0,
            radius: 0.0,
            _pad0: 0,
            _pad1: 0,
            position: [0.0; 4],
            normal: [0.0, 1.0, 0.0, 0.0],
        }];
        let colliders_data = if colliders.is_empty() {
            &dummy_collider[..]
        } else {
            colliders
        };
        let colliders_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("FEM Colliders Buffer"),
            contents: bytemuck::cast_slice(colliders_data),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });

        let compute_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: false },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::COMPUTE,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
                label: Some("fem_compute_layout"),
            });

        let compute_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &compute_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: params_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: nodes_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: elements_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: colliders_buffer.as_entire_binding(),
                },
            ],
            label: Some("fem_compute_bind_group"),
        });

        let compute_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("FEM Compute Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/fem_compute.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("FEM Compute Pipeline Layout"),
            bind_group_layouts: &[Some(&compute_bind_group_layout)],
            immediate_size: 0,
        });

        let pipeline_clear = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("FEM Clear Forces"),
            layout: Some(&pipeline_layout),
            module: &compute_shader,
            entry_point: Some("clear_forces"),
            compilation_options: Default::default(),
            cache: None,
        });

        let pipeline_stress = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("FEM Compute Stress"),
            layout: Some(&pipeline_layout),
            module: &compute_shader,
            entry_point: Some("compute_stress"),
            compilation_options: Default::default(),
            cache: None,
        });

        let pipeline_integrate = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("FEM Integrate"),
            layout: Some(&pipeline_layout),
            module: &compute_shader,
            entry_point: Some("integrate"),
            compilation_options: Default::default(),
            cache: None,
        });

        Self {
            nodes_buffer,
            elements_buffer,
            params_buffer,
            colliders_buffer,
            compute_bind_group,
            pipeline_clear,
            pipeline_stress,
            pipeline_integrate,
            num_nodes: nodes.len() as u32,
            num_elements: elements.len() as u32,
        }
    }

    /// Records one whole FEM step: clear, stress, integrate.
    pub fn compute_pass(&self, encoder: &mut wgpu::CommandEncoder) {
        let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
            label: Some("FEM Compute Pass"),
            timestamp_writes: None,
        });

        cpass.set_bind_group(0, &self.compute_bind_group, &[]);

        cpass.set_pipeline(&self.pipeline_clear);
        cpass.dispatch_workgroups(self.num_nodes.div_ceil(256), 1, 1);

        cpass.set_pipeline(&self.pipeline_stress);
        cpass.dispatch_workgroups(self.num_elements.div_ceil(256), 1, 1);

        cpass.set_pipeline(&self.pipeline_integrate);
        cpass.dispatch_workgroups(self.num_nodes.div_ceil(256), 1, 1);
    }
}
