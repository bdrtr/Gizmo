//! Uçtan uca P2P rollback oturumu (Faz 3).
//!
//! `RollbackSession`, deterministik fiziği (`PhysicsWorld` + Faz 2/3'ün `state_hash` /
//! `snapshot`/`restore_snapshot`'ı) bir ağ transport'uyla birleştirip tam GGPO tarzı
//! rollback döngüsünü sürer: yerel girdiyi gönder → uzak girdileri al → yanlış-tahmin
//! varsa geçmişe dön + yeniden simüle et → ilerle. Transport SOYUT (`Transport` trait):
//!   * `UdpTransport` — gerçek ağ (P2P UDP),
//!   * `LoopbackTransport` — bellek-içi eşli kanal (lag + paket-kaybı simülasyonu); CI'da
//!     deterministik test için.

use super::input_buffer::{InputBuffer, PlayerInput};
use super::packet::NetworkPacket;
use gizmo_physics_rigid::{PhysicsWorld, WorldSnapshot};
use std::collections::HashMap;

/// Ağ taşıma soyutlaması — gerçek UDP ile test loopback'i aynı oturum kodunu kullanır.
pub trait Transport {
    fn send(&mut self, packet: &NetworkPacket);
    fn poll(&mut self) -> Vec<NetworkPacket>;
}

// UdpTransport'u trait'e bağla (gerçek ağ yolu).
impl Transport for super::transport::UdpTransport {
    fn send(&mut self, packet: &NetworkPacket) {
        let _ = self.send_packet(packet);
    }
    fn poll(&mut self) -> Vec<NetworkPacket> {
        self.poll_events().into_iter().map(|(_addr, p)| p).collect()
    }
}

/// Tek-iş-parçacıklı eşli bellek-içi transport (TEST). `pair(lag, drop_modulo)` iki uç döner.
/// lag = paket teslim gecikmesi (poll sayısı); drop_modulo = her N. gönderilen paket düşer
/// (0 = kayıp yok). Paket kaybına dayanıklılık için oturum son girdileri yeniden gönderir.
#[derive(Debug)]
pub struct LoopbackTransport {
    inbox: std::rc::Rc<std::cell::RefCell<std::collections::VecDeque<(u32, NetworkPacket)>>>,
    outbox: std::rc::Rc<std::cell::RefCell<std::collections::VecDeque<(u32, NetworkPacket)>>>,
    lag: u32,
    drop_modulo: u32,
    sent: u32,
}

impl LoopbackTransport {
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

/// Bir oyuncunun girdisini fiziğe uygulayan oyun-spesifik geri çağrı tipi.
pub type ApplyInput = dyn Fn(&mut PhysicsWorld, u32, &PlayerInput);

/// İki-oyunculu deterministik rollback oturumu (PhysicsWorld otoriter durum).
pub struct RollbackSession<T: Transport> {
    pub world: PhysicsWorld,
    pub tick: u64,
    transport: T,
    local_id: u32,
    remote_id: u32,
    local_buf: InputBuffer,
    remote_buf: InputBuffer,
    /// tick başına o tick'in BAŞINDAKİ tam durum (rollback için).
    snaps: HashMap<u64, WorldSnapshot>,
    max_rollback: u64,
    fixed_dt: f32,
    /// Paket kaybına dayanıklılık: her gönderide son bu kadar yerel girdiyi yeniden yolla.
    resend_window: u64,
}

impl<T: Transport> RollbackSession<T> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        world: PhysicsWorld,
        transport: T,
        local_id: u32,
        remote_id: u32,
        max_rollback: u64,
        fixed_dt: f32,
    ) -> Self {
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
        }
    }

    /// Mevcut durum hash'i (desync tespiti / test).
    pub fn state_hash(&self) -> u64 {
        self.world.state_hash()
    }

    /// Bir tick ilerlet. `local_input` bu tick'in yerel girdisi; `apply` girdiyi fiziğe uygular.
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
        let mut rollback_to: Option<u64> = None;
        for pkt in self.transport.poll() {
            if let NetworkPacket::Input(ri) = pkt {
                let predicted = self.remote_buf.get_or_predict(ri.tick);
                self.remote_buf.insert(ri);
                let diverged = predicted.buttons != ri.buttons
                    || predicted.joystick_x != ri.joystick_x
                    || predicted.joystick_y != ri.joystick_y;
                if diverged && ri.tick < t {
                    rollback_to = Some(rollback_to.map_or(ri.tick, |cur| cur.min(ri.tick)));
                }
            }
        }

        // Rollback: hedefin başına dön, hedef..t arası iki oyuncunun (düzeltilmiş) girdisiyle resim.
        if let Some(target) = rollback_to {
            if let Some(snap) = self.snaps.get(&target).cloned() {
                self.world.restore_snapshot(&snap);
                for rt in target..t {
                    self.snaps.insert(rt, self.world.snapshot());
                    let li = self.local_buf.get_or_predict(rt);
                    let rr = self.remote_buf.get_or_predict(rt);
                    apply(&mut self.world, self.local_id, &li);
                    apply(&mut self.world, self.remote_id, &rr);
                    self.world.step(self.fixed_dt).ok();
                }
            }
            // (snap yoksa: rollback penceresi aşıldı = desync; gerçek oyunda FullState istenir.)
        }

        // t'nin başını kaydet, iki oyuncunun girdisini uygula, ilerle.
        self.snaps.insert(t, self.world.snapshot());
        let ri = self.remote_buf.get_or_predict(t);
        apply(&mut self.world, self.local_id, &local_input);
        apply(&mut self.world, self.remote_id, &ri);
        self.world.step(self.fixed_dt).ok();
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
}
