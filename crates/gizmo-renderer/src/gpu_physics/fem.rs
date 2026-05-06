use wgpu::util::DeviceExt;

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuSoftBodyNode {
    pub position_mass: [f32; 4],
    pub velocity_fixed: [f32; 4],
    pub forces: [i32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuTetrahedron {
    pub indices: [u32; 4],
    pub inv_rest_col0: [f32; 4],
    pub inv_rest_col1: [f32; 4],
    pub inv_rest_col2: [f32; 4],
    pub rest_volume_pad: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuFemCollider {
    pub shape_type: u32,
    pub radius: f32,
    pub _pad0: u32,
    pub _pad1: u32,
    pub position: [f32; 4],
    pub normal: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuFemParams {
    pub properties: [f32; 4], // dt, mu, lambda, damping
    pub gravity: [f32; 4],    // gx, gy, gz, _pad
    pub counts: [u32; 4],     // num_nodes, num_elements, num_colliders, _pad
}

pub struct GpuFemSystem {
    pub nodes_buffer: wgpu::Buffer,
    pub elements_buffer: wgpu::Buffer,
    pub params_buffer: wgpu::Buffer,
    pub colliders_buffer: wgpu::Buffer,

    pub compute_bind_group: wgpu::BindGroup,
    pub pipeline_clear: wgpu::ComputePipeline,
    pub pipeline_stress: wgpu::ComputePipeline,
    pub pipeline_integrate: wgpu::ComputePipeline,

    pub num_nodes: u32,
    pub num_elements: u32,
}

impl GpuFemSystem {
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
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC,
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
        let dummy_collider = [GpuFemCollider { shape_type: 0, radius: 0.0, _pad0: 0, _pad1: 0, position: [0.0; 4], normal: [0.0, 1.0, 0.0, 0.0] }];
        let colliders_data = if colliders.is_empty() { &dummy_collider[..] } else { colliders };
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
            bind_group_layouts: &[&compute_bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline_clear = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("FEM Clear Forces"),
            layout: Some(&pipeline_layout),
            module: &compute_shader,
            entry_point: "clear_forces",
            compilation_options: Default::default(),
        });

        let pipeline_stress = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("FEM Compute Stress"),
            layout: Some(&pipeline_layout),
            module: &compute_shader,
            entry_point: "compute_stress",
            compilation_options: Default::default(),
        });

        let pipeline_integrate = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("FEM Integrate"),
            layout: Some(&pipeline_layout),
            module: &compute_shader,
            entry_point: "integrate",
            compilation_options: Default::default(),
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
