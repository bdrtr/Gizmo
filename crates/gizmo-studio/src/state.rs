/// Per-application runtime state for Gizmo Studio (FPS, camera entities,
/// timers and frame statistics) carried across the engine's update loop.
pub struct StudioState {
    /// The smoothed frame rate shown in the status bar.
    pub current_fps: f32,
    /// The last frame's real duration in seconds, before any time scaling.
    pub actual_dt: f32,
    /// Entity id of the editor's own camera — the one the viewport flies.
    pub editor_camera: u32,
    /// Entity id of the camera the game view renders through.
    pub game_camera: u32,
    /// Set by the scene view when a click needs picking; cleared once the raycast has run.
    pub do_raycast: bool,
    /// The running game's frame — the *same* loop an exported game runs
    /// ([`gizmo::systems::PlayLoop`]). It carries the physics debt and which scripts are currently
    /// failing to load; both used to be fields here, next to a copy of the loop that used them.
    pub play: gizmo::systems::PlayLoop,
    /// Watches `demo/assets` for changes; `None` when the directory could not be watched.
    pub asset_watcher: Option<gizmo::renderer::hot_reload::AssetWatcher>,
    /// The garbage-collection timer — cleans up soft-deleted entities
    pub gc_timer: f32,
    /// The auto-save timer — backs the scene up at a fixed interval
    pub autosave_timer: f32,
    /// How many entities in the scene are active (visible)
    pub visible_entity_count: u32,
    /// The draw-call count of the last frame
    pub draw_call_count: u32,
}

/// GPU resources used to render editor debug gizmos (primitive meshes and a
/// default white texture bind group), and the meshes the `➕ Add` menu spawns.
pub struct DebugAssets {
    /// The cube mesh the `➕ Add` menu spawns.
    pub cube: gizmo::renderer::components::Mesh,
    /// The sphere mesh.
    pub sphere: gizmo::renderer::components::Mesh,
    /// The plane quad.
    pub plane: gizmo::renderer::components::Mesh,
    /// The cylinder mesh.
    pub cylinder: gizmo::renderer::components::Mesh,
    /// The capsule mesh.
    pub capsule: gizmo::renderer::components::Mesh,
    /// A 1×1 white texture's bind group, used as the default material for everything above.
    pub white_tex: std::sync::Arc<gizmo::wgpu::BindGroup>,
}

/// The dimensions every spawned primitive is built at — **the mesh and the collider both read
/// from here**.
///
/// One table because they are two readers of one number, and they had already drifted apart: the
/// sphere was spawned with a mesh of radius 0.5 and a collider of radius 1.0, so the collision
/// shape was twice the size of the thing you could see and a dropped sphere came to rest half a
/// radius above the floor. Nothing pointed that out, because the two numbers were written three
/// hundred lines apart in two different files.
pub struct PrimitiveSize;

impl PrimitiveSize {
    /// Half-extent of the cube. Not a parameter but a *fact*: `AssetManager::create_cube` has
    /// `±1.0` written into its vertex table and takes no size, so this is the number the collider
    /// has to agree with.
    pub const CUBE_HALF: f32 = 1.0;
    /// Radius of the sphere mesh, and of its collider — the pair that had drifted apart.
    pub const SPHERE_RADIUS: f32 = 0.5;
    /// Edge length of the plane quad; it is centred on the origin, so it spans ±half.
    pub const PLANE_SIZE: f32 = 10.0;
    /// The plane mesh has no thickness at all, and a zero-height box is a degenerate collider.
    /// It gets a thin one pushed *down* by its own half-height, so the top face lands exactly on
    /// the visible quad rather than hovering a centimetre over it.
    pub const PLANE_THICKNESS: f32 = 0.05;
    /// Radius of the cylinder mesh, and of its collider.
    pub const CYLINDER_RADIUS: f32 = 0.5;
    /// Full height of the cylinder mesh. `Collider::cylinder` takes half of it — see
    /// [`cylinder_collider_half_height`](Self::cylinder_collider_half_height).
    pub const CYLINDER_HEIGHT: f32 = 2.0;
    /// Radial segment count for the cylinder **mesh**.
    ///
    /// It used to be shared with the collider, because the engine had no cylinder shape and the
    /// studio stood a 24-sided convex hull in for one. Since `ColliderShape::Cylinder` exists
    /// (2026-08-17) the collider is a real cylinder and this number is the visual only — a
    /// coarser mesh no longer changes what the physics sees.
    pub const CYLINDER_SEGMENTS: u32 = 24;
    /// Radius of the capsule mesh, and of its collider.
    pub const CAPSULE_RADIUS: f32 = 0.5;
    /// Length of the capsule's *cylindrical* section. The mesh takes this whole length as `depth`
    /// and `Collider::capsule` takes half of it as `half_height` — the one conversion between the
    /// two, and the reason it is written down once.
    pub const CAPSULE_DEPTH: f32 = 1.0;

    /// `half_height` for the capsule's collider.
    ///
    /// The one unit conversion in this table, and it lives here rather than at the spawn site so a
    /// test can call the same function the spawner does: the mesh takes the cylindrical section's
    /// **whole** length as `depth`, `Collider::capsule` takes **half** of it. A dropped `* 0.5`
    /// doubles the collider and nothing on screen changes.
    pub fn capsule_collider_half_height() -> f32 {
        Self::CAPSULE_DEPTH * 0.5
    }

    /// `half_height` for the cylinder's collider.
    ///
    /// Here rather than at the spawn site for the same reason as the capsule's: a test can then
    /// call the function the spawner calls. The conversion is the same one — the mesh takes the
    /// whole height, `Collider::cylinder` takes half of it — but note the shapes disagree about
    /// what `half_height` covers: a capsule's excludes its caps, a cylinder's is simply half the
    /// solid.
    pub fn cylinder_collider_half_height() -> f32 {
        Self::CYLINDER_HEIGHT * 0.5
    }

    /// Where the plane's collider box sits relative to the entity origin.
    ///
    /// Pushed down by its own half-thickness so the box's **top face** is the quad you can see.
    /// Centred instead, it would stand a whole thickness proud of the visible surface and
    /// everything dropped on the plane would come to rest in mid-air.
    pub fn plane_collider_offset() -> gizmo::math::Vec3 {
        gizmo::math::Vec3::new(0.0, -Self::PLANE_THICKNESS, 0.0)
    }

    /// Half-extents of the plane's collider box: the quad's half-size, and its own thickness.
    pub fn plane_collider_half_extents() -> gizmo::math::Vec3 {
        gizmo::math::Vec3::new(
            Self::PLANE_SIZE * 0.5,
            Self::PLANE_THICKNESS,
            Self::PLANE_SIZE * 0.5,
        )
    }

}


#[cfg(test)]
mod primitive_size_tests {
    use super::PrimitiveSize as PS;

    /// The cylinder's collider has to be the cylinder the mesh draws.
    ///
    /// This used to check a ring of hull points, because the engine had no cylinder shape and the
    /// studio stood a 24-sided prism in for one. The shape exists now, so what has to agree is
    /// the pair of numbers — and they are read from the same function the spawner calls, not
    /// recomputed here, or the test would keep passing with the conversion deleted from the
    /// source. (That is how the old camera regression tests managed to guard nothing.)
    #[test]
    fn the_cylinder_collider_is_the_cylinder_the_mesh_draws() {
        let half = PS::cylinder_collider_half_height();
        assert!(
            (half * 2.0 - PS::CYLINDER_HEIGHT).abs() < 1e-6,
            "the collider must span the mesh's whole height, got {}",
            half * 2.0
        );
        let collider = gizmo::physics::Collider::cylinder(PS::CYLINDER_RADIUS, half);
        match collider.shape {
            gizmo::physics::ColliderShape::Cylinder(c) => {
                assert!((c.radius - PS::CYLINDER_RADIUS).abs() < 1e-6);
                assert!((c.half_height - half).abs() < 1e-6);
            }
            other => panic!("expected a cylinder shape, got {other:?}"),
        }
    }

    /// The one unit conversion in the table: the capsule mesh takes the cylindrical section's
    /// whole length, `Collider::capsule` takes half of it. Written down because it is the single
    /// place these two APIs disagree, and a missing `* 0.5` doubles the collider.
    #[test]
    fn the_capsule_collider_is_half_the_meshs_depth() {
        // Read from the SAME function the spawner calls. Recomputing `CAPSULE_DEPTH * 0.5` here
        // would have made this test pass with the conversion deleted from the source — which is
        // exactly how the old camera regression tests managed to guard nothing.
        let half = PS::capsule_collider_half_height();
        assert!(
            (half * 2.0 - PS::CAPSULE_DEPTH).abs() < 1e-6,
            "the collider's cylindrical section must be the mesh's `depth`, got {}",
            half * 2.0
        );
    }

    /// The plane's collider is pushed down by its own half-thickness so its top face is the quad
    /// you can see. Centred instead, it would stand a whole thickness proud of the visible
    /// surface and everything dropped on it would rest in mid-air.
    #[test]
    fn the_planes_collider_top_face_is_the_visible_quad() {
        let offset = PS::plane_collider_offset();
        let half = PS::plane_collider_half_extents();
        let top = offset.y + half.y;
        assert!(
            top.abs() < 1e-6,
            "the box's top face must land on the quad at y = 0, got {top}"
        );
        // ...and its footprint is the quad's, not something else that merely looks right.
        assert!((half.x * 2.0 - PS::PLANE_SIZE).abs() < 1e-6);
        assert!((half.z * 2.0 - PS::PLANE_SIZE).abs() < 1e-6);
    }
}
