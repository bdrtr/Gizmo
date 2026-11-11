//! Setting up the scene render pipelines: layouts, shadow resources, the core and shadow
//! pipelines, and the global bind groups. Split into submodules; the public API (SceneState,
//! build_scene_pipelines, rebuild_pipelines, load_shader) is re-exported unchanged from this
//! module.

pub(crate) mod layouts;
mod pipelines;
// `pub(crate)` for `compose_module`: the layout contract tests parse what the pipeline composes,
// which is the point of them — see `crate::shader_contract`.
pub(crate) mod shaders;
mod uniforms;

pub use shaders::load_shader;
pub use shaders::load_shader_composed;
#[cfg(gizmo_forward_only)]
pub use shaders::{load_shader_composed_web, load_shader_composed_web_with};

use layouts::{build_layouts, LayoutRefs};
use pipelines::{build_core_pipelines, build_shadow_pipelines};
pub(crate) use pipelines::{forward_pipeline, lit_set, shadow_pipeline};
use uniforms::{build_global_uniforms, build_shadow_resources};

use crate::gpu_types::ShadowVsUniform;
use std::sync::Arc;
use wgpu::util::DeviceExt;

/// Every lit pipeline a draw on one tier can need, from one shader: what
/// [`SceneState::lit_lobes`] holds on the forward-only path (the top tier of
/// [`SceneState::lit_pipeline`]), and what a game's surface is drawn forward with
/// ([`SurfaceMaterial`](crate::custom_material::SurfaceMaterial)).
pub struct LitSet {
    /// As [`SceneState::render_pipeline`].
    pub render: wgpu::RenderPipeline,
    /// As [`SceneState::render_double_sided_pipeline`].
    pub render_double_sided: wgpu::RenderPipeline,
    /// As [`SceneState::transparent_pipeline`].
    pub transparent: wgpu::RenderPipeline,
    /// As [`SceneState::blend_lit_pipelines`].
    pub blend: [wgpu::RenderPipeline; 3],
}

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
    /// The animated water surface: `shaders/water.wgsl`, Gerstner displacement in the vertex
    /// stage and a Fresnel sky term in the fragment stage. Forward-only — the G-buffer is filled
    /// by a shader that does not displace, so a water surface routed through it would be shaded at
    /// the flat plane's depth. Selected by [`Routing::is_water`](crate::routing::Routing::is_water).
    pub water_pipeline: wgpu::RenderPipeline,
    /// The same with culling off, for a water surface marked
    /// [`double_sided`](crate::components::Material::with_double_sided) — looking up at the
    /// underside of the ocean is the ordinary case, not the exotic one.
    pub water_double_sided_pipeline: wgpu::RenderPipeline,
    /// What a [`PlanarReflection`](crate::components::PlanarReflection) draws with once captured.
    pub mirror_pipeline: wgpu::RenderPipeline,
    /// Water with a [`PlanarReflection`](crate::components::PlanarReflection), once captured.
    pub water_mirror_pipeline: wgpu::RenderPipeline,
    /// Depth-only, for rendering into the shadow cascades: a mesh that closes, by its back faces.
    pub shadow_pipeline: wgpu::RenderPipeline,
    /// The same for a mesh that does not close (`Mesh::closed`), by both faces: a plane, a card, a
    /// sheet of cloth has no back to cast from. [`shadow_pipeline_for`](Self::shadow_pipeline_for)
    /// chooses.
    pub shadow_open_pipeline: wgpu::RenderPipeline,
    /// The two above with a fragment stage that cuts where the material does — a cut-out, a LOD
    /// crossfade — closed then open. Every shadow pipeline binds the same groups: the pass's light, the
    /// material, the skeleton and the instances.
    pub shadow_cut_out_pipelines: [wgpu::RenderPipeline; 2],
    /// Line-topology, for wireframe views.
    pub wireframe_pipeline: wgpu::RenderPipeline,
    /// Alpha-blended, no depth write, for the transparent bucket.
    pub transparent_pipeline: wgpu::RenderPipeline,
    /// The editor's ground grid.
    pub grid_pipeline: wgpu::RenderPipeline,
    /// The lit transparent pipeline under each of `BlendMode::VARIANTS` — see
    /// [`blend_variant`](Self::blend_variant).
    pub blend_lit_pipelines: [wgpu::RenderPipeline; 3],
    /// The unlit pipeline under each of them, not writing depth.
    pub blend_unlit_pipelines: [wgpu::RenderPipeline; 3],
    /// The wireframe pipeline's group 5, one bind group per line colour asked for.
    pub wire_color_layout: wgpu::BindGroupLayout,
    /// Those bind groups, by colour. `None` is the one that says "no colour of its own".
    pub wire_colors: std::collections::HashMap<Option<[u8; 3]>, std::sync::Arc<wgpu::BindGroup>>,
    /// The lit pipeline that also reads the frame behind it — see
    /// [`transmissive_pipeline`](Self::transmissive_pipeline).
    pub transmissive: [wgpu::RenderPipeline; 2],
    /// What group 0 offers as the frame behind glass until something draws glass: one black texel.
    pub behind_fallback: wgpu::TextureView,
    /// What group 0 offers now: that, or [`Renderer::behind`](crate::Renderer::behind)'s view.
    pub behind_view: wgpu::TextureView,
    /// How the glass shader reads that frame.
    pub behind_sampler: wgpu::Sampler,
    /// The environment drawn into a cube, which the forward-only path's lit shader reads (`env_cube`).
    pub(crate) env_cube: crate::env_cube::EnvironmentCube,
    /// Reflection probes as the forward-only path's lit shader reads them, on group 0.
    pub(crate) forward_probes: ForwardProbes,
    /// [`unlit_pipeline`](Self::unlit_pipeline) with `KEEPS`, on the forward-only path, whose plain one
    /// cannot discard: a cut-out, a LOD crossfade. `None` on the desktop path, whose unlit pipeline can.
    pub unlit_keeps_pipeline: Option<wgpu::RenderPipeline>,
    /// [`render_pipeline`](Self::render_pipeline) with `DETAILS`, on the forward-only path, whose
    /// plain one leaves them out: a single-sided opaque draw that needs a cut-out, a LOD crossfade or
    /// its own reflectance. `None` on the desktop path, whose lit pipelines always have them.
    pub render_details_pipeline: Option<wgpu::RenderPipeline>,
    /// The lit pipelines with anisotropy, clear coat and subsurface, where the ones above leave them
    /// out. `None` on the desktop path, whose lit pipelines always have them.
    ///
    /// An Adreno 618 pays for those branches whether they are taken or not: 43 fps without them and 20
    /// with them, drawing a cube and a floor. So materials that use none of them get pipelines without
    /// the code, and batching sends the ones that do here. What uses them still costs that much: a clear
    /// coat on a floor filling the screen reads 20 fps.
    pub lit_lobes: Option<LitSet>,
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
    /// The light-cookie array the shadow group carries at bindings 3 and 4. See [`crate::cookies`].
    pub cookies: crate::cookies::CookieTextures,
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

        let bind_group = global_group(
            device,
            &scene.global_bind_group_layout,
            label,
            [&uniform_buffer, &cluster_table_buffer, &cluster_index_buffer],
            &scene.behind_view,
            &scene.behind_sampler,
            scene.env_cube.view(),
            &scene.forward_probes,
        );

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
    /// The pipeline a transparent draw blending with `mode` uses instead of the default one —
    /// `None` for [`BlendMode::Alpha`](crate::components::BlendMode::Alpha), which is the
    /// `transparent` / `unlit` pipeline itself.
    pub fn blend_variant(&self, unlit: bool, mode: crate::components::BlendMode) -> Option<&wgpu::RenderPipeline> {
        let i = crate::components::BlendMode::VARIANTS.iter().position(|m| *m == mode)?;
        Some(if unlit { &self.blend_unlit_pipelines[i] } else { &self.blend_lit_pipelines[i] })
    }

    /// The wireframe pipeline's group 5 for this colour (`None`: the frame's, or the lit colour).
    pub fn wire_color_group(&mut self, device: &wgpu::Device, color: Option<[f32; 3]>) -> std::sync::Arc<wgpu::BindGroup> {
        let key = color.map(|c| c.map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8));
        let layout = &self.wire_color_layout;
        self.wire_colors
            .entry(key)
            .or_insert_with(|| {
                let [r, g, b] = color.unwrap_or([0.0; 3]);
                let alpha = if color.is_some() { 1.0f32 } else { 0.0 };
                let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("wire colour"),
                    contents: bytemuck::cast_slice(&[r, g, b, alpha]),
                    usage: wgpu::BufferUsages::UNIFORM,
                });
                std::sync::Arc::new(device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("wire colour"),
                    layout,
                    entries: &[wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() }],
                }))
            })
            .clone()
    }

    /// The shadow pass's pipeline for a draw of the engine's own material: by its back faces if the
    /// mesh closes and by both if not, and through the cut-out stage if the draw `cuts` — a cut-out or
    /// a LOD crossfade. A game's surface has its own
    /// ([`SurfaceMaterial::shadow_pipeline`](crate::custom_material::SurfaceMaterial::shadow_pipeline)).
    pub fn shadow_pipeline_for(&self, closed: bool, cuts: bool) -> &wgpu::RenderPipeline {
        match (cuts, closed) {
            (true, closed) => &self.shadow_cut_out_pipelines[usize::from(!closed)],
            (false, true) => &self.shadow_pipeline,
            (false, false) => &self.shadow_open_pipeline,
        }
    }

    /// The lit pipeline that also reads the frame behind it, for a draw that
    /// [is glass](crate::components::Material::is_transmissive). Opaque as far as the blend goes,
    /// and it writes depth: what it shows of the world, it read.
    pub fn transmissive_pipeline(&self, double_sided: bool) -> &wgpu::RenderPipeline {
        &self.transmissive[usize::from(double_sided)]
    }

    /// Points every view's group 0 at this frame behind glass, or back at the black texel. Group 0
    /// is the one bind group every pass uses, so this is the only place that has to know.
    pub fn set_behind_view(&mut self, device: &wgpu::Device, behind: Option<&wgpu::TextureView>) {
        self.behind_view = behind.unwrap_or(&self.behind_fallback).clone();
        let view = &self.behind_view;
        let build = |uniform: &wgpu::Buffer, table: &wgpu::Buffer, indices: &wgpu::Buffer| {
            global_group(
                device,
                &self.global_bind_group_layout,
                "global_bind_group",
                [uniform, table, indices],
                view,
                &self.behind_sampler,
                self.env_cube.view(),
                &self.forward_probes,
            )
        };
        let global = build(&self.global_uniform_buffer, &self.cluster_table_buffer, &self.cluster_index_buffer);
        let views: Vec<wgpu::BindGroup> = self
            .views
            .iter()
            .map(|v| build(&v.uniform_buffer, &v.cluster_table_buffer, &v.cluster_index_buffer))
            .collect();
        self.global_bind_group = global;
        for (v, group) in self.views.iter_mut().zip(views) {
            v.bind_group = group;
        }
    }

    /// Points the forward-only path's lit shader at a set of captured reflection probes, or at none. True
    /// when their shape changed — whether there is a background probe, whether there are boxed
    /// ones: the caller builds the pipelines again then ([`rebuild_pipelines`]), so a scene pays only for
    /// the probes it has, and nothing without any. The desktop path reads its probes through the
    /// irradiance group instead, and decides per pixel.
    pub fn set_forward_reflections(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        set: Option<crate::irradiance::ReflectionSet<'_>>,
    ) -> bool {
        let data = crate::irradiance::reflection_uniform(set.as_ref());
        queue.write_buffer(&self.forward_probes.uniform, 0, bytemuck::cast_slice(&data));
        let shape = set.as_ref().map_or(ProbeShape::default(), |s| ProbeShape::of(&s.probes));
        // Called when the probes change, which is rare, so every view's group 0 is simply built again.
        self.forward_probes.view = set.map(|s| s.view.clone());
        let behind = self.behind_view.clone();
        self.set_behind_view(device, Some(&behind));
        let changed = shape != self.forward_probes.shape;
        self.forward_probes.shape = shape;
        changed
    }

    /// Draws the environment cube again if this frame's sky, presets or sun changed what it shows; true if
    /// it did. The forward-only path's lit shader reads the cube. The desktop path works the environment
    /// out per pixel, next to its reflection probes, and has no reason to call this.
    pub fn update_environment(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        uniforms: &crate::gpu_types::SceneUniforms,
    ) -> bool {
        self.env_cube.update(device, queue, encoder, uniforms)
    }

    /// Lights the scene from `map`, reflects it and draws it behind the scene, on the forward-only path,
    /// or gives the sky back with `None`. A map at another size than the cube's makes a new cube, and every
    /// view's group 0 is built again to see it. The desktop path takes a map through its reflection probes.
    pub fn set_environment_map(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        map: Option<&crate::environment::EnvironmentMap>,
    ) {
        if self.env_cube.set_map(device, queue, encoder, &self.global_bind_group_layout, map) {
            let behind = self.behind_view.clone();
            self.set_behind_view(device, Some(&behind));
        }
    }

    /// What draws the environment map behind the scene on the forward-only path, while there is one. Bind
    /// group 0 and draw three vertices.
    pub fn environment_background(&self) -> Option<&wgpu::RenderPipeline> {
        self.env_cube.background()
    }

    /// The pipeline for an unlit draw blending by alpha, or not at all. `discards` says it may discard.
    pub fn unlit_for(&self, discards: bool) -> &wgpu::RenderPipeline {
        self.unlit_keeps_pipeline.as_ref().filter(|_| discards).unwrap_or(&self.unlit_pipeline)
    }

    /// The pipeline for a lit, non-custom draw, on the tier it needs.
    ///
    /// Desktop has one tier. On the forward-only path a phone pays for code whether or not a draw takes
    /// it, and not in proportion to it either (see `shader.wgsl`), so a plain surface has a pipeline of its
    /// own: single-sided and opaque, with no discard in it. A double-sided or blended draw's pipeline
    /// has `DETAILS` already, since its back faces show. `details` asks for them on a single-sided opaque
    /// draw (see [`routing::needs_details`](crate::routing::needs_details)), and `lobes` says the material
    /// uses anisotropy, clear coat or subsurface, which is the tier above.
    pub fn lit_pipeline(
        &self,
        lobes: bool,
        details: bool,
        transparent: bool,
        double_sided: bool,
        mode: crate::components::BlendMode,
    ) -> &wgpu::RenderPipeline {
        let index = crate::components::BlendMode::VARIANTS.iter().position(|m| *m == mode);
        match self.lit_lobes.as_ref().filter(|_| lobes) {
            Some(set) => match (transparent, index) {
                (true, Some(i)) => &set.blend[i],
                (true, None) => &set.transparent,
                (false, _) if double_sided => &set.render_double_sided,
                (false, _) => &set.render,
            },
            None => match (transparent, index) {
                (true, Some(i)) => &self.blend_lit_pipelines[i],
                (true, None) => &self.transparent_pipeline,
                (false, _) if double_sided => &self.render_double_sided_pipeline,
                (false, _) => self
                    .render_details_pipeline
                    .as_ref()
                    .filter(|_| details)
                    .unwrap_or(&self.render_pipeline),
            },
        }
    }
}

/// Reflection probes as the forward-only path reads them, on group 0: the captured cube array (a
/// placeholder while there is none), a sampler and the uniform that places them. See
/// [`SceneState::set_forward_reflections`].
pub(crate) struct ForwardProbes {
    placeholder: wgpu::TextureView,
    view: Option<wgpu::TextureView>,
    sampler: wgpu::Sampler,
    uniform: wgpu::Buffer,
    /// What the lit pipelines are built to read.
    pub(crate) shape: ProbeShape,
}

/// Which kinds of reflection probe a scene has, which is what the forward-only path's lit shader is
/// compiled for: `PROBE_BACKGROUND` reads the background probe where the sky would be, and `PROBE_BOXES`
/// blends the boxed ones over that. Neither, and the shader has no probe code at all.
///
/// Frame-constant, so decided when the pipelines are built rather than per fragment: on an Adreno 618 a
/// mirror floor read 107 fps without a probe and 77 with one when the shader asked the uniform every
/// pixel and worked out the sky it then replaced (2026-11-08).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct ProbeShape {
    pub(crate) background: bool,
    pub(crate) boxes: bool,
}

impl ProbeShape {
    /// The shape of `probes`, as many of them as the shader reads.
    pub(crate) fn of(probes: &[crate::irradiance::ReflectionSource]) -> Self {
        let probes = &probes[..probes.len().min(crate::reflection::MAX_PROBES)];
        Self {
            background: probes.iter().any(|p| p.box_half_extents.is_none()),
            boxes: probes.iter().any(|p| p.box_half_extents.is_some()),
        }
    }

    /// The shader defs that build a lit shader for it.
    #[cfg_attr(not(gizmo_forward_only), allow(dead_code))]
    pub(crate) fn defs(self) -> [(&'static str, naga_oil::compose::ShaderDefValue); 2] {
        [
            ("PROBE_BACKGROUND", naga_oil::compose::ShaderDefValue::Bool(self.background)),
            ("PROBE_BOXES", naga_oil::compose::ShaderDefValue::Bool(self.boxes)),
        ]
    }
}

impl ForwardProbes {
    fn new(device: &wgpu::Device) -> Self {
        let placeholder = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("no reflection probes"),
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
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("reflection probes"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        let uniform = wgpu::util::DeviceExt::create_buffer_init(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("reflection probes"),
                contents: bytemuck::cast_slice(&crate::irradiance::reflection_uniform(None)),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            },
        );
        Self { placeholder, view: None, sampler, uniform, shape: ProbeShape::default() }
    }

    fn view(&self) -> &wgpu::TextureView {
        self.view.as_ref().unwrap_or(&self.placeholder)
    }
}

/// Group 0 for one view: its uniforms and light lists, the frame behind glass, the environment cube and
/// the forward-only path's reflection probes. Every place that builds one goes through here, so a binding
/// cannot be left out of one of them.
fn global_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    label: &str,
    [uniform, table, indices]: [&wgpu::Buffer; 3],
    behind: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    environment: &wgpu::TextureView,
    probes: &ForwardProbes,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some(label),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: uniform.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: table.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 2, resource: indices.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(behind) },
            wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(sampler) },
            wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(environment) },
            wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(probes.view()) },
            wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::Sampler(&probes.sampler) },
            wgpu::BindGroupEntry { binding: 8, resource: probes.uniform.as_entire_binding() },
        ],
    })
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
    let cookies = crate::cookies::CookieTextures::new(device);

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

    let behind_fallback = device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("nothing behind glass yet"),
            size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default());
    let behind_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("behind glass"),
        address_mode_u: wgpu::AddressMode::ClampToEdge,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    });
    let env_cube = crate::env_cube::EnvironmentCube::new(device);
    let forward_probes = ForwardProbes::new(device);
    let global_bind_group = global_group(
        device,
        &layouts.global,
        "global_bind_group",
        [&global_uniform_buffer, &cluster_table_buffer, &cluster_index_buffer],
        &behind_fallback,
        &behind_sampler,
        env_cube.view(),
        &forward_probes,
    );

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
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&cookies.view),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::Sampler(&cookies.sampler),
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
            contents: bytemuck::bytes_of(&ShadowVsUniform::new(id4, [0.0; 3], 0.0)),
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
            contents: bytemuck::bytes_of(&ShadowVsUniform::new(id4, [0.0; 3], 0.0)),
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

    let wire_color_layout = layouts::wire_color_layout(device);
    let indirect = crate::irradiance::IrradianceState::layout(device);
    let layout_refs = LayoutRefs {
        global: &layouts.global,
        shadow: &layouts.shadow,
        shadow_pass: &layouts.shadow_pass,
        texture: &layouts.texture,
        skeleton: &layouts.skeleton,
        instance: &layouts.instance,
        indirect: &indirect,
        wire_color: &wire_color_layout,
    };
    let core_pipelines = build_core_pipelines(device, &layout_refs, ProbeShape::default());
    let pipelines::ShadowPipelines {
        plain: [shadow_pipeline, shadow_open_pipeline],
        cut_out: shadow_cut_out_pipelines,
    } = build_shadow_pipelines(device, &layout_refs);

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
        water_double_sided_pipeline: core_pipelines.water_double_sided,
        mirror_pipeline: core_pipelines.mirror,
        water_mirror_pipeline: core_pipelines.water_mirror,
        transparent_pipeline: core_pipelines.transparent,
        grid_pipeline: core_pipelines.grid,
        blend_lit_pipelines: core_pipelines.blend_lit,
        blend_unlit_pipelines: core_pipelines.blend_unlit,
        lit_lobes: core_pipelines.lobes,
        render_details_pipeline: core_pipelines.render_details,
        unlit_keeps_pipeline: core_pipelines.unlit_keeps,
        wire_color_layout,
        wire_colors: std::collections::HashMap::new(),
        transmissive: core_pipelines.transmissive,
        behind_view: behind_fallback.clone(),
        env_cube,
        forward_probes,
        behind_fallback,
        behind_sampler,
        shadow_pipeline,
        shadow_open_pipeline,
        shadow_cut_out_pipelines,
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
        cookies,
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

/// The layouts a [`SceneState`] was built with, as the pipeline builders take them. `indirect` is the
/// irradiance volume's group layout, which the scene does not keep.
fn layout_refs<'a>(scene: &'a SceneState, indirect: &'a wgpu::BindGroupLayout) -> LayoutRefs<'a> {
    LayoutRefs {
        global: &scene.global_bind_group_layout,
        shadow: &scene.shadow_bind_group_layout,
        shadow_pass: &scene.shadow_pass_bind_group_layout,
        texture: &scene.texture_bind_group_layout,
        skeleton: &scene.skeleton_bind_group_layout,
        instance: &scene.instance_bind_group_layout,
        indirect,
        wire_color: &scene.wire_color_layout,
    }
}

/// The layout a lit forward pipeline has on this path, for one built outside this module: a game's
/// surface ([`SurfaceMaterial`](crate::custom_material::SurfaceMaterial)).
pub(crate) fn lit_pipeline_layout(device: &wgpu::Device, scene: &SceneState) -> wgpu::PipelineLayout {
    let indirect = crate::irradiance::IrradianceState::layout(device);
    pipelines::lit_pipeline_layout(device, &layout_refs(scene, &indirect))
}

#[tracing::instrument(skip_all)]
/// Recompiles every shader and rebuilds every pipeline in place, keeping the existing buffers,
/// textures and bind groups — what [`Renderer::rebuild_shaders`](crate::Renderer::rebuild_shaders)
/// calls on a hot reload.
pub fn rebuild_pipelines(renderer: &mut crate::Renderer) {
    let device = &renderer.device;
    let post_shader = crate::post_process::post_shader_module(device);

    // Geçici LayoutRefs tutucusu, render pipeline'ı için mevcut layoutları referans alır
    let indirect = crate::irradiance::IrradianceState::layout(device);
    let layouts = layout_refs(&renderer.scene, &indirect);

    let core_pipelines = build_core_pipelines(device, &layouts, renderer.scene.forward_probes.shape);
    let pipelines::ShadowPipelines {
        plain: [shadow_pipeline, shadow_open_pipeline],
        cut_out: shadow_cut_out_pipelines,
    } = build_shadow_pipelines(device, &layouts);

    renderer.scene.lit_lobes = core_pipelines.lobes;
    renderer.scene.render_details_pipeline = core_pipelines.render_details;
    renderer.scene.unlit_keeps_pipeline = core_pipelines.unlit_keeps;
    renderer.scene.transmissive = core_pipelines.transmissive;
    renderer.scene.render_pipeline = core_pipelines.render;
    renderer.scene.render_double_sided_pipeline = core_pipelines.render_double_sided;
    renderer.scene.wireframe_pipeline = core_pipelines.wireframe;
    renderer.scene.unlit_pipeline = core_pipelines.unlit;
    renderer.scene.baked_lit_pipeline = core_pipelines.baked_lit;
    renderer.scene.baked_lit_transparent_pipeline = core_pipelines.baked_lit_transparent;
    renderer.scene.sky_pipeline = core_pipelines.sky;
    renderer.scene.backdrop_pipeline = core_pipelines.backdrop;
    renderer.scene.water_pipeline = core_pipelines.water;
    renderer.scene.water_double_sided_pipeline = core_pipelines.water_double_sided;
    renderer.scene.mirror_pipeline = core_pipelines.mirror;
    renderer.scene.water_mirror_pipeline = core_pipelines.water_mirror;
    renderer.scene.transparent_pipeline = core_pipelines.transparent;
    renderer.scene.grid_pipeline = core_pipelines.grid;
    renderer.scene.blend_lit_pipelines = core_pipelines.blend_lit;
    renderer.scene.blend_unlit_pipelines = core_pipelines.blend_unlit;
    renderer.scene.shadow_pipeline = shadow_pipeline;
    renderer.scene.shadow_open_pipeline = shadow_open_pipeline;
    renderer.scene.shadow_cut_out_pipelines = shadow_cut_out_pipelines;

    crate::post_process::rebuild_post_pipelines(renderer, &post_shader);

    // Text's two pipelines, rebuilt from the same disk-first source as everything above — without
    // this, `text.wgsl` is the one shader in the engine that hot reload silently ignores. The
    // fonts and the rasterised glyphs are kept; only the pipelines are replaced.
    if let Some(text) = renderer.text.as_mut() {
        text.rebuild_pipelines(
            &renderer.device,
            &renderer.scene.global_bind_group_layout,
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureFormat::Depth32Float,
        );
    }
    if let Some(sprites) = renderer.sprites.as_mut() {
        sprites.rebuild_pipelines(
            &renderer.device,
            wgpu::TextureFormat::Rgba16Float,
            wgpu::TextureFormat::Depth32Float,
        );
    }
    // The deferred path's own, and then every game surface, which is composed from both: a surface
    // that kept its pipelines would be drawn by the old shaders beside everything drawn by the new.
    if let Some(deferred) = renderer.deferred.as_mut() {
        deferred.rebuild_pipelines(&renderer.device, &renderer.scene);
    }
    renderer.custom_materials.rebuild_surfaces(&renderer.device, &renderer.scene, renderer.deferred.as_ref());
    tracing::info!("[Pipeline] core + shadow + deferred + post-process + text + sprite + surface pipelines rebuilt");
}

#[cfg(test)]
mod tests {
    /// **The forward-only path's plain lit shader carries no anisotropy, clear coat or subsurface.**
    /// An Adreno 618 pays for those branches even when no material takes them: 20 fps against 43,
    /// drawing a cube and a floor. Only a phone can measure that, so this holds the cause in place.
    #[test]
    fn the_plain_web_shader_leaves_the_lobes_out_and_the_variant_has_them() {
        use super::shaders::{compose_wgsl, native_render_defs, web_render_defs};
        use naga_oil::compose::ShaderDefValue::Bool;
        let src = include_str!("../shaders/shader.wgsl");
        let lobes = ["compute_direct_lighting_anisotropic", "layer_clear_coat", "subsurface_light"];
        // The builtin, not the word: `gizmo::surface::SurfaceInput` has a field of that name, and an
        // unused type is no code.
        let details = ["@builtin(front_facing)", "discard", "dielectric_f0", "instance_keeps"];
        let plain = compose_wgsl(src, "shader.wgsl", web_render_defs());
        let mut defs = web_render_defs();
        defs.insert("DETAILS".to_string(), Bool(true));
        let with_details = compose_wgsl(src, "shader.wgsl", defs.clone());
        defs.insert("LOBES".to_string(), Bool(true));
        let with_lobes = compose_wgsl(src, "shader.wgsl", defs);
        let desktop = compose_wgsl(src, "shader.wgsl", native_render_defs());
        for lobe in lobes {
            assert!(!plain.contains(lobe), "the plain web shader has {lobe}");
            assert!(!with_details.contains(lobe), "the details tier has {lobe}");
            assert!(with_lobes.contains(lobe), "the web variant lost {lobe}");
            assert!(desktop.contains(lobe), "the desktop shader lost {lobe}");
        }
        for detail in details {
            assert!(!plain.contains(detail), "a plain surface pays for {detail} on a phone");
            assert!(with_details.contains(detail), "the details tier lost {detail}");
            assert!(with_lobes.contains(detail), "the lobes tier lost {detail}");
            assert!(desktop.contains(detail), "the desktop shader lost {detail}");
        }
        // The entry points a blended draw needs are in every tier that draws one.
        assert!(with_details.contains("fs_additive") && !plain.contains("fs_additive"));

        let unlit = include_str!("../shaders/unlit.wgsl");
        assert!(!compose_wgsl(unlit, "unlit.wgsl", web_render_defs()).contains("discard"));
        let mut defs = web_render_defs();
        defs.insert("KEEPS".to_string(), Bool(true));
        assert!(compose_wgsl(unlit, "unlit.wgsl", defs).contains("discard"));
        assert!(compose_wgsl(unlit, "unlit.wgsl", native_render_defs()).contains("discard"));
    }

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
            let post = crate::post_process::post_shader_source(
                include_str!("../shaders/post_common.wgsl"),
                include_str!("../shaders/post_process.wgsl"),
            );
            let post_reduced = crate::post_process::post_shader_source(
                include_str!("../shaders/post_common.wgsl"),
                include_str!("../shaders/post_process_wasm.wgsl"),
            );
            let shaders: &[(&str, &str)] = &[
                ("shader.wgsl", include_str!("../shaders/shader.wgsl")),
                ("gbuffer.wgsl", include_str!("../shaders/gbuffer.wgsl")),
                ("deferred_lighting.wgsl", include_str!("../shaders/deferred_lighting.wgsl")),
                ("env_bake.wgsl", include_str!("../shaders/env_bake.wgsl")),
                ("environment_sky.wgsl", include_str!("../shaders/environment_sky.wgsl")),
                ("post_process.wgsl", post.as_str()),
                ("post_process_wasm.wgsl", post_reduced.as_str()),
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
                ("planar_mirror.wgsl", include_str!("../shaders/planar_mirror.wgsl")),
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
                ("text.wgsl", include_str!("../shaders/text.wgsl")),
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
                "env_bake.wgsl",
                "environment_sky.wgsl",
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
                    if *name == "unlit.wgsl" {
                        let mut defs = shaders::web_render_defs();
                        defs.insert("KEEPS".to_string(), naga_oil::compose::ShaderDefValue::Bool(true));
                        variants.push(("web keeps", shaders::compose_wgsl(src, name, defs)));
                    }
                    if *name == "shader.wgsl" {
                        // Every `ProbeShape` a scene can have, plain and with every tier.
                        for (label, shape) in [
                            ("background probe", ["PROBE_BACKGROUND"].as_slice()),
                            ("boxed probes", ["PROBE_BOXES"].as_slice()),
                            ("both kinds of probe", ["PROBE_BACKGROUND", "PROBE_BOXES"].as_slice()),
                        ] {
                            let mut probes = shaders::web_render_defs();
                            for def in shape {
                                probes.insert(def.to_string(), naga_oil::compose::ShaderDefValue::Bool(true));
                            }
                            variants.push((label, shaders::compose_wgsl(src, name, probes.clone())));
                            for tier in ["DETAILS", "LOBES", "TRANSMISSION"] {
                                probes.insert(tier.to_string(), naga_oil::compose::ShaderDefValue::Bool(true));
                            }
                            variants.push((label, shaders::compose_wgsl(src, name, probes)));
                        }
                        let mut defs = shaders::web_render_defs();
                        defs.insert("DETAILS".to_string(), naga_oil::compose::ShaderDefValue::Bool(true));
                        variants.push(("web with details", shaders::compose_wgsl(src, name, defs.clone())));
                        defs.insert("LOBES".to_string(), naga_oil::compose::ShaderDefValue::Bool(true));
                        variants.push(("web with lobes", shaders::compose_wgsl(src, name, defs.clone())));
                        defs.insert("TRANSMISSION".to_string(), naga_oil::compose::ShaderDefValue::Bool(true));
                        variants.push(("web glass", shaders::compose_wgsl(src, name, defs)));
                        let mut defs = shaders::native_render_defs();
                        defs.insert("TRANSMISSION".to_string(), naga_oil::compose::ShaderDefValue::Bool(true));
                        variants.push(("native glass", shaders::compose_wgsl(src, name, defs)));
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
