use gizmo_math::{Quat, Vec3};

/// How a [`Camera`] projects the scene onto the screen.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub enum ProjectionMode {
    /// Perspective projection (the default), using the camera's `fov`.
    #[default]
    Perspective,
    /// Orthographic projection. `height` is the vertical extent of the view
    /// volume in world units; the width is derived from the aspect ratio.
    Orthographic {
        /// The vertical extent of the view volume, in world units.
        height: f32,
    },
    /// Orthographic projection that **always shows at least** `width × height` world units,
    /// whatever shape the window is: the axis the window is long in gets the extra room.
    ///
    /// [`Orthographic`](Self::Orthographic) fixes only the vertical extent, so a portrait window
    /// cuts the sides off — measured in `kule_yikici`, a 948×1028 window showed ±7.4 units across
    /// and the catapult at x = −11 was off screen. A 2D game that has to keep its whole playfield
    /// visible wants this instead; [`ProjectionMode::view_extent`] says what the window actually
    /// shows, for placing the camera against an edge.
    OrthographicFit {
        /// The smallest horizontal extent shown, in world units.
        width: f32,
        /// The smallest vertical extent shown, in world units.
        height: f32,
    },
}

impl ProjectionMode {
    /// The `(width, height)` in world units an orthographic view shows in a viewport of the given
    /// aspect ratio (width / height). `None` for a perspective view, whose extent depends on the
    /// distance.
    #[must_use]
    pub fn view_extent(self, aspect: f32) -> Option<gizmo_math::Vec2> {
        let aspect = aspect.max(0.001);
        let height = match self {
            Self::Perspective => return None,
            Self::Orthographic { height } => height,
            Self::OrthographicFit { width, height } => height.max(width / aspect),
        }
        .max(0.002);
        Some(gizmo_math::Vec2::new(height * aspect, height))
    }

    /// Whether this is one of the orthographic modes — for code outside this crate that cannot
    /// `match` the `#[non_exhaustive]` enum without a wildcard that would swallow a new mode.
    #[must_use]
    pub fn is_orthographic(self) -> bool {
        match self {
            Self::Perspective => false,
            Self::Orthographic { .. } | Self::OrthographicFit { .. } => true,
        }
    }
}

/// A 3-D camera: where it looks, how far it sees, and how the result is projected.
///
/// Where the camera **is and where it looks** are not here: both come from the entity's
/// transform — its `GlobalTransform` once synced, else its `Transform` — which is why
/// [`Camera::view`] takes a position and a rotation. An unrotated camera looks down `−Z` with `+Y`
/// up, and a camera on a car's child entity turns with the car.
///
/// Until 2026-11-15 the aim was this component's own `yaw`/`pitch` and the transform's rotation
/// was ignored, so writing `Transform::rotation` on a camera (four games did) silently aimed
/// nothing. A controller that thinks in those two angles keeps them itself and writes
/// [`Camera::rotation_from`]`(yaw, pitch)` into the rotation; [`Camera::yaw_pitch_of`] reads them
/// back.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Camera {
    /// Vertical field of view, in radians. Perspective only.
    pub fov: f32,
    /// The near clip plane, in metres. Everything closer is clipped away.
    pub near: f32,
    /// The far clip plane. The near/far ratio is what depth precision costs, so a near plane of
    /// 0.001 with a far of 10 000 will z-fight.
    pub far: f32,
    /// Exposure multiplier applied to the whole frame, before tone mapping.
    pub exposure: f32,
    /// Whether this is the camera the frame is rendered from. With several primaries the first
    /// one found wins, which is a scene authoring error rather than a supported configuration.
    pub primary: bool,
    /// Perspective (default) or orthographic projection. `#[serde(default)]` keeps
    /// scenes saved before this field was added loading as perspective.
    #[serde(default)]
    pub projection: ProjectionMode,
    /// Where on the render target this camera draws — `None` (the default) for the whole of it.
    ///
    /// Only read by the split-screen path (`gizmo::render_viewports`): it draws **every** camera
    /// that has one, each into its own rectangle, and falls back to the single `primary` camera
    /// when none does. The primary camera without one is drawn too, over the whole target, at its
    /// own [`order`](Self::order) — so a full view with a small one over it (a minimap) is the
    /// primary camera plus one camera with a viewport and a higher order. Other cameras without
    /// one are not drawn. The projection takes the rectangle's aspect ratio, so a half-width view
    /// is not stretched. `#[serde(default)]`.
    #[serde(default)]
    pub viewport: Option<Viewport>,
    /// Draw order among cameras with a [`viewport`](Self::viewport): lower first, so a higher one
    /// lands on top — a picture-in-picture rear-view mirror is a small viewport with a higher
    /// order. Ties go by entity order. `#[serde(default)]`.
    #[serde(default)]
    pub order: i32,
    /// What fills the frame where nothing is drawn — linear RGB. `None` (the default) is
    /// [`DEFAULT_BACKGROUND`](crate::frame_uniforms::DEFAULT_BACKGROUND), a dark grey, on every path:
    /// the forward-only path's was a light blue until 2026-11-11. A skybox or a backdrop still draws
    /// over it. `#[serde(default)]`.
    #[serde(default)]
    pub clear_color: Option<gizmo_math::Vec3>,
    /// How much of the pipeline draws this camera's view. [`CameraDetail::Full`] (the default) is
    /// everything the renderer has; [`CameraDetail::Simple`] is the view lit without shadows, ambient
    /// occlusion, volumetric light, bloom or mirrors — for a secondary view that costs as much as the
    /// frame's own otherwise: a minimap, a rear-view mirror, a security monitor. `#[serde(default)]`.
    #[serde(default)]
    pub detail: CameraDetail,
}

/// How much of the pipeline draws a camera's view — see [`Camera::detail`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum CameraDetail {
    /// Everything the renderer has.
    #[default]
    Full,
    /// Lit, without the sun's and the lights' shadow passes, ambient occlusion, volumetric light,
    /// bloom or mirrors. In `dunya` a minimap drawn so cost a seventh of what a full one did (measured
    /// 2026-11-16).
    Simple,
}

/// A rectangle of the render target, as **fractions** of its size with the origin at the
/// top-left — so the same viewport fits any window size. See [`Camera::viewport`].
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Viewport {
    /// Left edge, 0 = the target's left.
    pub x: f32,
    /// Top edge, 0 = the target's top.
    pub y: f32,
    /// Width, 1 = the whole target.
    pub width: f32,
    /// Height, 1 = the whole target.
    pub height: f32,
}

impl Viewport {
    /// The whole target.
    pub const FULL: Self = Self { x: 0.0, y: 0.0, width: 1.0, height: 1.0 };

    /// A rectangle from its top-left corner and size, all fractions of the target.
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self { x, y, width, height }
    }

    /// The left half — player one of a side-by-side split.
    pub const fn left_half() -> Self {
        Self::new(0.0, 0.0, 0.5, 1.0)
    }

    /// The right half.
    pub const fn right_half() -> Self {
        Self::new(0.5, 0.0, 0.5, 1.0)
    }

    /// The top half — player one of a stacked split.
    pub const fn top_half() -> Self {
        Self::new(0.0, 0.0, 1.0, 0.5)
    }

    /// The bottom half.
    pub const fn bottom_half() -> Self {
        Self::new(0.0, 0.5, 1.0, 0.5)
    }

    /// Quarter `index` (0–3, row by row from the top-left) of a four-way split; an index past 3
    /// wraps.
    pub const fn quarter(index: u32) -> Self {
        let i = index % 4;
        Self::new((i % 2) as f32 * 0.5, (i / 2) as f32 * 0.5, 0.5, 0.5)
    }

    /// This rectangle in pixels of a `width` × `height` target: `[x, y, w, h]`, clipped to the
    /// target and at least one pixel in each direction, so it is always a valid wgpu viewport.
    pub fn to_pixels(self, width: u32, height: u32) -> [f32; 4] {
        let (tw, th) = (width.max(1) as f32, height.max(1) as f32);
        let x0 = (self.x.clamp(0.0, 1.0) * tw).round();
        let y0 = (self.y.clamp(0.0, 1.0) * th).round();
        let x1 = ((self.x + self.width).clamp(0.0, 1.0) * tw).round();
        let y1 = ((self.y + self.height).clamp(0.0, 1.0) * th).round();
        let x0 = x0.min(tw - 1.0);
        let y0 = y0.min(th - 1.0);
        [x0, y0, (x1 - x0).max(1.0), (y1 - y0).max(1.0)]
    }

    /// The aspect ratio (width / height) this rectangle has on a `width` × `height` target —
    /// what the camera's projection is built with.
    pub fn aspect(self, width: u32, height: u32) -> f32 {
        let [_, _, w, h] = self.to_pixels(width, height);
        w / h
    }
}

impl Default for Viewport {
    fn default() -> Self {
        Self::FULL
    }
}

impl Camera {
    /// A perspective camera, with every argument corrected into a usable range: a non-positive
    /// FOV or near plane becomes 0.001, and a far plane is pushed to at least `near + 0.1`. Where
    /// it looks is its transform's rotation — see [`Camera::rotation_from`].
    ///
    /// Corrected rather than rejected because these come from scene files and editor fields as
    /// often as from code, and a camera that renders nothing is a harder thing to diagnose than
    /// one that renders slightly wrong.
    pub fn new(
        mut fov: f32,
        mut near: f32,
        mut far: f32,
        primary: bool,
    ) -> Self {
        fov = fov.max(0.001);
        near = near.max(0.001);
        far = far.max(near + 0.1);

        Self {
            fov,
            near,
            far,
            exposure: 1.0, // Varsayılan pozlama 1.0
            primary,
            projection: ProjectionMode::Perspective,
            viewport: None,
            order: 0,
            clear_color: None,
            detail: CameraDetail::Full,
        }
    }

    /// This camera with `color` (linear RGB) as its background — see
    /// [`clear_color`](Self::clear_color).
    pub fn with_clear_color(mut self, color: gizmo_math::Vec3) -> Self {
        self.clear_color = Some(color.max(gizmo_math::Vec3::ZERO));
        self
    }

    /// This camera drawn into `viewport` of the target (see [`Camera::viewport`]).
    pub fn with_viewport(mut self, viewport: Viewport) -> Self {
        self.viewport = Some(viewport);
        self
    }

    /// This camera's draw [`order`](Self::order) among viewport cameras.
    pub fn with_order(mut self, order: i32) -> Self {
        self.order = order;
        self
    }

    /// Toggles between perspective and orthographic projection. When switching to
    /// orthographic, the vertical extent is chosen so the framing roughly matches
    /// the current perspective `fov` at the given `distance` from the camera.
    /// Switches between perspective and orthographic.
    ///
    /// `distance` is how far the subject is: going orthographic, it is what the ortho height is
    /// derived from, so the subject keeps the on-screen size it had under perspective. Without
    /// that the view appears to jump.
    pub fn toggle_projection(&mut self, distance: f32) {
        self.projection = match self.projection {
            ProjectionMode::Perspective => ProjectionMode::Orthographic {
                height: 2.0 * distance.abs().max(0.001) * (self.fov * 0.5).tan(),
            },
            ProjectionMode::Orthographic { .. } | ProjectionMode::OrthographicFit { .. } => {
                ProjectionMode::Perspective
            }
        };
    }

    /// The projection matrix for a viewport of the given aspect ratio (width / height),
    /// perspective or orthographic according to [`Self::projection`].
    pub fn get_projection(&self, aspect: f32) -> gizmo_math::Mat4 {
        match self.projection {
            ProjectionMode::Perspective => {
                gizmo_math::Mat4::perspective_rh(self.fov, aspect, self.near, self.far)
            }
            ProjectionMode::Orthographic { height } => {
                let half_h = (height * 0.5).max(0.001);
                let half_w = half_h * aspect.max(0.001);
                gizmo_math::Mat4::orthographic_rh(
                    -half_w, half_w, -half_h, half_h, self.near, self.far,
                )
            }
            mode @ ProjectionMode::OrthographicFit { .. } => {
                // `view_extent` is `Some` for every orthographic mode.
                let half = mode.view_extent(aspect).unwrap_or(gizmo_math::Vec2::ONE) * 0.5;
                gizmo_math::Mat4::orthographic_rh(
                    -half.x, half.x, -half.y, half.y, self.near, self.far,
                )
            }
        }
    }

    /// The view matrix of a camera at `position` turned by `rotation` — its transform's. An
    /// unrotated camera looks down `−Z` with `+Y` up.
    #[must_use]
    pub fn view(position: Vec3, rotation: Quat) -> gizmo_math::Mat4 {
        let rotation = rotation.normalize();
        gizmo_math::Mat4::look_at_rh(position, position + Self::front(rotation), rotation * Vec3::Y)
    }

    /// The direction a camera turned by `rotation` looks: `rotation · −Z`.
    #[must_use]
    pub fn front(rotation: Quat) -> Vec3 {
        (rotation * Vec3::NEG_Z).normalize()
    }

    /// The right of a camera turned by `rotation`: `rotation · +X` — horizontal when the
    /// rotation came from [`rotation_from`](Self::rotation_from).
    #[must_use]
    pub fn right(rotation: Quat) -> Vec3 {
        (rotation * Vec3::X).normalize()
    }

    /// The rotation that aims a camera at `yaw`/`pitch` with no roll — what a controller that
    /// thinks in those angles writes into the camera's `Transform::rotation`. Its front is
    /// [`forward_from`](Self::forward_from)`(yaw, pitch)` and its right
    /// [`right_from`](Self::right_from)`(yaw)`; pitch is kept just short of vertical.
    #[must_use]
    pub fn rotation_from(yaw: f32, pitch: f32) -> Quat {
        let pitch = pitch.clamp(-std::f32::consts::FRAC_PI_2 + 0.001, std::f32::consts::FRAC_PI_2 - 0.001);
        Quat::from_rotation_y(-yaw - std::f32::consts::FRAC_PI_2) * Quat::from_rotation_x(pitch)
    }

    /// The rotation that aims a camera at `position` toward `target`, `+Y` kept up — `None` when
    /// the two are the same point. Straight up or down keeps the yaw of `−Z`.
    #[must_use]
    pub fn looking_at(position: Vec3, target: Vec3) -> Option<Quat> {
        let (yaw, pitch) = Self::yaw_pitch_from_forward(target - position, -std::f32::consts::FRAC_PI_2)?;
        Some(Self::rotation_from(yaw, pitch))
    }

    /// The yaw and pitch a rotation aims at — [`rotation_from`](Self::rotation_from)'s inverse,
    /// with any roll dropped. Looking straight up or down, the yaw comes from the camera's right.
    #[must_use]
    pub fn yaw_pitch_of(rotation: Quat) -> (f32, f32) {
        let right = Self::right(rotation);
        let fallback = (-right.x).atan2(right.z);
        Self::yaw_pitch_from_forward(Self::front(rotation), fallback).unwrap_or((fallback, 0.0))
    }

    /// The world-space forward (aim) vector from yaw/pitch. [`rotation_from`](Self::rotation_from) AND the first-person
    /// camera controller (`FpsLook`, gizmo-engine::systems) share it, so the "aim direction"
    /// maths is not re-written BY HAND in every demo and game.
    pub fn forward_from(yaw: f32, pitch: f32) -> Vec3 {
        let pitch = pitch.clamp(
            -std::f32::consts::PI / 2.0 + 0.001,
            std::f32::consts::PI / 2.0 - 0.001,
        );
        Vec3::new(
            yaw.cos() * pitch.cos(),
            pitch.sin(),
            yaw.sin() * pitch.cos(),
        )
        .normalize()
    }

    /// The INVERSE of [`forward_from`](Self::forward_from): the yaw/pitch needed to look along a
    /// direction.
    ///
    /// This formula was written out by hand in four separate places in the repository (studio
    /// setup, the fighting camera's look-at, focusing, and the viewport's axis gizmo) — each
    /// under the same two-line "Invert `get_front()`" comment. One of them was written *wrong*:
    /// `x.atan2(-z)` / `(-y).asin()`, which does not invert `get_front` and points the camera
    /// somewhere else entirely. The inverse now lives next to the function it inverts, and the
    /// two have to change together.
    ///
    /// Two degenerate cases, both named instead of silently answered wrongly:
    /// - **A zero or non-finite direction** → `None`. Today that path writes a NaN yaw into the
    ///   camera through `atan2(NaN, NaN)` and kills the view matrix; "nowhere" has no angle.
    /// - **Looking straight up or down** → yaw is undefined (`atan2(0, 0)` returns `0` without
    ///   complaint and swings the scene to world +X), so it is not invented: `fallback_yaw` is
    ///   carried through instead.
    ///
    /// Because `forward_from` keeps pitch 0.001 rad short of vertical, an exactly vertical
    /// direction does not come back bit-identical through a round trip. That is the whole
    /// difference.
    pub fn yaw_pitch_from_forward(dir: Vec3, fallback_yaw: f32) -> Option<(f32, f32)> {
        let d = dir.normalize_or_zero();
        if d == Vec3::ZERO {
            return None;
        }
        let pitch = d.y.clamp(-1.0, 1.0).asin();
        let yaw = if d.x.abs() + d.z.abs() < 1e-4 {
            fallback_yaw
        } else {
            d.z.atan2(d.x)
        };
        Some((yaw, pitch))
    }

    /// The world-space right vector from yaw (horizontal). The closed form of
    /// forward × (0,1,0).
    pub fn right_from(yaw: f32) -> Vec3 {
        Vec3::new(-yaw.sin(), 0.0, yaw.cos())
    }

    /// Build a world-space picking ray from a screen/cursor pixel through this
    /// camera — the engine's screen→world unproject. Combine with
    /// `PhysicsWorld::raycast` (or a plane intersection) to pick / drag the object
    /// under the cursor.
    ///
    /// * `screen` — cursor position in pixels, origin **top-left** (matches
    ///   [`gizmo_core`]'s `Input::mouse_position`).
    /// * `viewport` — framebuffer size in the same pixels (e.g. `WindowInfo`).
    /// * `world_pos`, `rotation` — the camera's world pose (its transform's; see [`Camera`]).
    ///
    /// The heavy lifting (NDC → world via the inverse view-projection, with
    /// singular-matrix / degenerate-direction guards) is [`gizmo_math::Ray::from_ndc`].
    pub fn screen_to_ray(
        &self,
        screen: (f32, f32),
        viewport: (f32, f32),
        world_pos: Vec3,
        rotation: Quat,
    ) -> gizmo_math::Ray {
        let (w, h) = (viewport.0.max(1.0), viewport.1.max(1.0));
        // Pixel → NDC: x∈[-1,1] rightward, y∈[-1,1] UPward (flip the top-left screen y).
        let ndc = gizmo_math::Vec2::new((screen.0 / w) * 2.0 - 1.0, 1.0 - (screen.1 / h) * 2.0);
        let view_proj_inv = (self.get_projection(w / h) * Self::view(world_pos, rotation)).inverse();
        gizmo_math::Ray::from_ndc(ndc, view_proj_inv)
    }

    /// The other way: the pixel a world-space `point` lands on — [`screen_to_ray`](Self::screen_to_ray)'s
    /// inverse, with the same `viewport`, `world_pos` and `rotation` arguments and the same top-left
    /// origin.
    ///
    /// `None` when the point is behind the camera (or on its plane), where no pixel shows it. A
    /// point in front but outside the view gets a pixel outside `0..viewport`, which is what an
    /// off-screen marker wants; test the bounds for "is it on screen".
    pub fn world_to_screen(&self, point: Vec3, viewport: (f32, f32), world_pos: Vec3, rotation: Quat) -> Option<(f32, f32)> {
        let (w, h) = (viewport.0.max(1.0), viewport.1.max(1.0));
        let clip = self.get_projection(w / h) * Self::view(world_pos, rotation) * point.extend(1.0);
        if clip.w <= 1e-6 {
            return None;
        }
        let ndc = clip.truncate() / clip.w;
        Some(((ndc.x + 1.0) * 0.5 * w, (1.0 - ndc.y) * 0.5 * h))
    }
}

/// **A camera that draws into a texture of the game's own** instead of the window — a security
/// monitor, a portal, a rear-view mirror shown in the HUD, a thumbnail. On an entity with a
/// [`Camera`], it is drawn every frame into this texture before the frame's own view, in a
/// submission of its own and with its own temporal history (TAA, SSGI, motion blur), through the
/// game path's whole pipeline; several go in [`order`](Camera::order). It is never the frame's camera
/// ([`active_camera`] passes it over) nor a split-screen view, and it draws no HUD.
///
/// The texture is made in the renderer's output format — the one its last pass writes, so the
/// picture comes out graded and tone-mapped like the window's — and can be sampled (a material, the
/// UI) and copied out. The scene is rendered at the renderer's own size and resampled into it, as
/// a split-screen view is; the camera's projection takes the texture's aspect. Clones share one
/// texture.
///
/// A mirror the camera sees reflects for it: its mirrors are captured for it, just before it is
/// drawn — one more scene draw for each mirror it sees.
#[derive(Clone, Debug)]
pub struct CameraTarget {
    texture: std::sync::Arc<wgpu::Texture>,
    view: std::sync::Arc<wgpu::TextureView>,
}
gizmo_core::impl_component!(CameraTarget, CameraTargetDesc);

/// What a [`CameraTarget`] is but its texture — its size — which a scene saves; the texture is made
/// again from it ([`sync_camera_targets`]), and the camera draws into it as before.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CameraTargetDesc {
    /// Width in texels.
    pub width: u32,
    /// Height in texels.
    pub height: u32,
}

/// Keeps every [`CameraTarget`] and its [`CameraTargetDesc`] together: the description follows the
/// target's size, and a description with no target (a scene just opened) gets a new texture of it.
pub fn sync_camera_targets(world: &mut gizmo_core::World, renderer: &crate::Renderer) {
    use gizmo_core::Entity;
    let (missing, stale): (Vec<(Entity, CameraTargetDesc)>, Vec<(Entity, CameraTargetDesc)>) = {
        let targets = world.query::<&CameraTarget>();
        let descs = world.query::<&CameraTargetDesc>();
        let missing = world
            .query::<(Entity, &CameraTargetDesc)>()
            .iter()
            .filter(|(e, _)| targets.get(*e).is_none())
            .map(|(e, d)| (e, *d))
            .collect();
        let stale = world
            .query::<(Entity, &CameraTarget)>()
            .iter()
            .filter_map(|(e, t)| {
                let (width, height) = t.size();
                let now = CameraTargetDesc { width, height };
                (descs.get(e) != Some(&now)).then_some((e, now))
            })
            .collect();
        (missing, stale)
    };
    if !missing.is_empty() {
        let gpu = renderer.gpu_context();
        for (entity, desc) in missing {
            world.add_component(entity, CameraTarget::new(&gpu, desc.width, desc.height));
        }
    }
    for (entity, desc) in stale {
        world.add_component(entity, desc);
    }
}

impl CameraTarget {
    /// A `width` × `height` target in the renderer's output format, made with the game side's GPU
    /// handle (the world's [`GpuContext`](crate::GpuContext) — `renderer.gpu_context()` in a setup).
    /// A size of zero is one texel.
    #[must_use]
    pub fn new(gpu: &crate::GpuContext, width: u32, height: u32) -> Self {
        let texture = gpu.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("camera_target"),
            size: wgpu::Extent3d { width: width.max(1), height: height.max(1), depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: gpu.output_format(),
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Self { texture: std::sync::Arc::new(texture), view: std::sync::Arc::new(view) }
    }

    /// The texture the camera draws into.
    #[must_use]
    pub fn texture(&self) -> &wgpu::Texture {
        &self.texture
    }

    /// A view of the whole texture — what a material or a pass samples.
    #[must_use]
    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// Width and height in texels.
    #[must_use]
    pub fn size(&self) -> (u32, u32) {
        (self.texture.width(), self.texture.height())
    }
}

/// **Which camera the scene is rendered from.** One function, because every layer needs the answer
/// and three implementations of it would be three answers.
///
/// The rule: the camera flagged [`primary`](Camera::primary) — the convention `spawn_camera` and
/// `CameraBundle` maintain — falling back to the first camera in the world when none is marked, so
/// the result never depends on the (unstable) ECS iteration order. The audio listener follows the
/// same rule, and the render path picks its matrices with it. A camera that draws into a texture
/// ([`CameraTarget`]) is never the answer: it has a picture of its own.
///
/// It lives here rather than in the facade because the editor needs it too and sits below the
/// facade: an inspector panel editing "the scene's" camera has to mean the same camera the frame
/// renders from, or the panel is editing something the player will never look through.
pub fn active_camera(world: &gizmo_core::World) -> Option<gizmo_core::Entity> {
    let cameras = world.query_filtered::<(gizmo_core::Entity, &Camera), gizmo_core::query::Without<CameraTarget>>();
    cameras
        .iter()
        .find(|(_, c)| c.primary)
        .or_else(|| cameras.iter().next())
        .map(|(entity, _)| entity)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_to_ray_center_points_along_camera_front() {
        // Camera at (0,0,10), unrotated: looking down -Z.
        let cam = Camera::new(std::f32::consts::FRAC_PI_2, 0.1, 100.0, true);
        let pos = Vec3::new(0.0, 0.0, 10.0);
        // Centre pixel of a 200x100 viewport → NDC (0,0) → ray along the camera front.
        let ray = cam.screen_to_ray((100.0, 50.0), (200.0, 100.0), pos, Quat::IDENTITY);
        assert!((ray.direction.z - (-1.0)).abs() < 1e-4, "centre ray looks -Z, got {:?}", ray.direction);
        assert!(ray.direction.x.abs() < 1e-4 && ray.direction.y.abs() < 1e-4);
        assert!((ray.direction.length() - 1.0).abs() < 1e-5, "direction normalized");
    }

    #[test]
    fn screen_to_ray_offset_pixels_tilt_the_ray() {
        let cam = Camera::new(std::f32::consts::FRAC_PI_2, 0.1, 100.0, true);
        let pos = Vec3::new(0.0, 0.0, 10.0);
        // Right of centre → ray tilts +X; below centre (larger screen-y) → ray tilts -Y.
        let right = cam.screen_to_ray((150.0, 50.0), (200.0, 100.0), pos, Quat::IDENTITY);
        assert!(right.direction.x > 0.05, "right pixel tilts +X, got {:?}", right.direction);
        let down = cam.screen_to_ray((100.0, 90.0), (200.0, 100.0), pos, Quat::IDENTITY);
        assert!(down.direction.y < -0.05, "lower pixel tilts -Y, got {:?}", down.direction);
    }

    #[test]
    fn new_sanitizes_out_of_range_inputs() {
        // Degenerate fov/near, far ≤ near.
        let cam = Camera::new(-1.0, -1.0, -5.0, true);
        assert!(cam.fov >= 0.001);
        assert!(cam.near >= 0.001);
        assert!(cam.far >= cam.near + 0.1, "far must sit past near, got {}", cam.far);
    }

    /// The rotation a yaw/pitch controller writes aims where `forward_from` says, keeps its right
    /// where `right_from` says (no roll), and reads back as the same angles — so the view a
    /// rotation gives is the view the old yaw/pitch fields gave.
    #[test]
    fn rotation_from_aims_like_the_angles_and_reads_back() {
        for (yaw, pitch) in [(0.0, 0.0), (-std::f32::consts::FRAC_PI_2, 0.0), (0.7, 0.3), (2.9, -1.2), (-2.0, 1.4)] {
            let r = Camera::rotation_from(yaw, pitch);
            let f = Camera::forward_from(yaw, pitch);
            assert!((Camera::front(r) - f).length() < 1e-5, "front at {yaw},{pitch}: {:?} vs {f:?}", Camera::front(r));
            assert!((Camera::right(r) - Camera::right_from(yaw)).length() < 1e-5, "right at {yaw},{pitch}");
            let (y2, p2) = Camera::yaw_pitch_of(r);
            assert!((Camera::forward_from(y2, p2) - f).length() < 1e-4, "round trip at {yaw},{pitch}: {y2},{p2}");
            // The old `get_view`: look_at along forward_from with up = right × front.
            let old = gizmo_math::Mat4::look_at_rh(Vec3::ONE, Vec3::ONE + f, Camera::right_from(yaw).cross(f));
            let new = Camera::view(Vec3::ONE, r);
            assert!(old.abs_diff_eq(new, 1e-5), "view at {yaw},{pitch}");
        }
        assert!(Camera::rotation_from(0.0, 0.0).y.is_finite());
        assert!(Camera::rotation_from(-std::f32::consts::FRAC_PI_2, 0.0).angle_between(Quat::IDENTITY) < 1e-3, "yaw −90° is unrotated");
    }

    /// `looking_at` aims at the target, straight down keeps a usable rotation, and no direction is
    /// `None`.
    #[test]
    fn looking_at_aims_at_the_target() {
        let at = Vec3::new(1.0, 2.0, 3.0);
        for target in [Vec3::ZERO, Vec3::new(5.0, 2.0, 3.0), Vec3::new(1.0, -10.0, 3.0)] {
            let r = Camera::looking_at(at, target).unwrap();
            assert!((Camera::front(r) - (target - at).normalize()).length() < 1e-2, "{target:?}");
            assert!(Camera::right(r).y.abs() < 1e-5, "no roll");
        }
        assert!(Camera::looking_at(at, at).is_none());
    }

    #[test]
    fn forward_and_right_are_orthonormal() {
        let (yaw, pitch) = (0.7f32, 0.3f32);
        let f = Camera::forward_from(yaw, pitch);
        let r = Camera::right_from(yaw);
        assert!((f.length() - 1.0).abs() < 1e-5, "forward not unit: {f:?}");
        assert!((r.length() - 1.0).abs() < 1e-5, "right not unit: {r:?}");
        assert!(r.y.abs() < 1e-6, "right must stay horizontal: {r:?}");
        assert!(f.dot(r).abs() < 1e-5, "right ⟂ forward expected, dot={}", f.dot(r));
    }

    #[test]
    fn forward_with_zero_pitch_is_horizontal() {
        let f = Camera::forward_from(0.0, 0.0);
        assert!(f.y.abs() < 1e-6, "zero pitch → horizontal aim, got {f:?}");
        // Pitch beyond vertical is clamped, so y never reaches ±1.
        let up = Camera::forward_from(0.0, std::f32::consts::PI); // way over vertical
        assert!(up.y.abs() < 1.0);
        assert!((up.length() - 1.0).abs() < 1e-5);
    }

    #[test]
    fn toggle_projection_is_an_involution_matching_the_fov_framing() {
        let mut cam = Camera::new(std::f32::consts::FRAC_PI_2, 0.1, 100.0, true);
        assert!(matches!(cam.projection, ProjectionMode::Perspective));

        cam.toggle_projection(10.0);
        match cam.projection {
            ProjectionMode::Orthographic { height } => {
                let expected = 2.0 * 10.0 * (cam.fov * 0.5).tan();
                assert!((height - expected).abs() < 1e-3, "height {height} vs {expected}");
            }
            _ => panic!("expected orthographic after first toggle"),
        }
        // Toggling again returns to perspective (involution).
        cam.toggle_projection(10.0);
        assert!(matches!(cam.projection, ProjectionMode::Perspective));
    }

    /// The 3-D camera's projection is finite in both modes — and orthographic is the mode a 2-D
    /// view uses, which is why the separate `Camera2D` type could go: it re-implemented this with
    /// a zoom instead of a height, and no draw path ever read it.
    #[test]
    fn projections_are_finite_in_both_modes() {
        let mut cam = Camera::new(std::f32::consts::FRAC_PI_2, 0.1, 100.0, true);
        assert!(cam.get_projection(1.777).to_cols_array().iter().all(|v| v.is_finite()));

        cam.toggle_projection(10.0); // the distance the ortho height is derived from
        let ortho = cam.get_projection(1.777);
        assert!(ortho.to_cols_array().iter().all(|v| v.is_finite()));
        assert!(
            matches!(cam.projection, ProjectionMode::Orthographic { .. }),
            "the toggle must actually reach orthographic"
        );
    }

    /// The fit mode keeps the whole `width × height` box in view in a wide, a square and a tall
    /// window — the tall one is the case plain `Orthographic { height }` cuts the sides off.
    #[test]
    fn orthographic_fit_shows_the_whole_box_in_any_window_shape() {
        let mode = ProjectionMode::OrthographicFit { width: 30.0, height: 16.0 };
        for aspect in [16.0 / 9.0, 1.0, 948.0 / 1028.0, 0.3] {
            let e = mode.view_extent(aspect).unwrap();
            assert!(e.x >= 30.0 - 1e-3 && e.y >= 16.0 - 1e-3, "aspect {aspect}: {e:?}");
            assert!((e.x / e.y - aspect).abs() < 1e-4, "keeps the window's shape");
            // Tight on one axis: it does not show more than it has to.
            assert!((e.x - 30.0).abs() < 1e-3 || (e.y - 16.0).abs() < 1e-3, "{e:?}");
        }
        // The projection agrees with the extent: the box corner lands on the NDC edge or inside.
        let mut cam = Camera::new(1.0, 0.1, 100.0, true);
        cam.projection = mode;
        let tall = 948.0 / 1028.0;
        let p = cam.get_projection(tall) * gizmo_math::Vec4::new(15.0, 8.0, -1.0, 1.0);
        assert!((p.x - 1.0).abs() < 1e-4, "the box's side is the window's side, got {}", p.x);
        assert!(p.y.abs() < 1.0);
        // Plain Orthographic in the same window loses it — the measured kule_yikici failure.
        let plain = ProjectionMode::Orthographic { height: 16.0 }.view_extent(tall).unwrap();
        assert!(plain.x < 30.0);
        assert_eq!(ProjectionMode::Perspective.view_extent(1.0), None);
        assert!(mode.is_orthographic() && !ProjectionMode::Perspective.is_orthographic());
    }

    /// The fit mode toggles back to perspective like the plain one, and survives a scene round trip.
    #[test]
    fn orthographic_fit_toggles_and_serializes() {
        let mut cam = Camera::new(1.0, 0.1, 100.0, true);
        cam.projection = ProjectionMode::OrthographicFit { width: 4.0, height: 3.0 };
        let text = ron::to_string(&cam).unwrap();
        assert_eq!(ron::from_str::<Camera>(&text).unwrap(), cam);
        cam.toggle_projection(5.0);
        assert_eq!(cam.projection, ProjectionMode::Perspective);
    }
}
