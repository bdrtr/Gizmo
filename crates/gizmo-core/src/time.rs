//! Frame timing: [`Time`], the variable render clock, and [`PhysicsTime`], the fixed-step
//! accumulator that drives the simulation.
//!
//! Neither type reads the system clock. Both are *fed* a delta measured by the caller
//! ([`Time::update`], [`PhysicsTime::accumulate`]), which is what makes a recorded or
//! replayed frame sequence reproduce the same stepping as the original run.
//!
//! The two types do not know about each other either: `accumulate` banks whatever delta it
//! is handed, and whether that is [`Time::dt`] — already multiplied by `time_scale` and
//! capped at `max_dt` — or an independently measured one is the caller's choice, not
//! something decided here.

/// Engine-wide time management.
///
/// # Usage
/// ```
/// use gizmo_core::time::Time;
///
/// let mut time = Time::new();
///
/// // At the start of every frame:
/// time.update(1.0 / 60.0);
///
/// assert_eq!(time.frame(), 1);
/// assert!((time.dt() - 1.0 / 60.0).abs() < 1e-6);
/// assert!((time.raw_dt() - 1.0 / 60.0).abs() < 1e-6);
///
/// // The time scale scales dt and does NOT affect raw_dt.
/// time.set_time_scale(0.5); // slow motion
/// time.update(1.0 / 60.0);
/// assert!((time.dt() - 0.5 / 60.0).abs() < 1e-6);
/// assert!((time.raw_dt() - 1.0 / 60.0).abs() < 1e-6);
///
/// time.set_time_scale(0.0); // pause: dt goes to zero, the frame counter keeps running
/// time.update(1.0 / 60.0);
/// assert_eq!(time.dt(), 0.0);
/// assert_eq!(time.frame(), 3);
///
/// // A long stall (spike) is clamped — so physics does not explode in a single frame.
/// time.set_time_scale(1.0);
/// time.update(5.0);
/// assert!(time.dt() <= 0.05 + 1e-6, "dt max_dt'ye clamp'lenmeli");
/// assert!((time.raw_dt() - 5.0).abs() < 1e-6, "raw_dt clamp'lenmez");
/// ```
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct Time {
    /// Clamped delta time (seconds). `time_scale` applied.
    dt: f32,
    /// Raw delta time — clamp and scale not applied.
    raw_dt: f32,
    /// Total elapsed time (seconds, at f64 precision).
    elapsed: f64,
    /// Frame counter.
    frame_count: u64,
    /// Time scale. 1.0 = normal, 0.5 = slow motion, 0.0 = pause.
    time_scale: f32,
    /// Maximum dt cap (seconds). Default: 1/20 = 50ms.
    max_dt: f32,
}

/// Default max dt: 50ms (20 FPS minimum).
const DEFAULT_MAX_DT: f32 = 1.0 / 20.0;

impl Time {
    /// Creates a clock sitting at frame 0, with `time_scale = 1.0` and the default 50 ms
    /// `max_dt` cap (equivalently, a 20 FPS floor on the simulated step).
    ///
    /// `dt`, `raw_dt` and `elapsed` all start at exactly zero. That is a deliberate value,
    /// not a placeholder: a system that runs before the first [`Time::update`] sees *no*
    /// time passing rather than a guessed frame duration, so it advances nothing instead of
    /// integrating a fabricated step.
    pub fn new() -> Self {
        Self {
            dt: 0.0,
            raw_dt: 0.0,
            elapsed: 0.0,
            frame_count: 0,
            time_scale: 1.0,
            max_dt: DEFAULT_MAX_DT,
        }
    }

    /// Takes the raw dt, applies clamp + scale and updates all time values.
    /// Must be called once at the start of every frame.
    pub fn update(&mut self, raw_dt: f32) {
        self.raw_dt = raw_dt.max(0.0); // Negatif dt'yi engelle
        self.dt = (self.raw_dt * self.time_scale).min(self.max_dt);
        self.elapsed += self.dt as f64;
        self.frame_count += 1;
    }

    // ──── Getter'lar ────

    /// Clamped and scaled delta time (seconds).
    /// Systems such as physics, movement and animation should use this.
    #[inline]
    pub fn dt(&self) -> f32 {
        self.dt
    }

    /// Raw delta time — clamp and scale not applied.
    /// For systems that need real wall-clock time (e.g. the FPS counter).
    #[inline]
    pub fn raw_dt(&self) -> f32 {
        self.raw_dt
    }

    /// Total elapsed time (seconds, at f64 precision).
    /// Retains its precision even in long sessions.
    #[inline]
    pub fn elapsed(&self) -> f64 {
        self.elapsed
    }

    /// Total frame count.
    #[inline]
    pub fn frame(&self) -> u64 {
        self.frame_count
    }

    /// Current time scale.
    #[inline]
    pub fn time_scale(&self) -> f32 {
        self.time_scale
    }

    /// Current FPS (1/raw_dt). Returns 0.0 if raw_dt = 0.
    #[inline]
    pub fn fps(&self) -> f32 {
        if self.raw_dt > 0.0 {
            1.0 / self.raw_dt
        } else {
            0.0
        }
    }

    // ──── Setter'lar ────

    /// Sets the time scale. 0.0 = stop, 0.5 = slow motion, 1.0 = normal, 2.0 = fast.
    pub fn set_time_scale(&mut self, scale: f32) {
        self.time_scale = scale.max(0.0);
    }

    /// Sets the maximum dt cap (seconds).
    pub fn set_max_dt(&mut self, max: f32) {
        self.max_dt = max.max(0.001); // En az ~1ms
    }
}

impl Default for Time {
    fn default() -> Self {
        Self::new()
    }
}

// ═══════════════════════════════════════════════════════════════════════
//  PhysicsTime — Sabit zaman adımlı fizik zamanlayıcı
//
//  Fizik motoru sabit dt'de çalışır (varsayılan 1/60s = 16.67ms).
//  Render frame'leri değişken hızda çalışırken, fizik her zaman aynı
//  dt ile güncellenir → determinizm + kararlılık.
//
//  Kullanım:
//    Frame başında `accumulate(render_dt)` çağrılır.
//    `should_step()` true döndüğü sürece fizik adımları çalıştırılır.
//    `consume_step()` ile accumulator azaltılır.
//    `alpha()` render interpolasyonu için hazır bekler — motor onu HARCAMIYOR, bkz. `alpha()`.
// ═══════════════════════════════════════════════════════════════════════
/// Fixed-timestep accumulator for the physics clock.
///
/// The render loop runs at whatever rate the machine manages; the solver must not. Per frame
/// the caller adds the elapsed render time with [`PhysicsTime::accumulate`], runs one physics
/// step for each [`PhysicsTime::should_step`] / [`PhysicsTime::consume_step`] pair, then calls
/// [`PhysicsTime::compute_alpha`] once before rendering:
///
/// ```text
/// accumulate(dt); while should_step() { step_physics(fixed_dt()); consume_step(); } compute_alpha();
/// ```
///
/// Every step advances the simulation by exactly [`PhysicsTime::fixed_dt`] seconds no matter
/// what the frame rate did, which is the property the determinism and stability guarantees
/// rest on. Leftover time below one step stays in the accumulator and surfaces as
/// [`PhysicsTime::alpha`], which is **offered for** render interpolation rather than spent on
/// it — see that method for what the engine does and does not do with it.
///
/// The accumulator is capped at 8 x `fixed_dt`, so however long a stall lasts (a breakpoint,
/// a swap-in, a stuttering frame) at most 8 steps are ever queued. Time past the cap is
/// **dropped**, not deferred: the simulation falls behind wall-clock rather than entering a
/// spiral of death where each catch-up frame costs more than it recovers.
///
/// **Which ceiling actually governs depends on who feeds this, and on the windowed path it is not
/// this one.** Measured 2026-08-19:
///
/// - The **windowed** runtime hands in [`Time::dt`], which is clamped to `max_dt` (0.05 s by
///   default) *after* `time_scale` — so `set_time_scale(100.0)` cannot push past it either. At the
///   default 60 Hz that is **2 steps** from a clamped frame and 3 with a leftover already banked,
///   never 8: the cap sits at 0.1333 s and nothing can hand in more than 0.0667 s counting the
///   leftover. (Two, not the three the arithmetic suggests, because `3 x fixed_dt` is 0.050000004
///   in f32 — a hair over the clamp.) The 8 is dead code on that path, and the number a reader
///   should reason about is `Time::max_dt`.
/// - The **headless** runtime passes the raw wall-clock delta straight in (it never inserts a
///   `Time` at all). There this cap is the only one there is, it does fire, and the time past it
///   is genuinely gone — which is what the `warn!` in [`PhysicsTime::accumulate`] reports.
/// - Above ~140 Hz the cap shrinks below the windowed clamp (`8/hz < 0.05`) and starts firing
///   there too: at 240 Hz a stalled 0.05 s frame wants 12 steps and gets 8.
///
/// The three ceilings in this engine are easy to confuse and only one of them is doing anything at
/// a time: this one, `gizmo::systems::PlayLoop::MAX_STEPS` (16, and likewise unreachable from its
/// two drivers — they hand it the same clamped delta), and `PhysicsWorld`'s `MAX_SUBSTEPS` (64,
/// which bounds work per frame but not debt, since that accumulator is never clamped).
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct PhysicsTime {
    /// Fixed physics timestep (seconds). Default: 1/60.
    fixed_dt: f32,
    /// Accumulated time — time not yet spent as a physics step.
    accumulator: f32,
    /// Maximum accumulation limit (spiral of death protection).
    max_accumulator: f32,
    /// Total physics step count.
    step_count: u64,
    /// Total physics time (at f64 precision).
    physics_elapsed: f64,
    /// Interpolation coefficient: between 0.0..1.0.
    /// Used during rendering for `lerp(prev_state, curr_state, alpha)`.
    alpha: f32,
}

impl PhysicsTime {
    /// Creates a new PhysicsTime. `hz` = physics update rate (e.g. 60, 120, 240).
    pub fn new(hz: u32) -> Self {
        let fixed_dt = 1.0 / hz as f32;
        Self {
            fixed_dt,
            accumulator: 0.0,
            max_accumulator: fixed_dt * 8.0, // En fazla 8 fizik adımı birikebilir
            step_count: 0,
            physics_elapsed: 0.0,
            alpha: 0.0,
        }
    }

    /// Adds the render frame dt to the accumulator.
    /// Called once at the start of every frame.
    ///
    /// Time past `max_accumulator` is dropped, and **says so**: it used to vanish in silence, so a
    /// headless server ticking slower than the cap (the one path that reaches it — see the type's
    /// docs) fell behind wall-clock with nothing anywhere to read. The log fires only on the branch
    /// that loses time, so a frame that fits costs nothing.
    pub fn accumulate(&mut self, render_dt: f32) {
        self.accumulator += render_dt;
        // Spiral of death koruması
        if self.accumulator > self.max_accumulator {
            tracing::warn!(
                dropped_s = self.accumulator - self.max_accumulator,
                max_accumulator_s = self.max_accumulator,
                fixed_dt_s = self.fixed_dt,
                "[PhysicsTime] accumulator hit its cap — simulated time dropped, the simulation is                  now behind wall-clock"
            );
            self.accumulator = self.max_accumulator;
        }
    }

    /// Has enough time accumulated for one physics step?
    #[inline]
    pub fn should_step(&self) -> bool {
        self.accumulator >= self.fixed_dt
    }

    /// "Consumes" one physics step — subtracts fixed_dt from the accumulator.
    /// Called after every physics step.
    pub fn consume_step(&mut self) {
        self.accumulator -= self.fixed_dt;
        self.step_count += 1;
        self.physics_elapsed += self.fixed_dt as f64;
    }

    /// Computes the interpolation alpha — `accumulator / fixed_dt`, the fraction of a step that
    /// has accumulated but not been simulated.
    ///
    /// Called after all physics steps are finished, before rendering.
    /// `gizmo_app::frame::run_fixed_and_update` does this for a windowed game; a hand-written
    /// loop does it itself. See [`PhysicsTime::alpha`] for who reads the result — measured,
    /// nothing in this engine does.
    pub fn compute_alpha(&mut self) {
        self.alpha = self.accumulator / self.fixed_dt;
    }

    // ──── Getter'lar ────

    /// Fixed physics dt (seconds).
    #[inline]
    pub fn fixed_dt(&self) -> f32 {
        self.fixed_dt
    }

    /// Interpolation coefficient (0.0 .. 1.0): `render_pos = lerp(prev_pos, curr_pos, alpha)`.
    ///
    /// **The engine does not do that lerp, and cannot.** Nothing in this workspace keeps a
    /// previous `Transform` to interpolate against — the renderer draws the pose the last fixed
    /// step left behind, unchanged, for however many frames pass before the next one. So `prev`
    /// in that formula is the embedding game's to keep: snapshot the poses you draw at each
    /// fixed step, and this is the coefficient to blend them with.
    ///
    /// **What that costs, measured** (see this module's tests): at 60 Hz physics on a 60 Hz
    /// display nothing at all — every pose is drawn exactly once and this stays `0.0`. At 144 Hz
    /// each pose is held for two or three frames in a 3,2,3,2,3,2,2… pattern, so a body moving at
    /// a constant speed appears to advance by different amounts on neighbouring frames. And
    /// `alpha * fixed_dt` is how stale the drawn pose is: up to a full 16.7 ms at 60 Hz, which
    /// for something moving at 10 m/s is 16.7 cm, sawtoothing sixty times a second.
    ///
    /// Not to be confused with `PhysicsWorld::render_alpha` in `gizmo-physics-rigid`, which is
    /// the residue of that crate's **own** 240 Hz sub-step accumulator. Use this one when the
    /// fixed step is driven from here (the `App`/`PlayLoop` path, where consecutive poses are
    /// `fixed_dt` apart); use that one when you drive `PhysicsWorld::step` directly with a
    /// variable delta.
    #[inline]
    pub fn alpha(&self) -> f32 {
        self.alpha
    }

    /// Total physics step count.
    #[inline]
    pub fn step_count(&self) -> u64 {
        self.step_count
    }

    /// Total physics time (at f64 precision).
    #[inline]
    pub fn physics_elapsed(&self) -> f64 {
        self.physics_elapsed
    }

    /// Accumulated time (for debugging purposes).
    #[inline]
    pub fn accumulator(&self) -> f32 {
        self.accumulator
    }

    // ──── Setter'lar ────

    /// Changes the physics rate (Hz). Caution: accumulated time is not reset.
    pub fn set_hz(&mut self, hz: u32) {
        self.fixed_dt = 1.0 / hz.max(1) as f32;
        self.max_accumulator = self.fixed_dt * 8.0;
    }
}

impl Default for PhysicsTime {
    fn default() -> Self {
        Self::new(60) // 60 Hz fizik
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_update() {
        let mut time = Time::new();
        time.update(0.016);

        assert!((time.dt() - 0.016).abs() < 0.0001);
        assert!((time.raw_dt() - 0.016).abs() < 0.0001);
        assert!((time.elapsed() - 0.016).abs() < 0.001);
        assert_eq!(time.frame(), 1);
    }

    #[test]
    fn test_dt_clamp() {
        let mut time = Time::new();
        time.update(1.0); // 1 saniye spike

        assert!(time.dt() <= DEFAULT_MAX_DT + 0.0001);
        assert!((time.raw_dt() - 1.0).abs() < 0.0001);
    }

    #[test]
    fn test_negative_dt_clamped_to_zero() {
        let mut time = Time::new();
        time.update(-0.5);

        assert_eq!(time.dt(), 0.0);
        assert_eq!(time.raw_dt(), 0.0);
    }

    #[test]
    fn test_time_scale() {
        let mut time = Time::new();
        time.set_time_scale(0.5);
        time.update(0.016);

        assert!((time.dt() - 0.008).abs() < 0.0001); // 0.016 * 0.5
        assert!((time.raw_dt() - 0.016).abs() < 0.0001);
    }

    #[test]
    fn test_time_scale_zero_is_pause() {
        let mut time = Time::new();
        time.set_time_scale(0.0);
        time.update(0.016);

        assert_eq!(time.dt(), 0.0);
        assert_eq!(time.elapsed(), 0.0);
        assert_eq!(time.frame(), 1); // Frame hâlâ sayılır
    }

    #[test]
    fn test_elapsed_accumulates() {
        let mut time = Time::new();
        for _ in 0..100 {
            time.update(0.01);
        }

        assert!((time.elapsed() - 1.0).abs() < 0.01);
        assert_eq!(time.frame(), 100);
    }

    #[test]
    fn test_fps() {
        let mut time = Time::new();
        time.update(1.0 / 60.0);
        assert!((time.fps() - 60.0).abs() < 1.0);

        time.update(0.0);
        assert_eq!(time.fps(), 0.0); // Division by zero koruması
    }

    #[test]
    fn test_custom_max_dt() {
        let mut time = Time::new();
        time.set_max_dt(1.0 / 10.0); // 100ms
        time.update(0.5);

        assert!((time.dt() - 0.1).abs() < 0.0001); // 100ms cap
    }

    #[test]
    fn test_serde_derive() {
        // serde derive doğru çalışıyor — serialize/deserialize uygulanmış
        let mut time = Time::new();
        time.update(0.016);
        time.update(0.016);

        // Clone ile roundtrip kontrolü (serde_json bağımlılık gerektirmeden)
        let cloned = time;
        assert_eq!(cloned.frame(), time.frame());
        assert!((cloned.elapsed() - time.elapsed()).abs() < 0.001);
    }

    // ─── PhysicsTime Testleri ───

    #[test]
    fn test_physics_time_basic_step() {
        let mut pt = PhysicsTime::new(60);
        assert!(!pt.should_step()); // Henüz birikim yok

        pt.accumulate(1.0 / 60.0); // Tam bir fizik adımı
        assert!(pt.should_step());

        pt.consume_step();
        assert!(!pt.should_step());
        assert_eq!(pt.step_count(), 1);
    }

    #[test]
    fn test_physics_time_multiple_steps() {
        let mut pt = PhysicsTime::new(60);
        let fixed_dt = pt.fixed_dt();
        // 3.5 adıma yetecek birikim (FP hassasiyeti için margin)
        pt.accumulate(fixed_dt * 3.5);

        let mut steps = 0;
        while pt.should_step() {
            pt.consume_step();
            steps += 1;
        }
        assert_eq!(steps, 3);
        assert_eq!(pt.step_count(), 3);
    }

    #[test]
    fn test_physics_time_spiral_of_death() {
        let mut pt = PhysicsTime::new(60);
        // 1 saniyelik spike — max 8 adım birikebilir
        pt.accumulate(1.0);

        let mut steps = 0;
        while pt.should_step() {
            pt.consume_step();
            steps += 1;
        }
        assert_eq!(
            steps, 8,
            "spiral guard: a one-second delta must clamp to exactly the 8 steps the cap allows, \
             not merely to at most 8 — `<=` passed for every smaller number too, including the \
             3 the windowed path is really limited to"
        );
    }

    /// **The cap that governs on the windowed path is `Time::max_dt`, not this one.**
    ///
    /// The type's docs promise "however long a stall lasts, at most 8 steps": true, and on the
    /// windowed runtime unreachable, because `Time::update` clamps the delta to 0.05 s before
    /// `PhysicsTime` ever sees it. Three steps, not eight — and the difference matters to anyone
    /// sizing a catch-up budget from the 8.
    #[test]
    fn the_windowed_clamp_and_not_the_cap_is_what_limits_catch_up() {
        let mut time = Time::new();
        time.update(1.0); // a one-second stall: a breakpoint, a window drag
        assert!(
            (time.dt() - 0.05).abs() < 1e-6,
            "Time clamps to max_dt first: {}",
            time.dt()
        );

        let mut pt = PhysicsTime::new(60);
        pt.accumulate(time.dt());
        let mut steps = 0;
        while pt.should_step() {
            pt.consume_step();
            steps += 1;
        }
        assert_eq!(
            steps, 2,
            "0.05 s at 60 Hz is TWO steps, not the three the arithmetic suggests: 3 x fixed_dt is \
             0.050000004 in f32, a hair over the clamp"
        );

        // Three only when a leftover was already banked — and that is the ceiling, since the
        // leftover is always below one step: r + 0.05 < 4 x fixed_dt.
        let mut with_leftover = PhysicsTime::new(60);
        with_leftover.accumulate(0.016);
        with_leftover.accumulate(time.dt());
        let mut leftover_steps = 0;
        while with_leftover.should_step() {
            with_leftover.consume_step();
            leftover_steps += 1;
        }
        assert_eq!(leftover_steps, 3, "with a leftover banked, three — and never four");

        // Raise the rate and the cap starts governing again: at 240 Hz the same stalled frame
        // wants twelve steps and the accumulator only holds eight.
        let mut fast = PhysicsTime::new(240);
        fast.accumulate(time.dt());
        let mut fast_steps = 0;
        while fast.should_step() {
            fast.consume_step();
            fast_steps += 1;
        }
        assert_eq!(
            fast_steps, 8,
            "8/240 s = 0.0333 s is below the 0.05 s clamp, so above ~140 Hz the cap is live again"
        );
    }

    #[test]
    fn test_physics_time_alpha() {
        let mut pt = PhysicsTime::new(60);
        let fixed_dt = 1.0 / 60.0;

        // 1.5 fizik adımı birikim
        pt.accumulate(fixed_dt * 1.5);
        pt.consume_step(); // 1 adım tüket
        pt.compute_alpha();

        // Kalan 0.5 adım → alpha ≈ 0.5
        assert!(
            (pt.alpha() - 0.5).abs() < 0.01,
            "Alpha ≈ 0.5: {}",
            pt.alpha()
        );
    }

    #[test]
    fn test_physics_time_elapsed() {
        let mut pt = PhysicsTime::new(60);
        for _ in 0..60 {
            pt.accumulate(1.0 / 60.0);
            while pt.should_step() {
                pt.consume_step();
            }
        }
        // 60 adım × 1/60 = 1.0s
        assert!((pt.physics_elapsed() - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_physics_time_set_hz() {
        let mut pt = PhysicsTime::new(60);
        pt.set_hz(120);
        assert!((pt.fixed_dt() - 1.0 / 120.0).abs() < 1e-6);
    }

    /// Drives the accumulator at `render_hz` against a `fixed_hz` step and returns, for each
    /// simulated step, how many rendered frames had passed since the previous one — i.e. how
    /// many frames in a row a renderer that does not interpolate draws the *same* pose.
    fn hold_lengths(render_hz: u32, fixed_hz: u32, frames: u32) -> Vec<u32> {
        let mut pt = PhysicsTime::new(fixed_hz);
        let render_dt = 1.0 / render_hz as f32;
        let mut holds = Vec::new();
        let mut since = 0u32;
        for _ in 0..frames {
            pt.accumulate(render_dt);
            let mut stepped = false;
            while pt.should_step() {
                pt.consume_step();
                stepped = true;
            }
            pt.compute_alpha();
            since += 1;
            if stepped {
                holds.push(since);
                since = 0;
            }
        }
        holds
    }

    /// **What not interpolating costs, as a number.** This is the measurement behind
    /// [`PhysicsTime::alpha`]'s documentation, and the reason that documentation says the engine
    /// does not spend it.
    ///
    /// At 60 Hz physics on a 60 Hz display every pose is drawn exactly once and `alpha` never
    /// leaves 0 — nothing to interpolate, which is why the defect is invisible in the common
    /// case. Off that beat it is not: at 144 Hz each pose is held for **two or three** frames in
    /// an irregular 3,2,3,2,3,2,2… pattern, so the apparent per-frame displacement of a body
    /// moving at a constant speed swings by 50 % between neighbouring frames. That swing is the
    /// stutter; a uniform hold (300 Hz below: always 5) is merely a slideshow and looks smooth.
    #[test]
    fn a_render_rate_off_the_physics_beat_holds_each_pose_for_an_uneven_number_of_frames() {
        let on_beat = hold_lengths(60, 60, 240);
        assert!(
            on_beat.iter().all(|&h| h == 1),
            "60 Hz render against 60 Hz physics must draw every pose exactly once: {on_beat:?}"
        );

        let beating = hold_lengths(144, 60, 240);
        assert!(
            beating.contains(&2) && beating.contains(&3),
            "144 Hz against 60 Hz must alternate 2- and 3-frame holds — that alternation is the \
             stutter interpolation would remove: {beating:?}"
        );
        assert!(
            beating.iter().all(|&h| h == 2 || h == 3),
            "and it must never hold longer than that: {beating:?}"
        );

        let uniform = hold_lengths(300, 60, 240);
        assert!(
            uniform.iter().all(|&h| h == 5),
            "300 Hz is an exact multiple: every pose held for the same 5 frames, no beat: \
             {uniform:?}"
        );
    }

    /// `alpha` is exactly the unsimulated fraction of a step, so `alpha * fixed_dt` is **how
    /// stale the drawn pose is**. On the 144 Hz beat above it climbs to nearly a whole step —
    /// 16.7 ms at 60 Hz, which for a body moving at 10 m/s is 16.7 cm of position, sawtoothing
    /// sixty times a second. Nothing in the engine spends it, so that is what the picture shows.
    #[test]
    fn alpha_is_the_staleness_of_the_pose_the_renderer_draws() {
        let mut pt = PhysicsTime::new(60);
        let render_dt = 1.0 / 144.0;
        let mut worst = 0.0f32;
        for _ in 0..240 {
            pt.accumulate(render_dt);
            while pt.should_step() {
                pt.consume_step();
            }
            pt.compute_alpha();
            assert_eq!(
                pt.alpha(),
                pt.accumulator() / pt.fixed_dt(),
                "alpha is the leftover measured in steps, and nothing else"
            );
            worst = worst.max(pt.alpha() * pt.fixed_dt());
        }
        assert!(
            worst > 0.015,
            "the drawn pose should fall nearly a full 1/60 s step behind at some point on this \
             beat; worst was {worst} s"
        );
    }
}
