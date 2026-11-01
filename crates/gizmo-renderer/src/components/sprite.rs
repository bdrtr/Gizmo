use gizmo_math::{Vec2, Vec4};

use crate::sprite::SpriteImageId;

/// A textured quad the engine draws — in the world on the entity's plane, or on the screen.
///
/// The engine had no 2D path at all until this existed: no sprite, no 2D layering, nothing that
/// could put an image on the screen except an `egui` panel or a textured 3D cube.
/// `docs/CAPABILITY_GAPS.md` §A1.
///
/// The image comes from [`Renderer::load_sprite_image`](crate::Renderer::load_sprite_image) (or
/// its `_file` / `_rgba` siblings), and an id that was never handed out draws nothing rather than
/// a placeholder — the same rule [`Text`](super::Text) keeps for fonts, for the same reason.
///
/// # Where it is drawn
///
/// - [`SpriteSpace::World`]: a quad in the entity's **local XY plane**, `size` world units across,
///   placed by its `GlobalTransform` — so it moves, rotates and scales with the entity, and a 2D
///   game is an orthographic camera looking down `-Z` at a field of these. It does **not** turn to
///   face the camera (that is what a world-space [`Text`](super::Text) does); a sprite seen edge-on
///   is a line, which is what a card in a 3D scene should look like.
/// - [`SpriteSpace::Screen`]: window pixels, top-left origin, over the picture — a HUD.
///
/// # Order
///
/// Within each space, lower [`layer`](Self::layer) is drawn first. World sprites on the same layer
/// are then painted back to front by distance from the camera; screen sprites on the same layer in
/// entity order. World sprites test against the scene's depth and write none: a wall hides a
/// sprite behind it, and sprites never hide each other except by being painted over.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Sprite {
    /// Which loaded image to draw.
    pub image: SpriteImageId,
    /// The quad's size: world units in [`SpriteSpace::World`], pixels in [`SpriteSpace::Screen`].
    pub size: Vec2,
    /// Multiplied into the image. `a` scales its opacity.
    pub color: Vec4,
    /// The part of the image to draw, as `(u0, v0, u1, v1)` with `(0, 0)` the image's top-left.
    ///
    /// The whole image by default; a sub-rectangle is a frame of a sprite sheet —
    /// [`with_frame`](Self::with_frame) computes one for a regular grid.
    pub uv_rect: [f32; 4],
    /// Mirror the image left-to-right.
    pub flip_x: bool,
    /// Mirror the image top-to-bottom.
    pub flip_y: bool,
    /// Which point of the quad sits on the entity's position, as a fraction of its size with
    /// `(0, 0)` the **top-left** and `(1, 1)` the bottom-right — the convention
    /// [`TextAnchor::factors`](super::TextAnchor::factors) uses. `(0.5, 0.5)`, the centre, by
    /// default; `(0.5, 1.0)` stands a character on its feet.
    pub anchor: Vec2,
    /// Draw order within the space: lower first.
    pub layer: i32,
    /// On the entity's plane in the world, or in window pixels.
    pub space: SpriteSpace,
}

impl Sprite {
    /// A white-tinted world sprite `size` world units across, centred on the entity.
    #[must_use]
    pub fn world(image: SpriteImageId, size: Vec2) -> Self {
        Self {
            image,
            size,
            color: Vec4::ONE,
            uv_rect: [0.0, 0.0, 1.0, 1.0],
            flip_x: false,
            flip_y: false,
            anchor: Vec2::splat(0.5),
            layer: 0,
            space: SpriteSpace::World,
        }
    }

    /// A screen sprite `size` pixels across with its **top-left** at `position` (window pixels,
    /// origin top-left).
    ///
    /// Top-left rather than centred because that is what a HUD is laid out in, and what
    /// `gizmo-ui`'s `Node` publishes.
    #[must_use]
    pub fn screen(image: SpriteImageId, size: Vec2, position: Vec2) -> Self {
        Self {
            anchor: Vec2::ZERO,
            space: SpriteSpace::Screen { position },
            ..Self::world(image, size)
        }
    }

    /// Replaces the tint.
    #[must_use]
    pub fn with_color(mut self, color: Vec4) -> Self {
        self.color = color;
        self
    }

    /// Replaces the anchor — see [`anchor`](Self::anchor).
    #[must_use]
    pub fn with_anchor(mut self, anchor: Vec2) -> Self {
        self.anchor = anchor;
        self
    }

    /// Replaces the layer.
    #[must_use]
    pub fn with_layer(mut self, layer: i32) -> Self {
        self.layer = layer;
        self
    }

    /// Replaces the mirroring.
    #[must_use]
    pub fn with_flip(mut self, flip_x: bool, flip_y: bool) -> Self {
        self.flip_x = flip_x;
        self.flip_y = flip_y;
        self
    }

    /// Replaces the image sub-rectangle — see [`uv_rect`](Self::uv_rect).
    #[must_use]
    pub fn with_uv_rect(mut self, uv_rect: [f32; 4]) -> Self {
        self.uv_rect = uv_rect;
        self
    }

    /// Shows frame `index` of a sheet cut into `columns × rows` equal cells, counted left to
    /// right and then top to bottom.
    ///
    /// An index past the last cell wraps, so `frame = tick % count` and `frame = tick` animate the
    /// same way. A sheet with zero columns or rows has no frames and leaves the rectangle alone.
    #[must_use]
    pub fn with_frame(mut self, index: u32, columns: u32, rows: u32) -> Self {
        if let Some(rect) = sheet_frame(index, columns, rows) {
            self.uv_rect = rect;
        }
        self
    }

    /// Changes the frame in place — what an animation system calls every tick. See
    /// [`with_frame`](Self::with_frame).
    pub fn set_frame(&mut self, index: u32, columns: u32, rows: u32) {
        if let Some(rect) = sheet_frame(index, columns, rows) {
            self.uv_rect = rect;
        }
    }
}

/// Cell `index` of a `columns × rows` grid, as a UV rectangle. `None` for an empty grid.
fn sheet_frame(index: u32, columns: u32, rows: u32) -> Option<[f32; 4]> {
    let count = columns.checked_mul(rows).filter(|&n| n > 0)?;
    let i = index % count;
    let (col, row) = (i % columns, i / columns);
    let (w, h) = (1.0 / columns as f32, 1.0 / rows as f32);
    Some([col as f32 * w, row as f32 * h, (col + 1) as f32 * w, (row + 1) as f32 * h])
}

/// Plays a run of frames from a sprite sheet on the entity's [`Sprite`] — a walk cycle, a coin
/// spinning, an explosion that plays once.
///
/// The sheet is cut into `columns × rows` equal cells counted left to right, then top to bottom
/// (the rule [`Sprite::with_frame`] uses), and the animation shows cells `first..=last` at `fps`
/// frames per second. The engine advances it every frame on both draw paths — the game's
/// `default_render_pass` and the editor's viewport — by the clock's delta, so `set_time_scale`
/// and pause apply to it as they do to skeletal animation. Nothing has to be registered.
///
/// Until this existed a game animated a sprite by computing `frame = (t * fps) as u32` and
/// calling [`Sprite::set_frame`] itself every frame (`docs/CAPABILITY_GAPS.md` §C9).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct SpriteAnimation {
    /// Cells across the sheet.
    pub columns: u32,
    /// Cells down the sheet.
    pub rows: u32,
    /// The first cell shown.
    pub first: u32,
    /// The last cell shown, inclusive. A `last` before `first` plays the run backwards.
    pub last: u32,
    /// Frames per second. Zero or negative holds the current frame.
    pub fps: f32,
    /// Wrap around at the end, or stop on the last frame.
    pub looping: bool,
    /// Whether time advances it. A paused animation keeps showing its current frame.
    pub playing: bool,
    /// Seconds into the run. Kept below one loop's length while looping, so a long-running
    /// animation does not lose precision.
    elapsed: f32,
}

impl SpriteAnimation {
    /// A looping, playing animation over cells `first..=last` of a `columns × rows` sheet.
    #[must_use]
    pub fn new(columns: u32, rows: u32, first: u32, last: u32, fps: f32) -> Self {
        Self { columns, rows, first, last, fps, looping: true, playing: true, elapsed: 0.0 }
    }

    /// Plays once and stops on the last frame — see [`finished`](Self::finished).
    #[must_use]
    pub fn once(mut self) -> Self {
        self.looping = false;
        self
    }

    /// How many frames the run has.
    #[must_use]
    pub fn len(&self) -> u32 {
        self.first.abs_diff(self.last) + 1
    }

    /// Never true — a run has at least one frame. Here because `len` is.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        false
    }

    /// The sheet cell showing now.
    #[must_use]
    pub fn frame(&self) -> u32 {
        let step = self.step();
        if self.first <= self.last { self.first + step } else { self.first - step }
    }

    /// Which frame of the run (0-based) is showing.
    fn step(&self) -> u32 {
        if self.fps <= 0.0 || !self.fps.is_finite() {
            return 0;
        }
        let n = (self.elapsed * self.fps).floor().max(0.0) as u32;
        let len = self.len();
        if self.looping { n % len } else { n.min(len - 1) }
    }

    /// Whether a one-shot animation has reached its last frame. Always `false` while looping.
    #[must_use]
    pub fn finished(&self) -> bool {
        !self.looping && self.fps > 0.0 && self.elapsed * self.fps >= self.len() as f32 - 1.0
    }

    /// Switches to cells `first..=last` and starts them from the beginning — **unless** that is
    /// already the run, in which case nothing changes.
    ///
    /// That exception is the point: a game picks the run from its state every frame ("walking",
    /// "in the air", "standing"), and restarting a walk cycle sixty times a second would freeze it
    /// on its first frame. A single-frame run (`first == last`) is how a still pose is shown.
    pub fn set_run(&mut self, first: u32, last: u32) {
        if (self.first, self.last) != (first, last) {
            self.first = first;
            self.last = last;
            self.elapsed = 0.0;
        }
    }

    /// Back to the first frame, keeping `playing` as it is.
    pub fn restart(&mut self) {
        self.elapsed = 0.0;
    }

    /// Moves the animation `dt` seconds on, if it is playing. Negative and non-finite `dt` are
    /// ignored.
    pub fn advance(&mut self, dt: f32) {
        if !self.playing || !(dt.is_finite() && dt > 0.0) || !(self.fps.is_finite() && self.fps > 0.0) {
            return;
        }
        self.elapsed += dt;
        let period = self.len() as f32 / self.fps;
        if self.looping {
            self.elapsed %= period;
        } else {
            // Past the end nothing changes; stop the clock so it cannot grow without bound.
            self.elapsed = self.elapsed.min(period);
        }
    }
}

/// Advances every [`SpriteAnimation`] by `dt` and writes its frame onto the entity's [`Sprite`].
///
/// Both draw paths call this once a frame with the clock's delta, beside the skeletal animation
/// systems; a game does not call it. A `SpriteAnimation` on an entity with no `Sprite` still
/// advances and simply has nothing to show.
pub fn sprite_animation_system(world: &mut gizmo_core::World, dt: f32) {
    // Two passes: an animation advances whether or not its entity has a sprite, so it cannot be
    // a joint `(SpriteAnimation, Sprite)` query.
    let frames: Vec<(u32, [f32; 4])> = match world.query_mut::<gizmo_core::query::Mut<SpriteAnimation>>() {
        Some(mut q) => q
            .iter_mut()
            .filter_map(|(id, mut anim)| {
                anim.advance(dt);
                sheet_frame(anim.frame(), anim.columns, anim.rows).map(|rect| (id, rect))
            })
            .collect(),
        None => return,
    };
    let mut sprites = world.borrow_mut::<Sprite>();
    for (id, rect) in frames {
        if let Some(mut sprite) = sprites.get_mut(id) {
            if sprite.uv_rect != rect {
                sprite.uv_rect = rect;
            }
        }
    }
}

/// Where a [`Sprite`] is drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum SpriteSpace {
    /// In the entity's local XY plane, placed by its `GlobalTransform`.
    World,
    /// Window pixels, origin at the **top-left**.
    Screen {
        /// Where the anchor point goes, in window pixels.
        position: Vec2,
    },
}

impl SpriteSpace {
    /// Whether this space needs the entity's world transform.
    ///
    /// Here so a draw loop outside this crate does not have to `match` a `#[non_exhaustive]` enum
    /// with a wildcard — the same reasoning as
    /// [`TextSpace::needs_world_position`](super::TextSpace::needs_world_position).
    #[must_use]
    pub fn needs_world_transform(self) -> bool {
        match self {
            Self::World => true,
            Self::Screen { .. } => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_count_left_to_right_then_down_and_wrap() {
        // 4 × 2: frame 5 is column 1 of row 1.
        assert_eq!(sheet_frame(5, 4, 2), Some([0.25, 0.5, 0.5, 1.0]));
        assert_eq!(sheet_frame(0, 4, 2), Some([0.0, 0.0, 0.25, 0.5]));
        assert_eq!(sheet_frame(8, 4, 2), sheet_frame(0, 4, 2), "past the last cell wraps");
    }

    #[test]
    fn an_empty_sheet_leaves_the_rectangle_alone() {
        let s = Sprite::world(SpriteImageId(0), Vec2::ONE).with_uv_rect([0.1, 0.2, 0.3, 0.4]);
        assert_eq!(s.clone().with_frame(3, 0, 4).uv_rect, [0.1, 0.2, 0.3, 0.4]);
        assert_eq!(s.with_frame(3, 4, 0).uv_rect, [0.1, 0.2, 0.3, 0.4]);
        assert_eq!(sheet_frame(0, u32::MAX, u32::MAX), None, "an overflowing grid is not a grid");
    }

    #[test]
    fn screen_sprites_default_to_their_top_left_and_world_ones_to_their_centre() {
        let s = Sprite::screen(SpriteImageId(0), Vec2::ONE, Vec2::new(3.0, 4.0));
        assert_eq!(s.anchor, Vec2::ZERO);
        assert!(!s.space.needs_world_transform());
        let w = Sprite::world(SpriteImageId(0), Vec2::ONE);
        assert_eq!(w.anchor, Vec2::splat(0.5));
        assert!(w.space.needs_world_transform());
    }

    #[test]
    fn an_animation_steps_through_its_run_at_its_rate_and_loops() {
        let mut a = SpriteAnimation::new(4, 2, 2, 5, 10.0);
        assert_eq!(a.frame(), 2);
        a.advance(0.25); // 2.5 frames in
        assert_eq!(a.frame(), 4);
        a.advance(0.2); // 4.5 → wraps to step 0
        assert_eq!(a.frame(), 2);
        assert!(!a.finished());
        // A long run keeps its clock inside one loop.
        for _ in 0..100_000 {
            a.advance(1.0 / 60.0);
        }
        assert!(a.elapsed < a.len() as f32 / a.fps + 1e-6);
    }

    #[test]
    fn a_one_shot_stops_on_its_last_frame_and_says_so() {
        let mut a = SpriteAnimation::new(8, 1, 0, 3, 4.0).once();
        a.advance(0.5);
        assert_eq!((a.frame(), a.finished()), (2, false));
        a.advance(10.0);
        assert_eq!((a.frame(), a.finished()), (3, true));
        a.restart();
        assert_eq!((a.frame(), a.finished()), (0, false));
    }

    #[test]
    fn backwards_runs_paused_runs_and_bad_input() {
        let mut back = SpriteAnimation::new(4, 1, 3, 0, 1.0);
        back.advance(1.5);
        assert_eq!(back.frame(), 2);

        let mut paused = SpriteAnimation::new(4, 1, 0, 3, 1.0);
        paused.playing = false;
        paused.advance(2.0);
        assert_eq!(paused.frame(), 0);

        let mut a = SpriteAnimation::new(4, 1, 0, 3, 1.0);
        a.advance(f32::NAN);
        a.advance(-1.0);
        assert_eq!(a.frame(), 0);
        a.fps = 0.0;
        a.advance(5.0);
        assert_eq!(a.frame(), 0, "zero fps holds the frame");
    }

    /// Choosing the same run every frame keeps it playing; choosing another restarts it.
    #[test]
    fn set_run_restarts_only_on_a_change() {
        let mut a = SpriteAnimation::new(4, 1, 0, 3, 10.0);
        for _ in 0..3 {
            a.set_run(0, 3);
            a.advance(0.1);
        }
        assert_eq!(a.frame(), 3, "the walk cycle kept walking");
        a.set_run(3, 3);
        assert_eq!(a.frame(), 3);
        a.advance(1.0);
        assert_eq!(a.frame(), 3, "a one-cell run is a still pose");
        a.set_run(0, 3);
        assert_eq!(a.frame(), 0, "a new run starts at its beginning");
    }

    /// The system writes the animation's frame onto the sprite, and a sprite without an
    /// animation is left alone.
    #[test]
    fn the_system_writes_the_frame_onto_the_sprite() {
        let mut world = gizmo_core::World::new();
        let animated = world.spawn();
        world.add_component(animated, Sprite::world(SpriteImageId(0), Vec2::ONE));
        world.add_component(animated, SpriteAnimation::new(4, 2, 0, 7, 10.0));
        let still = world.spawn();
        world.add_component(still, Sprite::world(SpriteImageId(0), Vec2::ONE).with_uv_rect([0.1, 0.2, 0.3, 0.4]));

        sprite_animation_system(&mut world, 0.55); // step 5 → column 1 of row 1
        let sprites = world.borrow::<Sprite>();
        assert_eq!(sprites.get(animated.id()).unwrap().uv_rect, [0.25, 0.5, 0.5, 1.0]);
        assert_eq!(sprites.get(still.id()).unwrap().uv_rect, [0.1, 0.2, 0.3, 0.4]);
    }
}
