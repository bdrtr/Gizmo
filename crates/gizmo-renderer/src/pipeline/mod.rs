//! Setting up the scene render pipelines: layouts, shadow resources, the core and shadow
//! pipelines, and the global bind groups. Split into submodules; the public API (SceneState,
//! build_scene_pipelines, rebuild_pipelines, load_shader) is re-exported unchanged from this
//! module.

mod layouts;
mod pipelines;
// `pub(crate)` for `compose_module`: the layout contract tests parse what the pipeline composes,
// which is the point of them — see `crate::shader_contract`.
pub(crate) mod shaders;
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
    /// The main opaque PBR pipeline, back faces culled.
    pub render_pipeline: wgpu::RenderPipeline,
    /// The same, with culling off, for materials marked double-sided.
    pub render_double_sided_pipeline: wgpu::RenderPipeline,
    /// Albedo straight through, no lighting — see [`MaterialType::Unlit`](crate::components::MaterialType::Unlit).
    pub unlit_pipeline: wgpu::RenderPipeline,
    /// Baked lighting × texture, with the sun's cascade term. See `shaders/baked_lit.wgsl`.
    pub baked_lit_pipeline: wgpu::RenderPipeline,
    /// The same shader with alpha blending and no depth write, for a `BakedLit` material
    /// marked transparent — a decal or skid-mark layer, which the opaque variant would paint
    /// over the road as solid geometry no matter what alpha the shader produced.
    pub baked_lit_transparent_pipeline: wgpu::RenderPipeline,
    /// The generated atmospheric sky — a gradient from the sun's direction and colour, ignoring
    /// the mesh. Contrast [`backdrop_pipeline`](Self::backdrop_pipeline).
    pub sky_pipeline: wgpu::RenderPipeline,
    /// The scene's OWN painted sky/panorama geometry: `shaders/backdrop.wgsl`, no depth write,
    /// camera-locked. Unlike `sky_pipeline` it draws the mesh's texture and vertex colour
    /// instead of an invented gradient. See [`crate::backdrop`].
    pub backdrop_pipeline: wgpu::RenderPipeline,
    /// The animated water surface.
    pub water_pipeline: wgpu::RenderPipeline,
    /// Depth-only, for rendering into the shadow cascades.
    pub shadow_pipeline: wgpu::RenderPipeline,
    /// Line-topology, for wireframe views.
    pub wireframe_pipeline: wgpu::RenderPipeline,
    /// Alpha-blended, no depth write, for the transparent bucket.
    pub transparent_pipeline: wgpu::RenderPipeline,
    /// The editor's ground grid.
    pub grid_pipeline: wgpu::RenderPipeline,
    /// The [`SceneUniforms`](crate::gpu_types::SceneUniforms) buffer — group 0, rewritten each
    /// frame.
    pub global_uniform_buffer: wgpu::Buffer,
    /// Its layout. Every pipeline in the engine declares this as group 0.
    pub global_bind_group_layout: wgpu::BindGroupLayout,
    /// Its bind group.
    ///
    /// Read it through [`view_bind_group`](SceneState::view_bind_group) rather than directly,
    /// unless you specifically mean "the frame's own camera regardless of which view is active".
    pub global_bind_group: wgpu::BindGroup,
    /// Extra scene-uniform views, each with its own camera. Empty by default.
    ///
    /// See [`SceneView`] for why a second camera needs a second buffer rather than a second write
    /// to this one.
    pub views: Vec<SceneView>,
    /// Which of [`views`](SceneState::views) the next recorded pass binds, or `None` for the
    /// frame's own camera.
    ///
    /// Set it, record a pass, set it back. Nothing reads it outside pass recording, so it is a
    /// cursor rather than state — see [`view_bind_group`](SceneState::view_bind_group).
    pub active_view: Option<usize>,
    /// Clustered lights: `(offset, count)` per cluster. Allocated for the worst case so the bind
    /// group is built once — see [`crate::clustered::index_bytes`].
    pub cluster_table_buffer: wgpu::Buffer,
    /// Clustered lights: the index list the table points into.
    pub cluster_index_buffer: wgpu::Buffer,
    /// Layout of the group the lit shaders sample the shadow maps through.
    pub shadow_bind_group_layout: wgpu::BindGroupLayout,
    /// That group: the cascade array, the point-light cube, and their comparison samplers.
    pub shadow_bind_group: wgpu::BindGroup,
    /// Depth `texture_2d_array` (all CSM layers) for comparison sampling in lit shaders.
    pub shadow_texture_view: wgpu::TextureView,
    /// One 2D depth view per cascade for shadow map rendering passes.
    pub shadow_cascade_layer_views: [wgpu::TextureView; 4],
    /// The cascade depth texture itself — a depth array, one layer per cascade.
    pub shadow_depth_texture: wgpu::Texture,
    /// The point-light shadow cube's depth texture.
    pub point_shadow_depth_texture: wgpu::Texture,
    /// It as a cube view, for sampling.
    pub point_shadow_cube_view: wgpu::TextureView,
    /// One 2-D view per cube face, for rendering into.
    pub point_shadow_face_views: [wgpu::TextureView; 6],
    /// Layout of the shadow pass's own group, holding just its light-view-projection.
    pub shadow_pass_bind_group_layout: wgpu::BindGroupLayout,
    /// One uniform buffer + bind group per CSM cascade (avoids per-pass overwrite races on the queue).
    pub shadow_cascade_uniform_buffers: [wgpu::Buffer; 4],
    /// The matching bind groups, one per cascade.
    pub shadow_pass_bind_groups: [wgpu::BindGroup; 4],
    /// One uniform buffer per cube face, for the same reason as the cascades': six passes writing
    /// one buffer would race on the queue.
    pub point_shadow_uniform_buffers: [wgpu::Buffer; 6],
    /// The matching bind groups, one per face.
    pub point_shadow_pass_bind_groups: [wgpu::BindGroup; 6],
    /// Layout of a material's texture group — every [`Material`](crate::components::Material)
    /// builds its bind group against this.
    pub texture_bind_group_layout: wgpu::BindGroupLayout,
    /// Layout of a skinned mesh's joint-matrix group.
    pub skeleton_bind_group_layout: wgpu::BindGroupLayout,
    /// A one-joint identity skeleton, bound for unskinned meshes: a pipeline's groups must all be
    /// bound, so an unskinned draw still needs *something* here.
    pub dummy_skeleton_bind_group: Arc<wgpu::BindGroup>,
    /// Layout of the instance storage buffer's group.
    pub instance_bind_group_layout: wgpu::BindGroupLayout,
    /// The per-frame [`InstanceRaw`](crate::gpu_types::InstanceRaw) buffer, grown as needed by
    /// [`ensure_instance_capacity`](Self::ensure_instance_capacity).
    pub instance_buffer: wgpu::Buffer,
    /// Its bind group. Rebuilt whenever the buffer is reallocated.
    pub instance_bind_group: wgpu::BindGroup,
    /// Current capacity (number of InstanceRaw items) of `instance_buffer`.
    pub instance_capacity: usize,
}

/// A second camera for the same frame, with its own scene-uniform buffer.
///
/// # Why a second buffer and not a second write
///
/// The camera matrices live in one [`SceneState::global_uniform_buffer`], written with
/// `queue.write_buffer`. Those writes order against **submission**, not against encoder recording,
/// so two passes recorded into one encoder both read whichever camera was written last — recording
/// order is not write order. Measured in the `render_to_texture` demo: an offscreen pass and a
/// window pass gave byte-identical brightness profiles until the offscreen one was moved into its
/// own encoder and submitted immediately, at which point all three sampled bands changed
/// (54.07/113.55/83.28 → 87.95/95.83/60.11).
///
/// So the workaround was one encoder and one submit *per camera*, which is neither documented nor
/// cheap, and does not scale to the several views a planar mirror or a reflection probe wants.
///
/// A `SceneView` owns its own uniform buffer, so the writes have nowhere to collide: two views can
/// be written, then two passes recorded into one encoder, then one submit.
///
/// # Use
///
/// ```no_run
/// # use gizmo_renderer::pipeline::{SceneState, SceneView};
/// # fn demo(device: &wgpu::Device, queue: &wgpu::Queue, scene: &mut SceneState,
/// #         mirror: &gizmo_renderer::gpu_types::SceneUniforms) {
/// // Once, at setup:
/// scene.views.push(SceneView::new(device, scene, "mirror"));
///
/// // Each frame, before recording the mirror's pass:
/// scene.views[0].write(queue, mirror);
/// scene.active_view = Some(0);
/// // ... record the pass ...
/// scene.active_view = None;
/// # }
/// ```
///
/// The cluster table and light index list are shared with the frame's own view rather than
/// duplicated: they are built from the *scene*, not from the camera, so a second camera does not
/// need a second copy.
pub struct SceneView {
    /// This view's [`SceneUniforms`](crate::gpu_types::SceneUniforms) buffer.
    pub uniform_buffer: wgpu::Buffer,
    /// Its group-0 bind group, layout-identical to [`SceneState::global_bind_group`].
    pub bind_group: wgpu::BindGroup,

    /// This view's clustered-light table and index list.
    ///
    /// Clusters are a **view-space** grid, so two cameras do not agree about them — sharing them
    /// meant the second camera's upload overwrote the first's before either pass ran.
    pub cluster_table_buffer: wgpu::Buffer,
    /// The index list [`cluster_table_buffer`](Self::cluster_table_buffer) points into.
    pub cluster_index_buffer: wgpu::Buffer,

    /// This view's cascade light-view-projections, one per CSM cascade.
    ///
    /// The **shadow maps themselves are shared** and are not duplicated here: render passes execute
    /// in the order they are recorded, so a view's shadow pass rewrites the same texture just
    /// before that view's main pass reads it. What could not be shared is this — a uniform buffer,
    /// written with `queue.write_buffer`, which orders against submission rather than recording.
    ///
    /// That is the same distinction `SceneView` exists for, and it is why a second camera costs
    /// about **460 KB** rather than the 144 MB a second 3072²×4 cascade array would.
    pub shadow_cascade_uniform_buffers: [wgpu::Buffer; 4],
    /// The matching bind groups, one per cascade.
    pub shadow_pass_bind_groups: [wgpu::BindGroup; 4],
    /// This view's point-shadow face matrices, one per cube face.
    pub point_shadow_uniform_buffers: [wgpu::Buffer; 6],
    /// The matching bind groups, one per face.
    pub point_shadow_pass_bind_groups: [wgpu::BindGroup; 6],
}

impl SceneView {
    /// Builds a view with its own camera uniform, cluster buffers and shadow-pass uniforms.
    ///
    /// Everything here is per-camera *derived* state written with `queue.write_buffer`. Textures
    /// are not: the cascade array, the point-shadow cube and the G-buffer are all shared, because
    /// render passes run in recording order and each view's passes rewrite them in turn.
    #[must_use]
    pub fn new(device: &wgpu::Device, scene: &SceneState, label: &str) -> Self {
        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: std::mem::size_of::<crate::gpu_types::SceneUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Sized from the shared ones rather than from the grid, so a view cannot disagree with the
        // scene about how big a cluster table is.
        let cluster_table_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: scene.cluster_table_buffer.size(),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let cluster_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: scene.cluster_index_buffer.size(),
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            layout: &scene.global_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: cluster_table_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: cluster_index_buffer.as_entire_binding(),
                },
            ],
            label: Some(label),
        });

        let mk_shadow_uniform = |i: usize, kind: &str| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(&format!("{label} {kind} {i}")),
                size: std::mem::size_of::<crate::gpu_types::ShadowVsUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let mk_shadow_bg = |buf: &wgpu::Buffer, i: usize, kind: &str| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                layout: &scene.shadow_pass_bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buf.as_entire_binding(),
                }],
                label: Some(&format!("{label} {kind} bg {i}")),
            })
        };

        let shadow_cascade_uniform_buffers: [wgpu::Buffer; 4] =
            std::array::from_fn(|i| mk_shadow_uniform(i, "cascade"));
        let shadow_pass_bind_groups: [wgpu::BindGroup; 4] =
            std::array::from_fn(|i| mk_shadow_bg(&shadow_cascade_uniform_buffers[i], i, "cascade"));
        let point_shadow_uniform_buffers: [wgpu::Buffer; 6] =
            std::array::from_fn(|i| mk_shadow_uniform(i, "point"));
        let point_shadow_pass_bind_groups: [wgpu::BindGroup; 6] =
            std::array::from_fn(|i| mk_shadow_bg(&point_shadow_uniform_buffers[i], i, "point"));

        Self {
            uniform_buffer,
            bind_group,
            cluster_table_buffer,
            cluster_index_buffer,
            shadow_cascade_uniform_buffers,
            shadow_pass_bind_groups,
            point_shadow_uniform_buffers,
            point_shadow_pass_bind_groups,
        }
    }

    /// Writes this view's camera. Independent of every other view's write, which is the point.
    pub fn write(&self, queue: &wgpu::Queue, uniforms: &crate::gpu_types::SceneUniforms) {
        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::bytes_of(uniforms));
    }
}

impl SceneState {
    /// The group-0 bind group the next recorded pass should use.
    ///
    /// [`active_view`](Self::active_view) selects it; `None` means the frame's own camera. Every
    /// pass in the engine binds through here, which is what makes setting `active_view` enough.
    ///
    /// # Panics
    ///
    /// If `active_view` names an index that is not in [`views`](Self::views). That is a caller
    /// bug and silently falling back to the wrong camera would render a plausible wrong frame —
    /// exactly the failure this type exists to end.
    #[must_use]
    pub fn view_bind_group(&self) -> &wgpu::BindGroup {
        match self.active() {
            None => &self.global_bind_group,
            Some(v) => &v.bind_group,
        }
    }

    /// The scene-uniform buffer the next recorded pass will read, selected the same way
    /// [`view_bind_group`](Self::view_bind_group) selects its group.
    ///
    /// A pass that writes the camera must write *here*, not to
    /// [`global_uniform_buffer`](Self::global_uniform_buffer) — otherwise setting `active_view`
    /// binds one view's group and fills another view's buffer, which renders the wrong camera
    /// with no error anywhere.
    ///
    /// # Panics
    ///
    /// Same condition as [`view_bind_group`](Self::view_bind_group): an `active_view` that names
    /// no view.
    #[must_use]
    pub fn view_uniform_buffer(&self) -> &wgpu::Buffer {
        match self.active() {
            None => &self.global_uniform_buffer,
            Some(v) => &v.uniform_buffer,
        }
    }

    /// The active view, or `None` for the frame's own camera. The one place that resolves
    /// [`active_view`](Self::active_view), so every selector below agrees about which view is meant.
    fn active(&self) -> Option<&SceneView> {
        self.active_view.map(|i| {
            self.views.get(i).unwrap_or_else(|| {
                panic!(
                    "active_view = Some({i}) but only {} view(s) exist",
                    self.views.len()
                )
            })
        })
    }

    /// The shadow-pass bind group for cascade `i` in the active view.
    ///
    /// Per-view because the cascade split is derived from the *camera's* frustum: two cameras
    /// disagree about where the splits are, and the uniform carrying that is written with
    /// `queue.write_buffer`, which orders against submission rather than recording. The cascade
    /// **texture** is shared — passes run in recording order, so each view's shadow pass rewrites
    /// it just before that view's main pass reads it.
    ///
    /// # Panics
    ///
    /// If `active_view` names no view, or `i` is not a cascade index.
    #[must_use]
    pub fn view_shadow_pass_bind_group(&self, i: usize) -> &wgpu::BindGroup {
        match self.active() {
            None => &self.shadow_pass_bind_groups[i],
            Some(v) => &v.shadow_pass_bind_groups[i],
        }
    }

    /// The cascade light-view-projection buffer for cascade `i` in the active view.
    ///
    /// Write the matrices here, not to [`shadow_cascade_uniform_buffers`](Self::shadow_cascade_uniform_buffers)
    /// — otherwise a view binds its own group and fills the frame's buffer, and the shadow pass
    /// draws with the wrong camera's splits with nothing reporting it.
    ///
    /// # Panics
    ///
    /// Same condition as [`view_shadow_pass_bind_group`](Self::view_shadow_pass_bind_group).
    #[must_use]
    pub fn view_shadow_cascade_buffer(&self, i: usize) -> &wgpu::Buffer {
        match self.active() {
            None => &self.shadow_cascade_uniform_buffers[i],
            Some(v) => &v.shadow_cascade_uniform_buffers[i],
        }
    }

    /// The point-shadow bind group for cube face `i` in the active view.
    ///
    /// # Panics
    ///
    /// Same condition as [`view_shadow_pass_bind_group`](Self::view_shadow_pass_bind_group).
    #[must_use]
    pub fn view_point_shadow_pass_bind_group(&self, i: usize) -> &wgpu::BindGroup {
        match self.active() {
            None => &self.point_shadow_pass_bind_groups[i],
            Some(v) => &v.point_shadow_pass_bind_groups[i],
        }
    }

    /// The point-shadow face-matrix buffer for cube face `i` in the active view.
    ///
    /// # Panics
    ///
    /// Same condition as [`view_shadow_pass_bind_group`](Self::view_shadow_pass_bind_group).
    #[must_use]
    pub fn view_point_shadow_buffer(&self, i: usize) -> &wgpu::Buffer {
        match self.active() {
            None => &self.point_shadow_uniform_buffers[i],
            Some(v) => &v.point_shadow_uniform_buffers[i],
        }
    }

    /// Grows the instance buffer to hold at least `needed` instances, rebuilding its bind group,
    /// and returns whether it reallocated. A caller holding the old bind group must re-read it
    /// when this returns `true`.
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

impl SceneState {
    /// Upload one frame's clustered-light assignment.
    ///
    /// Both buffers are allocated for the worst case, so this only ever writes a prefix and can
    /// never need to grow — which is what keeps `global_bind_group` valid for the life of the
    /// renderer. A cluster whose count is zero is still written: a stale table from an earlier frame
    /// would light fragments from lights that are no longer there.
    ///
    /// `assignment.dropped` is the caller's to report; this is the transport, not the policy.
    /// Uploads the clustered-light assignment **for the active view**.
    ///
    /// Clusters are a view-space grid, so two cameras genuinely disagree about them; this writes
    /// to whichever view [`active_view`](Self::active_view) names, and the group-0 bind group that
    /// view binds reads those same buffers.
    ///
    /// # Panics
    ///
    /// If `active_view` names no view.
    pub fn upload_clusters(
        &self,
        queue: &wgpu::Queue,
        assignment: &crate::clustered::ClusterAssignment,
    ) {
        let (table_buf, index_buf) = match self.active() {
            None => (&self.cluster_table_buffer, &self.cluster_index_buffer),
            Some(v) => (&v.cluster_table_buffer, &v.cluster_index_buffer),
        };
        let table: Vec<u32> = assignment.table.iter().flat_map(|pair| *pair).collect();
        queue.write_buffer(table_buf, 0, bytemuck::cast_slice(&table));
        if !assignment.indices.is_empty() {
            queue.write_buffer(index_buf, 0, bytemuck::cast_slice(&assignment.indices));
        }
    }
}

#[tracing::instrument(skip_all)]
/// Builds every scene pipeline, bind-group layout, shadow target and shared buffer — the whole of
/// [`SceneState`].
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

    let grid = crate::clustered::ClusterGrid::default();
    let cluster_table_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cluster_table"),
        size: crate::clustered::table_bytes(grid),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let cluster_index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("cluster_light_indices"),
        size: crate::clustered::index_bytes(grid),
        usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });

    let global_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        layout: &layouts.global,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: global_uniform_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: cluster_table_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: cluster_index_buffer.as_entire_binding(),
            },
        ],
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
        baked_lit_pipeline: core_pipelines.baked_lit,
        baked_lit_transparent_pipeline: core_pipelines.baked_lit_transparent,
        sky_pipeline: core_pipelines.sky,
        backdrop_pipeline: core_pipelines.backdrop,
        water_pipeline: core_pipelines.water,
        transparent_pipeline: core_pipelines.transparent,
        grid_pipeline: core_pipelines.grid,
        shadow_pipeline,
        global_uniform_buffer,
        global_bind_group_layout: layouts.global,
        global_bind_group,
        views: Vec::new(),
        active_view: None,
        cluster_table_buffer,
        cluster_index_buffer,
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
/// Recompiles every shader and rebuilds every pipeline in place, keeping the existing buffers,
/// textures and bind groups — what [`Renderer::rebuild_shaders`](crate::Renderer::rebuild_shaders)
/// calls on a hot reload.
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
    renderer.scene.baked_lit_pipeline = core_pipelines.baked_lit;
    renderer.scene.baked_lit_transparent_pipeline = core_pipelines.baked_lit_transparent;
    renderer.scene.sky_pipeline = core_pipelines.sky;
    renderer.scene.backdrop_pipeline = core_pipelines.backdrop;
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

    /// The core WGSL shaders must compile with naga on a headless device.
    /// This checks that edits to shader.wgsl/gbuffer.wgsl (the skinned-normal inverse-transpose
    /// and the like) have not made the WGSL invalid. Skipped gracefully when there is no GPU
    /// adapter.
    ///
    /// **Passing does NOT mean the shader compiles on every backend — and that cost something
    /// once.** What happens here is a `create_shader_module`: naga validates the WGSL. Real
    /// compilation finishes when naga has emitted the target language (HLSL on D3D12) and *that*
    /// language's compiler (FXC) accepts it — and that compiler has constraints this layer knows
    /// nothing about. On 2026-08-14 the shadow PCF loop called `textureSampleCompare` — an
    /// implicit derivative — inside a conditional branch. Flawless by WGSL, but to FXC a
    /// *"gradient instruction used in a loop with varying iteration"*, and **the deferred
    /// lighting pipeline could not be created at all on Windows**: on D3D12 the engine drew
    /// nothing. A defect this test stayed green through for nine months.
    ///
    /// The only thing that catches backend-specific compilation is a test that actually
    /// **creates a pipeline** on that backend: `gizmo`'s `golden_render_tests`, which drive three
    /// different backends across CI's three platforms. The list here is a type check; the list
    /// there is coverage.
    #[test]
    fn core_shaders_compile() {
        // Bu test kendi wgpu cihazını kuruyor. Guard testin TAMAMI boyunca tutulur —
        // yalnız yaratımı serileştirmek ölçüldü ve yetmedi (bkz. `crate::test_gpu`).
        let _gpu = crate::test_gpu::gpu_lock();
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
                ("backdrop.wgsl", include_str!("../shaders/backdrop.wgsl")),
                ("unlit.wgsl", include_str!("../shaders/unlit.wgsl")),
                ("baked_lit.wgsl", include_str!("../shaders/baked_lit.wgsl")),
                ("grid.wgsl", include_str!("../shaders/grid.wgsl")),
                ("water.wgsl", include_str!("../shaders/water.wgsl")),
                ("shadow.wgsl", include_str!("../shaders/shadow.wgsl")),
                ("point_shadow.wgsl", include_str!("../shaders/point_shadow.wgsl")),
                ("decal.wgsl", include_str!("../shaders/decal.wgsl")),
                (
                    "decal_forward.wgsl",
                    include_str!("../shaders/decal_forward.wgsl"),
                ),
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
                // Loaded via create_shader_module in gpu_physics/gpu_particles (not in the
                // golden render test's pipelines), so validate their naga_oil composition
                // here — this test auto-composes any src containing `#import`.
                ("physics_render.wgsl", include_str!("../shaders/physics_render.wgsl")),
                ("particle_render.wgsl", include_str!("../shaders/particle_render.wgsl")),
            ];

            // Shaders that go through the wasm `load_shader_composed_web` path: validate BOTH
            // the native and web shader-def variants here so the web build (no browser in CI)
            // is verified — the web variant strips `#ifdef SHADOWS` and remaps
            // `@group(#{SKELETON_GROUP/INSTANCE_GROUP})`, which is exactly where a bad #ifdef
            // (e.g. a shadow binding used outside the guard) would surface as an undefined id.
            let web_path = [
                "shader.wgsl",
                "unlit.wgsl",
                "baked_lit.wgsl",
                "water.wgsl",
                "sky.wgsl",
                "backdrop.wgsl",
                "grid.wgsl",
            ];

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
        // Bu test kendi wgpu cihazını kuruyor. Guard testin TAMAMI boyunca tutulur —
        // yalnız yaratımı serileştirmek ölçüldü ve yetmedi (bkz. `crate::test_gpu`).
        let _gpu = crate::test_gpu::gpu_lock();
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
