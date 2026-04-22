use gizmo_math::Vec3;
#[derive(Clone, Default)]
pub struct Gizmos {
    pub lines: Vec<GizmoVertex>,
    pub depth_test: bool,
}

impl Gizmos {
    pub fn clear(&mut self) {
        self.lines.clear();
    }

    pub fn draw_line(&mut self, start: Vec3, end: Vec3, color: [f32; 4]) {
        self.lines.push(GizmoVertex {
            position: start.to_array(),
            color,
        });
        self.lines.push(GizmoVertex {
            position: end.to_array(),
            color,
        });
    }

    pub fn draw_box(&mut self, min: Vec3, max: Vec3, color: [f32; 4]) {
        let p0 = Vec3::new(min.x, min.y, min.z);
        let p1 = Vec3::new(max.x, min.y, min.z);
        let p2 = Vec3::new(max.x, max.y, min.z);
        let p3 = Vec3::new(min.x, max.y, min.z);
        let p4 = Vec3::new(min.x, min.y, max.z);
        let p5 = Vec3::new(max.x, min.y, max.z);
        let p6 = Vec3::new(max.x, max.y, max.z);
        let p7 = Vec3::new(min.x, max.y, max.z);
        // Bottom
        self.draw_line(p0, p1, color); self.draw_line(p1, p2, color);
        self.draw_line(p2, p3, color); self.draw_line(p3, p0, color);
        // Top
        self.draw_line(p4, p5, color); self.draw_line(p5, p6, color);
        self.draw_line(p6, p7, color); self.draw_line(p7, p4, color);
        // Pillers
        self.draw_line(p0, p4, color); self.draw_line(p1, p5, color);
        self.draw_line(p2, p6, color); self.draw_line(p3, p7, color);
    }
}

#[derive(Clone, Copy, Debug)]
pub struct GizmoVertex {
    pub position: [f32; 3],
    pub color: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct GpuGizmoVertex {
    pub position: [f32; 3],
    pub color: [f32; 4],
}

impl GpuGizmoVertex {
    pub fn desc<'a>() -> wgpu::VertexBufferLayout<'a> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<GpuGizmoVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: 12,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x4,
                },
            ],
        }
    }
}

pub struct GizmoRendererSystem {
    pub pipeline: wgpu::RenderPipeline,
    pub pipeline_no_depth: wgpu::RenderPipeline,
    pub vertex_buffer: wgpu::Buffer,
    pub max_vertices: u32,
    pub index_count: u32,
}

impl GizmoRendererSystem {
    pub fn new(
        device: &wgpu::Device,
        global_bind_group_layout: &wgpu::BindGroupLayout,
        output_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Debug Lines Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("debug_lines.wgsl").into()),
        });

        let render_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Debug Lines Pipeline Layout"),
            bind_group_layouts: &[global_bind_group_layout],
            push_constant_ranges: &[],
        });

        let mut desc = wgpu::RenderPipelineDescriptor {
            label: Some("Debug Lines Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[GpuGizmoVertex::desc()],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: output_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: depth_format,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        };

        let pipeline = device.create_render_pipeline(&desc);

        // Variant without depth testing (to overlay unconditionally)
        desc.depth_stencil = Some(wgpu::DepthStencilState {
            format: depth_format,
            depth_write_enabled: false,
            depth_compare: wgpu::CompareFunction::Always,
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        });
        desc.label = Some("Debug Lines No-Depth Pipeline");
        let pipeline_no_depth = device.create_render_pipeline(&desc);

        let max_vertices = 200_000;
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Gizmo Vertex Buffer"),
            size: (max_vertices as usize * std::mem::size_of::<GpuGizmoVertex>()) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        Self {
            pipeline,
            pipeline_no_depth,
            vertex_buffer,
            max_vertices,
            index_count: 0,
        }
    }

    pub fn update(&mut self, queue: &wgpu::Queue, gizmos: &Gizmos) {
        self.index_count = gizmos.lines.len() as u32;
        if self.index_count > 0 {
            let to_write = self.index_count.min(self.max_vertices) as usize;
            
            // Map gizmo_core::GizmoVertex to GpuGizmoVertex
            let mut gpu_data = Vec::with_capacity(to_write);
            for v in &gizmos.lines[0..to_write] {
                gpu_data.push(GpuGizmoVertex {
                    position: v.position,
                    color: v.color,
                });
            }

            queue.write_buffer(
                &self.vertex_buffer,
                0,
                bytemuck::cast_slice(&gpu_data),
            );
        }
    }

    pub fn render<'a>(
        &'a self,
        rpass: &mut wgpu::RenderPass<'a>,
        global_bind_group: &'a wgpu::BindGroup,
        depth_test: bool,
    ) {
        if self.index_count == 0 {
            return;
        }
        let draw_count = self.index_count.min(self.max_vertices);
        if depth_test {
            rpass.set_pipeline(&self.pipeline);
        } else {
            rpass.set_pipeline(&self.pipeline_no_depth);
        }
        rpass.set_bind_group(0, global_bind_group, &[]);
        rpass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        rpass.draw(0..draw_count, 0..1);
    }
}
