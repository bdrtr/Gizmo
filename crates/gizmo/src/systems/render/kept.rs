//! What the draw collection keeps of each object from one collection to the next.
//!
//! `collect_draw_items` used to work everything out about every object, every collection: its
//! world matrix and box, its instance data, its batch key and the batch that key names. In a city
//! that does not move it is the same answer for every object, every frame, and getting it meant
//! reading the object's `Mesh`, `Material`, `GlobalTransform` and `MeshRenderer` — a few hundred
//! bytes each, apart from one another. Sampled in `nfsu2-gizmo`'s city (14 600 objects, 11 495 of
//! them drawn), those reads, the box transform, the visibility test and the key comparison were
//! nearly all of the collection's 11.4 ms on the integrated GPU, whose CPU shares its memory with
//! the GPU drawing the last frame; the same code took 3.5 ms beside a discrete one (2026-11-18).
//!
//! What does not depend on the view is now [`Prepared`] once and [`Kept`] per entity, and worked out
//! again only when something it was made from changed: the entity's `Mesh`, `GlobalTransform`,
//! `Material`, `MeshRenderer` or `PlanarReflection`, written since the last collection
//! (`World::query_since`). [`KeptScene::begin`] drops those entities' entries before the walk, so an
//! entry the walk finds was made from what the entity holds now, and an entity without one goes
//! the whole way, as every entity did before, and leaves one behind. What depends on the view —
//! culling, the detail level, the views that skip mirrors or the sky, `IsHidden` — is still
//! decided every collection, from the kept box and flags. Not kept: a skinned object (its skeleton
//! is posed for its drawn size), an object with a `LodGroup` (its mesh is picked by distance), a
//! backdrop (its box follows the camera), and nothing at all on a frame that draws wireframes, whose
//! line colour is part of a batch's key and is worked out per object.

use super::batching::MeshRange;
use crate::math::{Mat4, Vec3};
use crate::renderer::components::{Material, MaterialType, Mesh, MeshRenderer};
use gizmo_core::Entity;
use gizmo_physics_core::components::GlobalTransform;

/// Everything about one drawn (entity, mesh) that does not depend on the view.
#[derive(Clone, Copy)]
pub(super) struct Prepared {
    /// Which passes and pipelines the material goes to (`gizmo_renderer::routing`).
    pub(super) routing: crate::renderer::routing::Routing,
    /// The box it is drawn in, in the world.
    pub(super) aabb: crate::math::Aabb,
    /// The centre of its mesh's box in the world, and the box's radius scaled by the largest axis:
    /// what the detail level and a skeleton's drawn size are measured with.
    pub(super) centre: Vec3,
    pub(super) radius: f32,
    /// The detail level's distance bias (`MeshRenderer::lod_bias`).
    pub(super) lod_bias: f32,
    /// Whether the mesh has a second detail level (`Mesh::lod_vbufs`).
    pub(super) has_lod1: bool,
    /// What `classify_visibility_world` asks of the material.
    pub(super) material_type: MaterialType,
    pub(super) material_transparent: bool,
    pub(super) alpha: f32,
    /// `MeshRenderer::shadows.casts()`.
    pub(super) casts: bool,
    /// What each detail level draws of the batch's buffers: the mesh, then its second level.
    pub(super) ranges: [MeshRange; 2],
    /// The instance as uploaded.
    pub(super) instance: crate::renderer::gpu_types::InstanceRaw,
}

/// Works out what `mesh` on `trans` with `mat` draws, apart from the view: the part of
/// `collect_draw_items`' walk that reads the components. `cam_pos` is its one input that changes
/// without the components changing — a backdrop's box and model follow the camera — which is why a
/// backdrop is never kept. A crossfading `LodGroup` level's fade is not in it either: the walk adds
/// it to the instance (`InstanceRaw::with_lod_fade`, which writes only the cut-out's slot, after
/// everything here), and such a level is never kept.
pub(super) fn prepare(
    mesh: &Mesh,
    trans: &GlobalTransform,
    mat: &Material,
    mesh_renderer: &MeshRenderer,
    reflection: Option<&crate::renderer::components::PlanarReflection>,
    cam_pos: Vec3,
) -> Prepared {
    let routing = crate::renderer::routing::route(mat.material_type);
    let center_mat = Mat4::from_translation(mesh.center_offset);
    let model = trans.matrix * center_mat;

    // Where this mesh is actually DRAWN. The same matrix for everything except a camera-locked
    // backdrop, whose vertex shader adds the camera position itself (`backdrop.wgsl`) — so its
    // authored transform is not where the triangles end up, and culling or LOD-ing against it
    // reasons about the wrong place. The INSTANCE below keeps the authored matrix: hand the shader
    // this one and it adds the camera position twice.
    let drawn_model = crate::renderer::backdrop::camera_locked_model(mat.material_type, &model, cam_pos);

    let local_cx = (mesh.bounds.min.x + mesh.bounds.max.x) * 0.5;
    let local_cy = (mesh.bounds.min.y + mesh.bounds.max.y) * 0.5;
    let local_cz = (mesh.bounds.min.z + mesh.bounds.max.z) * 0.5;
    let centre = drawn_model.transform_point3(Vec3::new(local_cx, local_cy, local_cz));
    let hx = (mesh.bounds.max.x - mesh.bounds.min.x) * 0.5;
    let hy = (mesh.bounds.max.y - mesh.bounds.min.y) * 0.5;
    let hz = (mesh.bounds.max.z - mesh.bounds.min.z) * 0.5;
    let local_r = (hx * hx + hy * hy + hz * hz).sqrt();
    let sx = drawn_model.x_axis.truncate().length();
    let sy = drawn_model.y_axis.truncate().length();
    let sz = drawn_model.z_axis.truncate().length();
    let radius = local_r * sx.max(sy).max(sz);
    // ONE `Aabb::transform` for every decision about this object: culling, the shadow maps, the
    // scene's extent.
    let aabb = mesh.bounds.transform(&drawn_model);

    // What the shader is handed, which is not always what was authored: a PLACED backdrop rides the
    // same pipeline as a locked one, and that pipeline adds the camera position in the vertex
    // shader. `instance_model` takes it back out so the two cancel and the geometry lands where the
    // level put it. Identity for everything else. See `renderer::backdrop`.
    let upload_model = crate::renderer::backdrop::instance_model(mat.material_type, &model, cam_pos);
    let mut instance = crate::renderer::gpu_types::InstanceRaw::new(
        upload_model.to_cols_array_2d(),
        [mat.albedo.x, mat.albedo.y, mat.albedo.z, mat.albedo.w],
        mat.roughness,
        mat.metallic,
        routing.instance_flag,
        mat.anisotropy,
        mat.clear_coat,
        mat.subsurface,
        // The two `Material` lighting knobs. Zero unless the material sets them, and the shader's
        // `(lit + ambient) * base + emissive` then collapses to exactly the multiply chain it was
        // before they existed.
        mat.ambient.to_array(),
        mat.emissive.to_array(),
    )
    // The cut-out threshold rides in `ambient.w`, which was padding. Zero unless the material
    // asks, and zero is what every shader already reads there.
    .with_alpha_cutoff(mat.alpha_cutoff)
    .with_reflectance(mat.reflectance)
    .with_lit_extras_of(mat)
    .with_shadow_receiving(mesh_renderer.receives_shadows);
    // How much a mirror reflects, for its shader alone: no other pipeline draws this batch while it
    // has a capture, and the plain one ignores `ambient`.
    if let Some(reflection) = reflection {
        instance.ambient[0] = reflection.reflectance;
    }

    // What each level draws of the buffers it is in: 0 onwards for a mesh with buffers of its own,
    // its place in a shared one for a mesh made in a `MeshArena`. The second level is a flattened
    // triangle list (`Mesh::new` unpacks meshopt's output), drawn without the mesh's indices.
    let full = match mesh.ibuf {
        Some(_) => MeshRange { first: mesh.first_index, count: mesh.index_count, base_vertex: mesh.first_vertex as i32 },
        None => MeshRange { first: mesh.first_vertex, count: mesh.vertex_count, base_vertex: 0 },
    };
    let has_lod1 = !mesh.lod_vbufs.is_empty();
    let lod1 = if has_lod1 {
        MeshRange {
            first: mesh.lod_first_vertices.first().copied().unwrap_or(0),
            count: mesh.lod_vertex_counts[0],
            base_vertex: 0,
        }
    } else {
        MeshRange::default()
    };

    Prepared {
        routing,
        aabb,
        centre,
        radius,
        lod_bias: mesh_renderer.lod_bias,
        has_lod1,
        material_type: mat.material_type,
        material_transparent: mat.is_transparent,
        alpha: mat.albedo.w,
        casts: mesh_renderer.shadows.casts(),
        ranges: [full, lod1],
        instance,
    }
}

impl Prepared {
    /// Which detail level the view draws: the second, when the mesh has one and its box is more
    /// than fifteen of its radii away (scaled by the object's bias) — a 2 m object switches at 30 m.
    pub(super) fn lod_for(&self, cam_pos: Vec3) -> usize {
        let distance = crate::renderer::components::effective_lod_distance((self.centre - cam_pos).length(), self.lod_bias);
        usize::from(self.has_lod1 && distance > self.radius * 15.0)
    }
}

/// One entity's [`Prepared`], with the batch each of its detail levels went to.
#[derive(Clone, Copy)]
pub(super) struct Kept {
    /// Whose: an id is taken again by a later entity, with another generation.
    pub(super) entity: Entity,
    /// Whether it had a `PlanarReflection` when it was prepared. Taking the component off writes
    /// nothing `Changed` can see, so it is compared instead.
    pub(super) mirror: bool,
    /// Per detail level, the batch slot it went to and that batch's serial: a batch let go and its
    /// slot given to another fails the check, and the object goes the whole way again.
    pub(super) slots: [Option<(u32, u64)>; 2],
    pub(super) prepared: Prepared,
}

/// Every kept entry of one world, by entity id.
#[derive(Default)]
pub(super) struct KeptScene {
    /// The world they were made from (`World::id`): a thread's cache serves any world it draws.
    world: Option<u64>,
    /// The tick the last collection ran at, less one: what changed since is news.
    since: u32,
    /// `Renderer::shading_mode` when they were made: it picks the forward-only path's pipelines,
    /// and with them batch keys.
    shading_mode: u32,
    entries: Vec<Option<Kept>>,
}

impl KeptScene {
    /// Readies the entries for a collection of `world`: drops every one whose entity's components
    /// changed since the last collection, or all of them when the world or the shading mode is not
    /// the one they were made for. `usable` false — a frame drawing wireframes — drops them all and
    /// keeps none; the answer is whether the walk may use and leave entries.
    pub(super) fn begin(&mut self, world: &gizmo_core::World, shading_mode: u32, usable: bool) -> bool {
        use gizmo_core::query::{Changed, Or};
        if !usable || self.world != Some(world.id()) || self.shading_mode != shading_mode {
            self.entries.clear();
            self.since = 0;
            self.world = usable.then(|| world.id());
            self.shading_mode = shading_mode;
            if !usable {
                return false;
            }
        }
        if !self.entries.is_empty() {
            let changed = world.query_since::<
                Entity,
                Or<
                    Changed<Mesh>,
                    Or<
                        Changed<GlobalTransform>,
                        Or<Changed<Material>, Or<Changed<MeshRenderer>, Changed<crate::renderer::components::PlanarReflection>>>,
                    >,
                >,
            >(self.since);
            for entity in changed.iter() {
                if let Some(entry) = self.entries.get_mut(entity.id() as usize) {
                    *entry = None;
                }
            }
        }
        self.since = world.tick.wrapping_sub(1);
        true
    }

    /// `entity`'s entry, if it has one made from what it holds now — `mirror`: whether it has a
    /// `PlanarReflection` now.
    #[inline]
    pub(super) fn get(&self, entity: Entity, mirror: bool) -> Option<&Kept> {
        self.entries
            .get(entity.id() as usize)
            .and_then(Option::as_ref)
            .filter(|kept| kept.entity == entity && kept.mirror == mirror)
    }

    /// Leaves `prepared` as `entity`'s entry, keeping the batches its levels went to if the entry it
    /// replaces is this entity's and was made from the same components.
    pub(super) fn store(&mut self, entity: Entity, mirror: bool, prepared: &Prepared) {
        let id = entity.id() as usize;
        if self.entries.len() <= id {
            self.entries.resize(id + 1, None);
        }
        let slots = self.get(entity, mirror).map_or([None; 2], |kept| kept.slots);
        self.entries[id] = Some(Kept { entity, mirror, slots, prepared: *prepared });
    }

    /// Notes that `entity`'s detail level `lod` went to the batch in `slot`, of `serial`.
    pub(super) fn note_slot(&mut self, entity: Entity, lod: usize, slot: u32, serial: u64) {
        if let Some(Some(kept)) = self.entries.get_mut(entity.id() as usize) {
            if kept.entity == entity {
                kept.slots[lod] = Some((slot, serial));
            }
        }
    }
}
