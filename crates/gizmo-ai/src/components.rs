//! ECS components for grid navigation.
//!
//! [`NavAgent`] is the per-entity navigation state: a destination, the waypoint list
//! computed for it, and the tuning values that turn that list into a velocity. It is a
//! plain data component — it never moves anything by itself. All of it is driven by
//! [`crate::system::ai_navigation_system`], which requires the entity to also carry a
//! `Transform` and a `Velocity`; an agent on an entity missing either is skipped
//! entirely.
//!
//! Navigation is planar: the system overwrites the target's `y` with the agent's own `y` and
//! zeroes `Velocity::linear.y` on every update in which it steers. Pathfinding and steering
//! are therefore done entirely in XZ and never contribute vertical velocity — the agent's
//! height is whatever the rest of the pipeline leaves it at.

use gizmo_math::Vec3;

/// What an agent is currently doing, as last written by
/// [`crate::system::ai_navigation_system`].
///
/// This is an output of the navigation system, not an input: assigning it yourself has
/// no effect beyond the next update, which recomputes it from the target and path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub enum NavAgentState {
    /// No destination set. The navigation system damps `Velocity::linear` toward zero
    /// (multiplying it by `1 - min(dt * 5, 1)` each update) and does nothing else.
    ///
    /// This is the state of a freshly constructed agent, and the state a **loaded** one starts
    /// in: a scene stores what an agent *is*, not what it was doing when someone pressed ⏹.
    #[default]
    Idle,
    /// Following a path, or — when no path could be computed — steering straight at the
    /// target. Both cases look identical from here; use [`NavAgent::path_len`] to tell
    /// them apart.
    Moving,
    /// The agent arrived at its destination during the last update. The same update
    /// clears [`NavAgent::target`], so the *next* update sees no target and switches to
    /// [`NavAgentState::Idle`] — this state is visible for one update only.
    Reached,
    /// Set when the agent has stayed within 5 cm of its last recorded position for more
    /// than 2 seconds (both thresholds are hard-coded in the navigation system); the
    /// current path is dropped so a fresh one is computed.
    ///
    /// Transient in practice: the same agent update continues on to steer and overwrites
    /// the state with [`NavAgentState::Moving`] or [`NavAgentState::Reached`] before it
    /// returns, so an observer running after the navigation system will not see `Stuck`.
    /// To detect a blocked agent from outside, watch [`NavAgent::stuck_timer`] instead.
    Stuck,
}

/// Bookkeeping that decides when an agent's path is thrown away and recomputed.
///
/// Recomputation is not free (a full A* query per agent), so it is rate-limited by
/// `timer`/`interval` and additionally forced whenever the destination has moved far
/// enough that the old waypoints are worthless.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NavAgentRecalcState {
    /// Seconds remaining until the next scheduled recalculation. Counts *down* by `dt`
    /// each update; at `<= 0.0` a new path is computed and it is reloaded with
    /// `interval`. Setting it to `0.0` (as [`NavAgent::set_target`] does) forces a
    /// recalculation on the next update.
    pub timer: f32,
    /// Seconds between scheduled recalculations, i.e. the value `timer` is reset to.
    /// `0.0` means recompute the path every single update.
    pub interval: f32,
    /// Destination the current path was computed for, in world space (metres), with `y`
    /// already flattened to the agent's own height.
    ///
    /// If the live target has since moved more than 2 metres from this point, a
    /// recalculation is forced regardless of `timer`. `None` also forces one, which is
    /// why a newly created agent paths immediately.
    pub last_target_pos: Option<Vec3>,
}

impl Default for NavAgentRecalcState {
    /// The seeding [`NavAgent::new`] does: replan immediately, then twice a second.
    ///
    /// Written out rather than derived, and the difference is not cosmetic — a derived `Default`
    /// would give `interval: 0.0`, which means **recompute the path every single update**, i.e. a
    /// full A* query per agent per frame. This value is what a loaded agent gets, because the
    /// replan schedule is skipped by serde (see [`NavAgent`]).
    fn default() -> Self {
        Self {
            timer: 0.0,
            interval: 0.5,
            last_target_pos: None,
        }
    }
}

/// A path-following agent: where it wants to go, the route it is taking, and how hard it
/// is allowed to steer.
///
/// Attach it alongside a `Transform` and a `Velocity`: the navigation system reads the
/// transform and writes only `Velocity::linear`, never the transform itself. An agent on
/// an entity missing either component is skipped.
///
/// The waypoint list is private — it is owned by the navigation system, which replaces it
/// on every recalculation. Use [`set_path`](Self::set_path) /
/// [`clear_path`](Self::clear_path) to write it and
/// [`current_waypoint`](Self::current_waypoint) / [`path_len`](Self::path_len) /
/// [`path_index`](Self::path_index) to read it.
/// # What a scene file keeps
///
/// The **tuning** — `max_speed`, `steering_force`, `arrival_radius` — and the `target`, because
/// those are what an author sets. Everything else is skipped: the path, the cursor into it, the
/// state, the replan timer, the stall detector. Those describe a moment in a running simulation,
/// and a file that carried them would load an agent that believes it is halfway along a route
/// through a level that has just been rebuilt from scratch. (The same rule `AudioSource` needed
/// for its sink id, and for the same reason: runtime state outlives nothing.)
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct NavAgent {
    /// Destination in world space, metres. `None` means "stand still" — the agent is
    /// parked in [`NavAgentState::Idle`] and its velocity is damped out.
    ///
    /// The `y` component is ignored: the navigation system substitutes the agent's own
    /// height before pathing and steering, so only XZ matters.
    ///
    /// Cleared automatically on arrival, so an agent that has finished its journey is
    /// indistinguishable from one that was never given a destination. Prefer
    /// [`set_target`](Self::set_target) over assigning this directly: a raw assignment
    /// does not reset the recalculation timer, and a new destination within 2 metres of
    /// the old one is not far enough to force a recalculation on its own, so the agent
    /// may keep following the stale route until the timer next expires.
    #[serde(default)]
    pub target: Option<Vec3>,
    #[serde(skip)]
    path: Vec<Vec3>,
    #[serde(skip)]
    current_path_index: usize, // path.remove(0) yerine indeks takibi — O(1)
    /// Last state written by the navigation system. See [`NavAgentState`] — writing it
    /// yourself does not steer the agent.
    #[serde(skip)]
    pub state: NavAgentState,
    /// When the current route is thrown away and replanned. The navigation system only
    /// advances this while [`target`](Self::target) is `Some`, so an idle agent's timer
    /// does not tick down and its path is never replaced. [`NavAgent::new`] seeds it with
    /// `timer: 0.0`, `interval: 0.5` s and `last_target_pos: None`, which is what makes
    /// the first update after a target is set plan immediately.
    #[serde(skip)]
    pub recalc: NavAgentRecalcState,
    /// Speed cap in metres per second. Applied twice: as the magnitude of the desired
    /// velocity the steering behaviours aim for, and as a hard clamp on
    /// `Velocity::linear` after the steering force has been integrated — so the agent
    /// cannot be accelerated past it by its own steering.
    ///
    /// It is not a cap on the entity's velocity in general — the clamp is applied only
    /// while the navigation system is running, so anything else that writes the
    /// velocity in between is free to exceed it.
    pub max_speed: f32,
    /// Maximum magnitude of the steering vector, in metres per second squared.
    ///
    /// Despite the name this is an acceleration limit, not a newton force — mass is
    /// never involved; the navigation system integrates it as
    /// `linear += steering * dt`. Larger values mean sharper cornering and faster
    /// reaction; small values make the agent overshoot its waypoints.
    ///
    /// It bounds each behaviour separately rather than their sum: the separation force
    /// is clamped to this value, then added on top of the already-clamped seek/arrive
    /// force with a weight of 1.5, so one update can change velocity by up to
    /// `2.5 * steering_force * dt`.
    pub steering_force: f32,
    /// Waypoint switching distance in metres: once the agent is closer than this to its
    /// current waypoint, the cursor advances to the next one.
    ///
    /// It doubles as the tolerance for the final destination (the `arrive` behaviour
    /// starts slowing down at `2 * arrival_radius`), and there is an anti-stall rule
    /// that also advances the cursor when the agent has nearly stopped
    /// (< 0.2 m/s) within `2.5 * arrival_radius` of the waypoint.
    ///
    /// Acceptance is a distance test performed once per update, so a radius smaller than
    /// the distance covered in one update at `max_speed` can be stepped straight over,
    /// leaving the agent to overshoot and turn back.
    pub arrival_radius: f32,
    /// Seconds accumulated since the agent last made meaningful progress.
    ///
    /// It grows while the agent stays within 5 cm of `last_agent_pos` and is reset to
    /// zero the moment it moves further than that. Note that the reference point is not
    /// last update's position but the last position at which progress was recorded, so
    /// this measures genuine stalling rather than a single slow update. Crossing 2
    /// seconds triggers [`NavAgentState::Stuck`], but the timer keeps climbing until the
    /// agent actually moves.
    #[serde(skip)]
    pub stuck_timer: f32,
    /// Reference position for stall detection, in world space (metres) — the last place
    /// the agent was seen to have made progress, not necessarily its position last
    /// update.
    ///
    /// `None` until the navigation system first observes the agent, at which point it is
    /// seeded from the transform.
    #[serde(skip)]
    pub last_agent_pos: Option<Vec3>,
}

impl NavAgent {
    /// Builds an idle agent: no destination, no path, [`NavAgentState::Idle`], and a
    /// recalculation interval of 0.5 s.
    ///
    /// `max_speed` is in metres per second, `steering_force` in metres per second
    /// squared, `arrival_radius` in metres; see the corresponding fields for what each
    /// one controls. The values are stored verbatim — nothing is validated or clamped.
    /// A non-negative `max_speed` is assumed: the speed clamp compares against it
    /// directly, so a negative value flips the agent's velocity every update instead of
    /// limiting it.
    pub fn new(max_speed: f32, steering_force: f32, arrival_radius: f32) -> Self {
        Self {
            target: None,
            path: Vec::new(),
            current_path_index: 0,
            state: NavAgentState::Idle,
            recalc: NavAgentRecalcState {
                timer: 0.0,
                interval: 0.5,
                last_target_pos: None,
            },
            max_speed,
            steering_force,
            arrival_radius,
            stuck_timer: 0.0,
            last_agent_pos: None,
        }
    }

    /// Replaces the waypoint list and rewinds the cursor to the first waypoint.
    ///
    /// `path` is a sequence of world-space points in metres ordered from the agent
    /// outward, ending at the destination. Passing an empty vector is legal and leaves
    /// the agent [`is_done`](Self::is_done) — it will then steer straight at
    /// [`target`](Self::target) with no route.
    ///
    /// Neither the target nor the state is touched, so this does not by itself make an
    /// idle agent move. While [`target`](Self::target) is `None` the navigation system
    /// skips the agent before it looks at the path at all, so a hand-written route
    /// survives indefinitely; once a target is set, the system replaces it at the next
    /// recalculation — at the latest after [`NavAgentRecalcState::interval`] seconds.
    pub fn set_path(&mut self, path: Vec<Vec3>) {
        self.path = path;
        self.current_path_index = 0;
    }

    /// Discards the current route and rewinds the cursor.
    ///
    /// This does *not* stop the agent: [`target`](Self::target) survives, and an empty
    /// path forces a recalculation on the very next update, so the route comes straight
    /// back. If no path can be found the agent steers directly at the target instead.
    /// Use [`clear_target`](Self::clear_target) to actually halt.
    pub fn clear_path(&mut self) {
        self.path.clear();
        self.current_path_index = 0;
    }

    /// Stops the agent: clears the destination AND the current path. Clearing only the
    /// path (see [`clear_path`]) leaves `target` set, so the navigation system just
    /// recomputes the path and keeps moving — use this to actually halt the agent.
    pub fn clear_target(&mut self) {
        self.target = None;
        self.clear_path();
    }

    /// The waypoint currently being steered towards, in world space (metres), or `None`
    /// once the route has been walked to the end (equivalently, when
    /// [`is_done`](Self::is_done) is `true`).
    ///
    /// This is the *next* point to reach, not the last one passed.
    pub fn current_waypoint(&self) -> Option<&Vec3> {
        self.path.get(self.current_path_index)
    }

    /// Moves the cursor to the next waypoint.
    ///
    /// Purely an index bump: it does not check that the current waypoint was actually
    /// reached, and it does not saturate at the end of the route — calling it on a
    /// finished path just pushes the cursor further past the end, which
    /// [`is_done`](Self::is_done) and [`current_waypoint`](Self::current_waypoint)
    /// handle without panicking. No waypoint is ever removed, so
    /// [`path_len`](Self::path_len) is unaffected.
    pub fn advance(&mut self) {
        self.current_path_index += 1;
    }

    /// Whether the route has been exhausted — the cursor is at or past the last
    /// waypoint.
    ///
    /// Also `true` when there is no path at all, so a freshly constructed agent is
    /// "done". This means "nothing left to follow", not "arrived": the agent may still
    /// be far from [`target`](Self::target), and the navigation system treats a done
    /// path as a signal to recompute one.
    pub fn is_done(&self) -> bool {
        self.current_path_index >= self.path.len()
    }

    /// Total number of waypoints in the current route, including the ones already
    /// passed. `0` when the agent has no path.
    pub fn path_len(&self) -> usize {
        self.path.len()
    }

    /// Index of the waypoint being steered towards, counting from `0` at the start of
    /// the route. Equal to or greater than [`path_len`](Self::path_len) once the route
    /// is finished, so `path_len() - path_index()` is only a valid "waypoints remaining"
    /// count while [`is_done`](Self::is_done) is `false`.
    pub fn path_index(&self) -> usize {
        self.current_path_index
    }

    /// Sets the destination (world space, metres) and forces the path to be recomputed
    /// on the next navigation update by zeroing the recalculation timer.
    ///
    /// The `y` component is ignored during navigation — the agent's own height is used
    /// instead. The existing path is deliberately *not* cleared, so the agent keeps
    /// following its old route until the recalculation replaces it rather than stalling
    /// for an update.
    pub fn set_target(&mut self, target: Vec3) {
        self.target = Some(target);
        self.recalc.timer = 0.0; // Zorla yeniden hesaplat
    }
}

impl Default for NavAgent {
    /// A human-scale walker: 5 m/s top speed, 10 m/s² of steering authority, and a
    /// 0.5 m arrival radius.
    fn default() -> Self {
        Self::new(5.0, 10.0, 0.5)
    }
}

gizmo_core::impl_component!(NavAgent);

#[cfg(test)]
mod tests {
    use super::*;

    // REGRESYON (audit round 2): ai.clear_target() hem hedefi hem path'i temizlemeli.
    // Sadece path temizlenirse target durur, ai_navigation_system yeniden hesaplayıp
    // ajanı yürütmeye devam eder.
    #[test]
    fn clear_target_clears_both_target_and_path() {
        let mut a = NavAgent::default();
        a.set_target(Vec3::new(5.0, 0.0, 0.0));
        a.set_path(vec![Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0)]);
        assert!(a.target.is_some() && a.path_len() > 0);

        a.clear_target();
        assert!(a.target.is_none(), "clear_target hedefi temizlemeli");
        assert_eq!(a.path_len(), 0, "clear_target path'i de temizlemeli");
    }

    #[test]
    fn clear_path_keeps_target() {
        // Ayrımı belgeler: clear_path tek başına ajanı durdurmaz (target kalır).
        let mut a = NavAgent::default();
        a.set_target(Vec3::new(5.0, 0.0, 0.0));
        a.set_path(vec![Vec3::ZERO]);
        a.clear_path();
        assert!(a.target.is_some(), "clear_path target'ı temizlememeli");
        assert_eq!(a.path_len(), 0);
    }
}

#[cfg(test)]
mod scene_serde_tests {
    use super::*;

    /// **What a scene keeps of an agent is what an author set.**
    ///
    /// The tuning and the destination survive; the route, the cursor into it, the state and the
    /// replan schedule do not. A file that carried those would load an agent that believes it is
    /// halfway along a path through a level that has just been rebuilt.
    #[test]
    fn a_saved_agent_keeps_its_tuning_and_forgets_where_it_was() {
        let mut agent = NavAgent::new(3.5, 12.0, 0.75);
        agent.set_target(Vec3::new(4.0, 0.0, -9.0));
        agent.set_path(vec![Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), Vec3::new(2.0, 0.0, 0.0)]);
        agent.advance();
        agent.state = NavAgentState::Moving;
        agent.stuck_timer = 1.9;
        agent.last_agent_pos = Some(Vec3::new(1.0, 0.0, 0.0));

        let json = serde_json::to_string(&agent).expect("NavAgent is serializable");
        let loaded: NavAgent = serde_json::from_str(&json).expect("and round-trips");

        assert_eq!(loaded.max_speed, 3.5);
        assert_eq!(loaded.steering_force, 12.0);
        assert_eq!(loaded.arrival_radius, 0.75);
        assert_eq!(loaded.target, Some(Vec3::new(4.0, 0.0, -9.0)));

        assert_eq!(loaded.path_len(), 0, "the route is a moment, not a property");
        assert_eq!(loaded.path_index(), 0);
        assert_eq!(loaded.state, NavAgentState::Idle);
        assert_eq!(loaded.stuck_timer, 0.0);
        assert_eq!(loaded.last_agent_pos, None);
    }

    /// The replan schedule a loaded agent gets is the one `new` seeds — **not** a derived
    /// `Default`, whose `interval: 0.0` would mean a full A* query per agent per frame.
    #[test]
    fn a_loaded_agent_replans_twice_a_second_not_every_frame() {
        let json = serde_json::to_string(&NavAgent::default()).unwrap();
        let loaded: NavAgent = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.recalc.interval, NavAgent::default().recalc.interval);
        assert!(loaded.recalc.interval > 0.0, "0.0 is 'replan every update'");
        assert_eq!(loaded.recalc.timer, 0.0, "and the first update plans at once");
    }

    /// A scene written before any of this loads: every skipped field has a default, so the file
    /// only has to carry what an author set.
    #[test]
    fn a_file_with_only_the_authored_fields_loads() {
        let minimal = r#"{"max_speed":6.0,"steering_force":9.0,"arrival_radius":1.0}"#;
        let agent: NavAgent = serde_json::from_str(minimal).expect("the tuning is enough");
        assert_eq!(agent.max_speed, 6.0);
        assert_eq!(agent.target, None);
        assert_eq!(agent.state, NavAgentState::Idle);
    }
}
