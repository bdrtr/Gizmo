use gizmo_math::Vec3;
#[derive(Clone, Default)]
/// The frame's debug lines, accumulated immediate-mode.
///
/// Draw into it from anywhere during the frame, then hand it to
/// [`GizmoRendererSystem::update`]; [`clear`](Self::clear) starts the next frame. Nothing here
/// touches the GPU, so a system can draw gizmos without holding a device.
///
/// **Two lifetimes, because a game draws from two schedules.** A line drawn in the per-frame
/// update lives one rendered frame. A line drawn inside a fixed physics step lives until the
/// *next* step: the frame loop calls [`end_fixed_step`](Self::end_fixed_step) after each one,
/// which moves what it drew into [`fixed_lines`](Self::fixed_lines). Until 2026-11-04 there was
/// one buffer and the renderer cleared it only on frames that had stepped physics — so at 144 Hz
/// over a 60 Hz step, a line drawn every frame was drawn twice on most frames (a moving one
/// left a ghost a frame behind), and a line drawn every step was drawn twice on a frame with two
/// steps.
pub struct Gizmos {
    /// The line vertices, in pairs — each two consecutive entries are one segment. Everything
    /// drawn since the last [`clear`](Self::clear) or [`end_fixed_step`](Self::end_fixed_step).
    pub lines: Vec<GizmoVertex>,
    /// What the last fixed step drew, kept and drawn on every frame until the next step replaces
    /// it — a frame that steps no physics still shows the physics' lines, once.
    pub fixed_lines: Vec<GizmoVertex>,
    /// Whether the lines are occluded by the scene. False draws them over everything, which is
    /// what makes a gizmo inside a mesh still selectable.
    pub depth_test: bool,
}

impl Gizmos {
    /// Ends a rendered frame: drops the lines drawn this frame. The last fixed step's
    /// [`fixed_lines`](Self::fixed_lines) stay — they belong to the simulation's clock, not the
    /// frame's.
    pub fn clear(&mut self) {
        self.lines.clear();
    }

    /// Ends a rendered frame and hands over what it draws: this frame's lines, moved out (so this
    /// buffer starts the next frame empty, as [`clear`](Self::clear) leaves it), and a copy of the
    /// last fixed step's, which stay here until the next step replaces them.
    ///
    /// The engine's frame preparation calls this once a frame and keeps the result as the
    /// [`GizmoFrame`] resource; the draw reads that and never writes the game's `Gizmos`.
    #[must_use = "the frame's lines are moved out: dropping them draws nothing"]
    pub fn take_frame(&mut self) -> GizmoFrame {
        GizmoFrame(Gizmos {
            lines: std::mem::take(&mut self.lines),
            fixed_lines: self.fixed_lines.clone(),
            depth_test: self.depth_test,
        })
    }

    /// Ends a fixed step: what was drawn since the last frame or step becomes the step's lines,
    /// replacing the previous step's. Called by the frame loop after each step.
    pub fn end_fixed_step(&mut self) {
        std::mem::swap(&mut self.lines, &mut self.fixed_lines);
        self.lines.clear();
    }

    /// Every line to draw this frame: the last fixed step's, then this frame's.
    pub fn vertices(&self) -> impl Iterator<Item = &GizmoVertex> {
        self.fixed_lines.iter().chain(&self.lines)
    }

    /// How many vertices [`vertices`](Self::vertices) yields.
    pub fn vertex_count(&self) -> usize {
        self.fixed_lines.len() + self.lines.len()
    }

    /// One world-space segment.
    pub fn draw_line(&mut self, start: Vec3, end: Vec3, color: [f32; 4]) {
        // Prevent GPU driver crashes from NaN/Infinity vertices
        if !start.x.is_finite()
            || !start.y.is_finite()
            || !start.z.is_finite()
            || !end.x.is_finite()
            || !end.y.is_finite()
            || !end.z.is_finite()
        {
            return;
        }

        // Prevent zero-length lines which crash some Vulkan/Mesa drivers
        if start.distance_squared(end) < 1e-8 {
            return;
        }

        self.lines.push(GizmoVertex {
            position: start.to_array(),
            color,
        });
        self.lines.push(GizmoVertex {
            position: end.to_array(),
            color,
        });
    }

    /// The twelve edges of an axis-aligned box.
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
        self.draw_line(p0, p1, color);
        self.draw_line(p1, p2, color);
        self.draw_line(p2, p3, color);
        self.draw_line(p3, p0, color);
        // Top
        self.draw_line(p4, p5, color);
        self.draw_line(p5, p6, color);
        self.draw_line(p6, p7, color);
        self.draw_line(p7, p4, color);
        // Pillers
        self.draw_line(p0, p4, color);
        self.draw_line(p1, p5, color);
        self.draw_line(p2, p6, color);
        self.draw_line(p3, p7, color);
    }

    /// [`draw_box`](Self::draw_box) over an [`Aabb`](gizmo_math::Aabb).
    pub fn draw_aabb(&mut self, aabb: gizmo_math::Aabb, color: [f32; 4]) {
        self.draw_box(aabb.min.into(), aabb.max.into(), color);
    }

    /// The wireframe of a camera's view volume, by unprojecting the eight NDC corners through the
    /// inverse of `view_proj`. Useful for seeing what a shadow cascade or a culling camera actually
    /// covers — which is otherwise invisible by construction.
    pub fn draw_frustum(&mut self, view_proj: gizmo_math::Mat4, color: [f32; 4]) {
        let inv = view_proj.inverse();
        
        let corners_ndc = [
            Vec3::new(-1.0, -1.0, 1.0), // Near bottom left
            Vec3::new(1.0, -1.0, 1.0),  // Near bottom right
            Vec3::new(1.0, 1.0, 1.0),   // Near top right
            Vec3::new(-1.0, 1.0, 1.0),  // Near top left
            Vec3::new(-1.0, -1.0, 0.0), // Far bottom left
            Vec3::new(1.0, -1.0, 0.0),  // Far bottom right
            Vec3::new(1.0, 1.0, 0.0),   // Far top right
            Vec3::new(-1.0, 1.0, 0.0),  // Far top left
        ];

        let mut corners_world = [Vec3::ZERO; 8];
        for i in 0..8 {
            let p = inv.project_point3(corners_ndc[i]);
            corners_world[i] = p;
        }

        // Near plane
        self.draw_line(corners_world[0], corners_world[1], color);
        self.draw_line(corners_world[1], corners_world[2], color);
        self.draw_line(corners_world[2], corners_world[3], color);
        self.draw_line(corners_world[3], corners_world[0], color);

        // Far plane
        self.draw_line(corners_world[4], corners_world[5], color);
        self.draw_line(corners_world[5], corners_world[6], color);
        self.draw_line(corners_world[6], corners_world[7], color);
        self.draw_line(corners_world[7], corners_world[4], color);

        // Connecting lines
        self.draw_line(corners_world[0], corners_world[4], color);
        self.draw_line(corners_world[1], corners_world[5], color);
        self.draw_line(corners_world[2], corners_world[6], color);
        self.draw_line(corners_world[3], corners_world[7], color);
    }
}

/// The debug lines one rendered frame draws, taken from the game's [`Gizmos`] by
/// [`Gizmos::take_frame`] before the frame is drawn.
///
/// A resource of its own so the draw only reads: it used to clear the game's `Gizmos` after
/// drawing them, which is a write into the world the game owns — and a draw that writes there
/// cannot draw from a copy of that world. Replaced whole every frame, so nothing has to clear it.
#[derive(Clone, Default)]
pub struct GizmoFrame(Gizmos);

impl GizmoFrame {
    /// The lines to draw: the last fixed step's and this frame's, with the game's depth-test
    /// setting.
    #[must_use]
    pub fn gizmos(&self) -> &Gizmos {
        &self.0
    }
}

#[derive(Clone, Copy, Debug)]
/// One debug-line vertex, CPU-side.
pub struct GizmoVertex {
    /// World-space position.
    pub position: [f32; 3],
    /// Its colour, RGBA.
    pub color: [f32; 4],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, bytemuck::Pod, bytemuck::Zeroable)]
/// The same vertex in the layout the GPU reads — `Pod`, and therefore uploadable, which
/// [`GizmoVertex`] is not.
pub struct GpuGizmoVertex {
    /// World-space position.
    pub position: [f32; 3],
    /// Its colour, RGBA.
    pub color: [f32; 4],
}

impl GpuGizmoVertex {
    /// This vertex as wgpu describes it: position at location 0, colour at location 1.
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

/// The GPU side of the debug lines: two pipelines and one growable vertex buffer.
pub struct GizmoRendererSystem {
    /// The depth-tested pipeline — a line behind geometry is hidden by it.
    ///
    /// It genuinely is one since 2026-08-24; before that it was built with
    /// `CompareFunction::Always`, identical to [`pipeline_no_depth`](Self::pipeline_no_depth).
    pub pipeline: wgpu::RenderPipeline,
    /// The same pipeline without the depth test, for lines that must stay visible through
    /// geometry.
    pub pipeline_no_depth: wgpu::RenderPipeline,
    /// The vertex buffer the frame's lines are uploaded into.
    pub vertex_buffer: wgpu::Buffer,
    /// How many vertices that buffer holds now. It grows, by powers of two, to what a frame asks
    /// for, up to [`MAX_GIZMO_VERTICES`]; lines past that are dropped. It held 200 000 from the
    /// start until 2026-11-12 — 5.3 MiB that a game drawing no debug line, every phone game, paid for.
    pub max_vertices: u32,
    /// How many vertices the last [`update`](Self::update) wrote — the draw count.
    pub index_count: u32,
}

/// The most debug-line vertices a frame draws; the rest are dropped, with a warning.
pub const MAX_GIZMO_VERTICES: u32 = 200_000;
/// What the vertex buffer holds before a frame asks for more: 1 024 vertices, 28 KiB.
const FIRST_GIZMO_VERTICES: u32 = 1024;

fn gizmo_vertex_buffer(device: &wgpu::Device, vertices: u32) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Gizmo Vertex Buffer"),
        size: (vertices as usize * std::mem::size_of::<GpuGizmoVertex>()) as wgpu::BufferAddress,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

impl GizmoRendererSystem {
    /// Builds both pipelines and a small vertex buffer, which grows with the lines.
    pub fn new(
        device: &wgpu::Device,
        global_bind_group_layout: &wgpu::BindGroupLayout,
        output_format: wgpu::TextureFormat,
        depth_format: wgpu::TextureFormat,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Debug Lines Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/debug_lines.wgsl").into()),
        });

        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Debug Lines Pipeline Layout"),
                bind_group_layouts: &[Some(global_bind_group_layout)],
                immediate_size: 0,
            });

        let mut desc = wgpu::RenderPipelineDescriptor {
            label: Some("Debug Lines Pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(GpuGizmoVertex::desc())],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
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
                // Never written: a debug line is an overlay, and one that wrote depth would
                // occlude the next one drawn behind it.
                depth_write_enabled: Some(false),
                depth_compare: Some(wgpu::CompareFunction::LessEqual),
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        };

        let pipeline = device.create_render_pipeline(&desc);

        // Variant without depth testing (to overlay unconditionally).
        //
        // This arm said `CompareFunction::Always` and so did the one above it — the two pipelines
        // were built from identical state, so `Gizmos::depth_test` selected between two copies of
        // the same thing and the field's own doc ("whether the lines are occluded by the scene")
        // was false. Nothing caught it because nothing in the workspace sets the flag to `true`:
        // `Gizmos` derives `Default` (false) and `gizmo-studio` writes `false` explicitly, so
        // fixing it changes no picture the engine draws today and makes the flag mean what it says.
        desc.depth_stencil = Some(wgpu::DepthStencilState {
            format: depth_format,
            depth_write_enabled: Some(false),
            depth_compare: Some(wgpu::CompareFunction::Always),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        });
        desc.label = Some("Debug Lines No-Depth Pipeline");
        let pipeline_no_depth = device.create_render_pipeline(&desc);

        let max_vertices = FIRST_GIZMO_VERTICES;
        let vertex_buffer = gizmo_vertex_buffer(device, max_vertices);

        Self {
            pipeline,
            pipeline_no_depth,
            vertex_buffer,
            max_vertices,
            index_count: 0,
        }
    }

    /// Uploads this frame's lines, growing the buffer to hold them up to [`MAX_GIZMO_VERTICES`] and
    /// truncating past that.
    pub fn update(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, gizmos: &Gizmos) {
        self.index_count = gizmos.vertex_count() as u32;
        let wanted = self.index_count.min(MAX_GIZMO_VERTICES);
        if wanted > self.max_vertices {
            self.max_vertices = wanted.next_power_of_two().min(MAX_GIZMO_VERTICES);
            self.vertex_buffer = gizmo_vertex_buffer(device, self.max_vertices);
        }
        if self.index_count > 0 {
            let to_write = self.index_count.min(self.max_vertices) as usize;

            // Map gizmo_core::GizmoVertex to GpuGizmoVertex
            let mut gpu_data = Vec::with_capacity(to_write);
            for v in gizmos.vertices().take(to_write) {
                gpu_data.push(GpuGizmoVertex {
                    position: v.position,
                    color: v.color,
                });
            }

            queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&gpu_data));
        }
    }

    #[tracing::instrument(skip_all, level = "trace")]
    /// Draws the uploaded lines, choosing the depth-tested or depth-ignoring pipeline.
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
        if self.index_count > self.max_vertices {
            // The gizmo vertex buffer is fixed-capacity; excess debug lines are
            // dropped this frame. Flag it so "missing" debug geometry is explained.
            tracing::warn!(
                requested = self.index_count,
                capacity = self.max_vertices,
                "[GizmoDebug] debug line vertices exceed buffer capacity; tail dropped"
            );
        }
        tracing::trace!(vertices = draw_count, depth_test, "[GizmoDebug] drawing lines");
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

#[cfg(test)]
mod lifetime_tests {
    use super::*;

    fn line(g: &mut Gizmos, x: f32) {
        g.draw_line(Vec3::new(x, 0.0, 0.0), Vec3::new(x, 1.0, 0.0), [1.0; 4]);
    }

    /// A step's lines outlive the frame that drew them and die at the next step; a frame's lines
    /// die at the frame's end.
    #[test]
    fn step_lines_live_until_the_next_step_and_frame_lines_one_frame() {
        let mut g = Gizmos::default();
        line(&mut g, 1.0); // drawn in a fixed step
        g.end_fixed_step();
        line(&mut g, 2.0); // drawn in update
        assert_eq!(g.vertex_count(), 4);
        g.clear(); // frame rendered

        // A frame that steps nothing still shows the step's line — once, not the frame's twice.
        line(&mut g, 2.0);
        assert_eq!(g.vertex_count(), 4, "a frame with no step lost or doubled lines");
        g.clear();

        // Two steps in one frame: only the second one's line is shown.
        line(&mut g, 3.0);
        g.end_fixed_step();
        line(&mut g, 4.0);
        g.end_fixed_step();
        assert_eq!(g.vertex_count(), 2, "both steps' lines drawn");
        assert_eq!(g.vertices().next().unwrap().position[0], 4.0);
    }
}
