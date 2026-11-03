//! The end-to-end P2P rollback session (phase 3).
//!
//! `RollbackSession` joins deterministic physics (`PhysicsWorld` plus phase 2/3's `state_hash` /
//! `snapshot` / `restore_snapshot`) to a network transport and drives the full GGPO-style
//! rollback loop: send the local input → receive remote inputs → on a misprediction, roll back
//! and re-simulate → advance. The transport is ABSTRACT (the `Transport` trait):
//!   * `UdpTransport` — a real network (P2P UDP),
//!   * `LoopbackTransport` — an in-memory paired channel (with lag and packet-loss simulation),
//!     for deterministic tests in CI.

use super::input_buffer::{InputBuffer, PlayerInput};
use super::packet::NetworkPacket;
use gizmo_physics_rigid::{PhysicsWorld, WorldSnapshot};
use std::collections::HashMap;

/// The network transport abstraction — real UDP and the test loopback run the same session
/// code.
pub trait Transport {
    /// Hands one packet to the network, best effort. Delivery, ordering and uniqueness
    /// are all unguaranteed and failures are not reported back to the caller — by design:
    /// [`RollbackSession`] absorbs loss by re-sending its last few local inputs (a
    /// `tick-8..=tick` window) on every tick instead of asking for retransmission.
    ///
    /// Implementations must not block; this runs inline in the fixed-step loop. The
    /// `UdpTransport` impl folds send errors into a `tracing::warn!`, and while its
    /// `remote_addr` is still `None` (no peer known yet) it discards the packet without
    /// even that — a session started before the peer is known simply loses its opening
    /// ticks, which the resend window then papers over.
    fn send(&mut self, packet: &NetworkPacket);

    /// Drains everything that has arrived since the last call and returns it in arrival
    /// order; an empty `Vec` means nothing was waiting. Must never block — a poll that
    /// waits for the peer stalls the simulation and defeats the point of rollback.
    ///
    /// Each packet is yielded exactly once; a second poll will not repeat it.
    /// [`RollbackSession::advance`] calls this exactly once per tick, so poll cadence
    /// *is* the tick rate — [`LoopbackTransport`] leans on that and uses the poll count
    /// itself as the clock for its lag simulation.
    fn poll(&mut self) -> Vec<NetworkPacket>;
}

// UdpTransport'u trait'e bağla (gerçek ağ yolu).
impl Transport for super::transport::UdpTransport {
    fn send(&mut self, packet: &NetworkPacket) {
        if let Err(e) = self.send_packet(packet) {
            tracing::warn!(error = ?e, "UDP paket gönderimi başarısız (rollback oturumu)");
        }
    }
    fn poll(&mut self) -> Vec<NetworkPacket> {
        self.poll_events().into_iter().map(|(_addr, p)| p).collect()
    }
}

/// A single-threaded paired in-memory transport (TEST). `pair(lag, drop_modulo)` returns the two
/// ends. `lag` is the delivery delay in polls; `drop_modulo` drops every Nth sent packet (0 = no
/// loss). To survive packet loss, the session re-sends its recent inputs.
#[derive(Debug)]
pub struct LoopbackTransport {
    inbox: std::rc::Rc<std::cell::RefCell<std::collections::VecDeque<(u32, NetworkPacket)>>>,
    outbox: std::rc::Rc<std::cell::RefCell<std::collections::VecDeque<(u32, NetworkPacket)>>>,
    lag: u32,
    drop_modulo: u32,
    sent: u32,
}

impl LoopbackTransport {
    /// Builds two connected endpoints: whatever one sends, the *other* polls. Each
    /// endpoint is one-directional — a sender never receives its own packets — and both
    /// are created with the same `lag` / `drop_modulo` settings, so the simulated link is
    /// symmetric.
    ///
    /// `lag` is counted in **`poll()` calls on the receiving endpoint**, not seconds and
    /// not ticks: with `lag = 2` a packet first surfaces on the receiver's third poll.
    /// Since [`RollbackSession`] polls exactly once per tick, inside a session that does
    /// amount to "N ticks of latency".
    ///
    /// `drop_modulo` discards every Nth packet **sent by that endpoint** (counted from 1,
    /// tracked per endpoint); `0` disables loss entirely. Packets that survive keep their
    /// send order.
    ///
    /// Deterministic by construction — no RNG, no wall clock — which is what lets the
    /// convergence test assert exact `state_hash` equality against a ground-truth run
    /// under lag *and* loss. Single-threaded only: the two queues are shared through
    /// `Rc<RefCell<_>>`, so both endpoints must be driven from the same thread. This is
    /// test scaffolding, not a shippable transport.
    pub fn pair(lag: u32, drop_modulo: u32) -> (Self, Self) {
        use std::cell::RefCell;
        use std::collections::VecDeque;
        use std::rc::Rc;
        let a = Rc::new(RefCell::new(VecDeque::new()));
        let b = Rc::new(RefCell::new(VecDeque::new()));
        (
            Self { inbox: a.clone(), outbox: b.clone(), lag, drop_modulo, sent: 0 },
            Self { inbox: b, outbox: a, lag, drop_modulo, sent: 0 },
        )
    }
}

impl Transport for LoopbackTransport {
    fn send(&mut self, packet: &NetworkPacket) {
        self.sent += 1;
        if self.drop_modulo != 0 && self.sent.is_multiple_of(self.drop_modulo) {
            return; // paket kaybı simülasyonu
        }
        self.outbox.borrow_mut().push_back((self.lag, packet.clone()));
    }
    fn poll(&mut self) -> Vec<NetworkPacket> {
        let mut ready = Vec::new();
        let mut q = self.inbox.borrow_mut();
        let mut keep = std::collections::VecDeque::with_capacity(q.len());
        while let Some((d, p)) = q.pop_front() {
            if d == 0 {
                ready.push(p);
            } else {
                keep.push_back((d - 1, p));
            }
        }
        *q = keep;
        ready
    }
}

/// The game-specific callback type that applies one player's input to the physics.
pub type ApplyInput = dyn Fn(&mut PhysicsWorld, u32, &PlayerInput);

/// A two-player deterministic rollback session (PhysicsWorld is the authoritative state).
pub struct RollbackSession<T: Transport> {
    /// The authoritative simulation state. Public so gameplay and rendering can read it
    /// (and so it can be seeded before the first tick), but [`RollbackSession::advance`]
    /// may rewind and re-simulate it in place — anything read out of it belongs to a
    /// *predicted* frontier and can change retroactively once the remote input for that
    /// tick lands.
    ///
    /// The body set must stay fixed for the life of the session. Rollback restores
    /// transforms/velocities/bodies/contacts/joints **by array index** and assumes
    /// `entities` and `colliders` are untouched, so adding or removing a body mid-session
    /// misaligns the SoA arrays and corrupts every subsequent restore.
    pub world: PhysicsWorld,

    /// Ticks simulated so far, and therefore the tick `advance` will simulate next.
    /// Starts at 0 and increments once per `advance`, after the step.
    ///
    /// A rollback never moves it backwards: re-simulation replays *up to* this tick
    /// rather than rewinding the frontier, so the counter is monotonic. Ticks, not
    /// seconds — simulated time is `tick * fixed_dt`. Nor is it a shared clock: each peer
    /// advances its own and [`advance`](Self::advance) never stalls waiting for remote input
    /// (it predicts instead), so two peers' tick counters drift apart in wall time unless the
    /// game drives the session with [`try_advance`](Self::try_advance), which holds the one
    /// that is ahead.
    pub tick: u64,
    transport: T,
    local_id: u32,
    remote_id: u32,
    local_buf: InputBuffer,
    remote_buf: InputBuffer,
    /// Per tick, the complete state as it was at the START of that tick (for rollback).
    snaps: HashMap<u64, WorldSnapshot>,
    max_rollback: u64,
    fixed_dt: f32,
    /// Packet-loss resilience: re-send this many of the most recent local inputs on every send.
    resend_window: u64,
    /// The newest tick a remote input has arrived for — what [`frames_ahead`](Self::frames_ahead)
    /// measures against. `None` until the peer's first input lands.
    remote_tick: Option<u64>,
    /// The earliest past tick whose remote input contradicted the prediction, gathered by
    /// [`poll_remote`](Self::poll_remote) and consumed by the next [`advance`](Self::advance).
    pending_rollback: Option<u64>,
}

impl<T: Transport> RollbackSession<T> {
    /// Wraps an already-populated `world` and a transport into a session that starts at
    /// tick 0.
    ///
    /// Both peers must start from **bit-identical** worlds (same bodies, added in the
    /// same order, same `fixed_dt`). Nothing here negotiates or verifies that, and a
    /// mismatch never raises an error — it only shows up as `state_hash` values that
    /// never converge. `local_id` / `remote_id` are opaque tags passed straight through
    /// to the [`ApplyInput`] callback, and the two peers must mirror them: if A is
    /// `(0, 1)`, B is `(1, 0)`.
    ///
    /// `max_rollback` is a count of **ticks** of snapshot history (at 60 Hz, `120` ≈ 2 s).
    /// It is the hard limit on how late a corrected remote input may arrive: a
    /// misprediction older than the window has no snapshot left to rewind to, so
    /// `advance` can only log the desync and keep going — recovering from that needs a
    /// [`NetworkPacket::FullState`] resync, which this type does not implement. It is
    /// also the memory knob, since one full `WorldSnapshot` (all transforms, velocities,
    /// rigid bodies, the contact warm-start cache and every joint) is retained per tick
    /// in the window.
    ///
    /// `fixed_dt` is **seconds of simulated time per tick**, handed straight to
    /// `PhysicsWorld::step`. The world sub-steps it internally at 240 Hz and carries the
    /// remainder in an accumulator that is itself part of the snapshot, so a `dt` that is
    /// not a whole multiple of 1/240 s still rewinds exactly.
    ///
    /// Input history is sized to `max(max_rollback + 8, 64)` ticks so it always outlives
    /// the snapshot window — an input can never expire while a snapshot that needs it is
    /// still around.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        world: PhysicsWorld,
        transport: T,
        local_id: u32,
        remote_id: u32,
        max_rollback: u64,
        fixed_dt: f32,
    ) -> Self {
        tracing::info!(
            local_id,
            remote_id,
            max_rollback,
            fixed_dt,
            "P2P rollback oturumu oluşturuldu"
        );
        let cap = (max_rollback as usize + 8).max(64);
        Self {
            world,
            tick: 0,
            transport,
            local_id,
            remote_id,
            local_buf: InputBuffer::new(local_id, cap),
            remote_buf: InputBuffer::new(remote_id, cap),
            snaps: HashMap::new(),
            max_rollback,
            fixed_dt,
            resend_window: 8,
            remote_tick: None,
            pending_rollback: None,
        }
    }

    /// How many ticks this peer has simulated past the newest input it has from the other one:
    /// `tick - (newest remote tick + 1)`, or `tick` before anything has arrived.
    ///
    /// **Why it exists.** The session never waits for the peer — it predicts — so nothing kept
    /// two peers' clocks together: one that started 150 ticks earlier stayed 150 ticks ahead
    /// for good, every remote input reached it 150 ticks late, past a 120-tick rollback window,
    /// and the two never agreed again. In steady play this hovers around the one-way lag in
    /// ticks; a peer far ahead of that should wait — [`try_advance`](Self::try_advance) does.
    pub fn frames_ahead(&self) -> u64 {
        self.tick.saturating_sub(self.remote_tick.map_or(0, |t| t + 1))
    }

    /// Drains the transport without simulating: stores the remote inputs that arrived and notes
    /// any that contradict a past prediction, for the next [`advance`](Self::advance) to roll
    /// back to. What a waiting peer calls each frame so it hears the other one catch up.
    pub fn poll_remote(&mut self) {
        let t = self.tick;
        for pkt in self.transport.poll() {
            if let NetworkPacket::Input(ri) = pkt {
                let predicted = self.remote_buf.get_or_predict(ri.tick);
                self.remote_buf.insert(ri);
                self.remote_tick = Some(self.remote_tick.map_or(ri.tick, |r| r.max(ri.tick)));
                let diverged = predicted.buttons != ri.buttons
                    || predicted.joystick_x != ri.joystick_x
                    || predicted.joystick_y != ri.joystick_y;
                if diverged && ri.tick < t {
                    self.pending_rollback = Some(self.pending_rollback.map_or(ri.tick, |cur| cur.min(ri.tick)));
                }
            }
        }
    }

    /// [`advance`](Self::advance), unless this peer is more than `max_lead` ticks ahead of the
    /// other ([`frames_ahead`](Self::frames_ahead)) — then it only polls and returns `false`,
    /// and the caller should hold `local_input` and try again next frame.
    ///
    /// This is the time sync the session otherwise lacks: a peer that starts first runs
    /// `max_lead` ticks and waits for the other to appear, and one whose frames run faster is
    /// held back to the slower one. Pick `max_lead` above the one-way lag in ticks (a 50 ms
    /// link is 3 ticks at 60 Hz) and below `max_rollback`, so a late input can always still be
    /// rolled back to.
    pub fn try_advance(&mut self, local_input: PlayerInput, apply: &ApplyInput, max_lead: u64) -> bool {
        if self.frames_ahead() > max_lead {
            self.poll_remote();
            return false;
        }
        self.advance(local_input, apply);
        true
    }

    /// Mevcut durum hash'i (desync tespiti / test).
    pub fn state_hash(&self) -> u64 {
        self.world.state_hash()
    }

    /// Advance one tick. `local_input` is this tick's local input; `apply` applies an input to
    /// the physics.
    #[tracing::instrument(skip_all, name = "rollback_advance")]
    pub fn advance(&mut self, mut local_input: PlayerInput, apply: &ApplyInput) {
        let t = self.tick;
        local_input.tick = t;
        self.local_buf.insert(local_input);

        // Yerel girdiyi + son resend_window girdiyi yolla (paket-kaybı dayanıklılığı).
        let from = t.saturating_sub(self.resend_window);
        for rt in from..=t {
            let inp = self.local_buf.get_or_predict(rt);
            if inp.tick == rt {
                self.transport.send(&NetworkPacket::Input(inp));
            }
        }

        // Uzak girdileri al; geçmiş bir tick için tahmin bozulursa rollback hedefi belirle.
        self.poll_remote();
        let rollback_to = self.pending_rollback.take();

        // Rollback: hedefin başına dön, hedef..t arası iki oyuncunun (düzeltilmiş) girdisiyle resim.
        if let Some(target) = rollback_to {
            if let Some(snap) = self.snaps.get(&target).cloned() {
                let resim_frames = t.saturating_sub(target);
                self.world.restore_snapshot(&snap);
                for rt in target..t {
                    self.snaps.insert(rt, self.world.snapshot());
                    let li = self.local_buf.get_or_predict(rt);
                    let rr = self.remote_buf.get_or_predict(rt);
                    apply(&mut self.world, self.local_id, &li);
                    apply(&mut self.world, self.remote_id, &rr);
                    // `.ok()` fizik adımı hatasını (NaN/Inf) sessizce yutuyordu; davranışı
                    // koruyup (kareyi atla, ilerlemeye devam et) artık bağlamlı raporluyoruz.
                    if let Err(e) = self.world.step(self.fixed_dt) {
                        tracing::warn!(tick = rt, error = ?e, "Rollback yeniden-simülasyonunda fizik adımı başarısız");
                    }
                }
                tracing::debug!(
                    target,
                    current_tick = t,
                    resim_frames,
                    "Rollback: hedef tick'e dönüldü ve yeniden simüle edildi"
                );
            } else {
                // snap yoksa: rollback penceresi aşıldı = desync; gerçek oyunda FullState istenir.
                tracing::warn!(
                    target,
                    current_tick = t,
                    max_rollback = self.max_rollback,
                    "Rollback penceresi aşıldı: hedef snapshot yok (desync); FullState gerekli"
                );
            }
        }

        // t'nin başını kaydet, iki oyuncunun girdisini uygula, ilerle.
        self.snaps.insert(t, self.world.snapshot());
        let ri = self.remote_buf.get_or_predict(t);
        apply(&mut self.world, self.local_id, &local_input);
        apply(&mut self.world, self.remote_id, &ri);
        if let Err(e) = self.world.step(self.fixed_dt) {
            tracing::warn!(tick = t, error = ?e, "Fizik adımı başarısız (advance ileri adım)");
        }
        self.tick += 1;

        // Eski snapshot'ları buda (pencere dışı).
        if t >= self.max_rollback {
            let cutoff = t - self.max_rollback;
            self.snaps.retain(|&k, _| k >= cutoff);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_physics_rigid::BodyHandle;
    use gizmo_math::Vec3;
    use gizmo_physics_core::{Collider, Transform};
    use gizmo_physics_rigid::{PhysicsWorld, RigidBody, Velocity};

    const DT: f32 = 1.0 / 60.0;

    // player 0 → cisim idx 1, player 1 → cisim idx 2 (zemin idx 0). Bağımsız cisimler →
    // uygulama sırası önemsiz (komütatif).
    fn body_of(player_id: u32) -> usize {
        if player_id == 0 { 1 } else { 2 }
    }

    fn build_scene() -> PhysicsWorld {
        let mut w = PhysicsWorld::new();
        let mut g = RigidBody::new_static();
        g.wake_up();
        w.add_body(BodyHandle::from_id(0), g, Transform::new(Vec3::new(0.0, -1.0, 0.0)),
            Velocity::default(), Collider::box_collider(Vec3::new(20.0, 1.0, 20.0)));
        for id in 1..=3u32 {
            let mut rb = RigidBody::new(1.0, true);
            rb.wake_up();
            let col = Collider::box_collider(Vec3::splat(0.5));
            rb.update_inertia_from_collider(&col);
            w.add_body(BodyHandle::from_id(id), rb,
                Transform::new(Vec3::new(id as f32 * 1.02 - 1.5, 0.5, 0.0)),
                Velocity::default(), col);
        }
        w
    }

    fn apply(w: &mut PhysicsWorld, player_id: u32, input: &PlayerInput) {
        let idx = body_of(player_id);
        if input.joystick_x != 0 && w.rigid_bodies[idx].is_sleeping {
            w.rigid_bodies[idx].wake_up();
        }
        let inv_m = w.rigid_bodies[idx].inv_mass();
        w.velocities[idx].linear.x += (input.joystick_x as f32 / 127.0) * 2.0 * inv_m;
    }

    fn input_for(player: u32, tick: usize) -> PlayerInput {
        let r = (tick.wrapping_mul(if player == 0 { 2654435761 } else { 40503 }) >> 20) % 7;
        PlayerInput { tick: tick as u64, buttons: 0, joystick_x: (r as i8 - 3) * 30, joystick_y: 0 }
    }

    #[test]
    fn two_peers_converge_under_lag_and_packet_loss() {
        const N: usize = 60;
        const DRAIN: usize = 25;
        let total = N + DRAIN;

        // Ground truth: tek dünya, her tick İKİ oyuncunun gerçek girdisiyle.
        let mut gt = build_scene();
        for t in 0..total {
            let i0 = if t < N { input_for(0, t) } else { PlayerInput::empty(t as u64) };
            let i1 = if t < N { input_for(1, t) } else { PlayerInput::empty(t as u64) };
            apply(&mut gt, 0, &i0);
            apply(&mut gt, 1, &i1);
            gt.step(DT).ok();
        }
        let truth = gt.state_hash();

        // İki peer, lag=3 + her 7. paket düşer (resend_window=8 ile kurtarılır).
        let (ta, tb) = LoopbackTransport::pair(3, 7);
        let mut a = RollbackSession::new(build_scene(), ta, 0, 1, 32, DT);
        let mut b = RollbackSession::new(build_scene(), tb, 1, 0, 32, DT);
        let apply_fn: &ApplyInput = &apply;

        for t in 0..total {
            let ia = if t < N { input_for(0, t) } else { PlayerInput::empty(t as u64) };
            let ib = if t < N { input_for(1, t) } else { PlayerInput::empty(t as u64) };
            a.advance(ia, apply_fn);
            b.advance(ib, apply_fn);
        }

        // Her iki peer birbirine VE ground-truth'a yakınsamalı (senkron).
        assert_eq!(a.state_hash(), b.state_hash(), "iki peer ayrıştı (desync)");
        assert_eq!(a.state_hash(), truth, "peer A ground-truth'a yakınsamadı (lag/loss sonrası)");
    }

    /// One peer starts `late` frames after the other; both play `N` ticks of real input then
    /// idle, each stopping at `total`. `lead` = `None` runs the plain `advance`, `Some(m)` runs
    /// `try_advance(.., m)`. Returns the two final hashes and the ground truth.
    fn late_start(late: usize, lead: Option<u64>) -> (u64, u64, u64) {
        const N: usize = 60;
        let total = N + 40;
        let mut gt = build_scene();
        for t in 0..total {
            let i0 = if t < N { input_for(0, t) } else { PlayerInput::empty(t as u64) };
            let i1 = if t < N { input_for(1, t) } else { PlayerInput::empty(t as u64) };
            apply(&mut gt, 0, &i0);
            apply(&mut gt, 1, &i1);
            gt.step(DT).ok();
        }
        let (ta, tb) = LoopbackTransport::pair(3, 0);
        let mut a = RollbackSession::new(build_scene(), ta, 0, 1, 32, DT);
        let mut b = RollbackSession::new(build_scene(), tb, 1, 0, 32, DT);
        let apply_fn: &ApplyInput = &apply;
        let input = |player: u32, t: u64| {
            if (t as usize) < N { input_for(player, t as usize) } else { PlayerInput::empty(t) }
        };
        let step = |s: &mut RollbackSession<LoopbackTransport>, player: u32| {
            let i = input(player, s.tick);
            match lead {
                None => s.advance(i, apply_fn),
                Some(m) => {
                    s.try_advance(i, apply_fn, m);
                }
            }
        };
        for frame in 0..total * 4 {
            if a.tick < total as u64 {
                step(&mut a, 0);
            } else {
                a.poll_remote();
            }
            if frame >= late {
                if b.tick < total as u64 {
                    step(&mut b, 1);
                } else {
                    b.poll_remote();
                }
            }
        }
        assert_eq!((a.tick, b.tick), (total as u64, total as u64));
        (a.state_hash(), b.state_hash(), gt.state_hash())
    }

    #[test]
    fn a_peer_that_starts_late_desyncs_without_time_sync() {
        // The measurement behind `try_advance`: 50 frames late against a 32-tick window.
        let (a, b, _) = late_start(50, None);
        assert_ne!(a, b, "expected the plain advance to desync after a late start");
    }

    #[test]
    fn try_advance_holds_the_early_peer_until_the_late_one_catches_up() {
        let (a, b, truth) = late_start(50, Some(8));
        assert_eq!(a, b, "the peers desynced");
        assert_eq!(a, truth, "the peers agree, but not with the real run");
    }

    #[test]
    fn frames_ahead_counts_from_the_newest_remote_input() {
        let (ta, tb) = LoopbackTransport::pair(0, 0);
        let mut a = RollbackSession::new(build_scene(), ta, 0, 1, 32, DT);
        let mut b = RollbackSession::new(build_scene(), tb, 1, 0, 32, DT);
        let apply_fn: &ApplyInput = &apply;
        for _ in 0..5 {
            a.advance(PlayerInput::empty(0), apply_fn);
        }
        assert_eq!(a.frames_ahead(), 5, "nothing heard from b yet");
        // `try_advance` refuses past the lead and leaves the tick alone.
        assert!(!a.try_advance(PlayerInput::empty(0), apply_fn, 4));
        assert_eq!(a.tick, 5);
        b.advance(PlayerInput::empty(0), apply_fn);
        b.advance(PlayerInput::empty(0), apply_fn);
        a.poll_remote();
        assert_eq!(a.frames_ahead(), 3, "b's tick 1 arrived: 5 - (1 + 1)");
        assert!(a.try_advance(PlayerInput::empty(0), apply_fn, 4));
    }

    // --- LoopbackTransport (deterministik test transport'unun kendisi) ---
    // Bu transport tüm rollback testlerinin temeli; buradaki bir hata convergence
    // testini geçersiz kılar, bu yüzden lag/drop/yön davranışı doğrudan test edilir.

    fn ping(ts: u64) -> NetworkPacket {
        NetworkPacket::Ping { timestamp: ts }
    }

    fn ping_ts(p: &NetworkPacket) -> u64 {
        match p {
            NetworkPacket::Ping { timestamp } => *timestamp,
            other => panic!("Ping beklendi, gelen {other:?}"),
        }
    }

    #[test]
    fn loopback_delivers_one_way_across_the_pair() {
        let (mut a, mut b) = LoopbackTransport::pair(0, 0);
        a.send(&ping(1));
        assert!(a.poll().is_empty(), "gönderen kendi paketini almamalı (tek yönlü)");
        let got = b.poll();
        assert_eq!(got.len(), 1);
        assert_eq!(ping_ts(&got[0]), 1);
        assert!(b.poll().is_empty(), "paket yalnız bir kez teslim edilmeli");
    }

    #[test]
    fn loopback_lag_delays_delivery_by_poll_count() {
        let (mut a, mut b) = LoopbackTransport::pair(2, 0); // 2 poll gecikme
        a.send(&ping(7));
        assert!(b.poll().is_empty(), "poll #1: henüz gelmemeli (lag=2)");
        assert!(b.poll().is_empty(), "poll #2: henüz gelmemeli");
        let got = b.poll(); // poll #3: teslim
        assert_eq!(got.len(), 1);
        assert_eq!(ping_ts(&got[0]), 7);
    }

    #[test]
    fn loopback_drops_every_nth_packet() {
        let (mut a, mut b) = LoopbackTransport::pair(0, 2); // her 2. gönderim düşer
        for ts in 1..=4 {
            a.send(&ping(ts));
        }
        // sent=1→teslim, 2→düş, 3→teslim, 4→düş → yalnız 1 ve 3 gelir, sıra korunur.
        let tss: Vec<u64> = b.poll().iter().map(ping_ts).collect();
        assert_eq!(tss, vec![1, 3], "her 2. paket düşmeli, sıra korunmalı");
    }

    #[test]
    fn loopback_never_drops_when_modulo_zero() {
        let (mut a, mut b) = LoopbackTransport::pair(0, 0);
        for ts in 1..=5 {
            a.send(&ping(ts));
        }
        assert_eq!(b.poll().len(), 5, "drop_modulo=0 iken hiçbir paket düşmemeli");
    }
}
