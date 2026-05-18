use gizmo_core::World;
use gizmo_math::Vec3;
use gizmo_network::{NetworkPacket, PlayerInput, RollbackManager, UdpTransport};
use gizmo_physics_core::components::transform::Transform;
use gizmo_physics_rigid::components::velocity::Velocity;
use std::env;
use std::net::SocketAddr;
use std::thread::sleep;
use std::time::Duration;

/// Basit bir gecikme simülatörü (Sahte Ping)
struct DelayQueue {
    queue: std::collections::VecDeque<(std::time::Instant, NetworkPacket)>,
    delay: Duration,
}

impl DelayQueue {
    fn new(delay_ms: u64) -> Self {
        Self {
            queue: std::collections::VecDeque::new(),
            delay: Duration::from_millis(delay_ms),
        }
    }

    fn push(&mut self, packet: NetworkPacket) {
        self.queue.push_back((std::time::Instant::now(), packet));
    }

    fn pop_ready(&mut self) -> Vec<NetworkPacket> {
        let mut ready = Vec::new();
        let now = std::time::Instant::now();
        while let Some(&(time, _)) = self.queue.front() {
            if now.duration_since(time) >= self.delay {
                ready.push(self.queue.pop_front().unwrap().1);
            } else {
                break;
            }
        }
        ready
    }
}

fn main() {
    // Argümanlar: local_port remote_port
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        println!("Usage: cargo run --example p2p_rollback_test <local_port> <remote_port>");
        println!("Example: cargo run --example p2p_rollback_test 8000 8001");
        return;
    }

    let local_port: u16 = args[1].parse().expect("Invalid local port");
    let remote_port: u16 = args[2].parse().expect("Invalid remote port");

    println!("Starting Rollback Node on port {} (Target: {})", local_port, remote_port);

    let mut transport = UdpTransport::bind(local_port).expect("Failed to bind UDP socket");
    let remote_addr: SocketAddr = format!("127.0.0.1:{}", remote_port).parse().unwrap();
    transport.set_remote(remote_addr);

    let mut world = World::new();
    let mut manager = RollbackManager::new(120);
    manager.register_player(1, 120); // Local player
    manager.register_player(2, 120); // Remote player

    let ent = world.spawn();
    world.add_component(ent, Transform::default());
    world.add_component(ent, Velocity::default());

    // Ağdan gelen paketleri kasten bekleten kuyruk (Örn: 100ms Ping)
    let mut fake_ping_queue = DelayQueue::new(100);

    for tick in 0..100 {
        // 1. Ağdan gelen paketleri oku
        let events = transport.poll_events();
        for (_, packet) in events {
            fake_ping_queue.push(packet);
        }

        // 2. Gecikme süresi dolmuş paketleri RollbackManager'a aktar
        for packet in fake_ping_queue.pop_ready() {
            if let NetworkPacket::Input(input) = packet {
                println!("[Tick {}] Gecikmeli Paket Geldi: {:?}", tick, input);
                manager.receive_remote_input(2, input);
            }
        }

        // 3. Rollback Tetiklenmesi Gerekiyor mu?
        if manager.begin_frame(&mut world) {
            println!(">>> ZAMAN MAKINESI AKTIF! Rollback to Tick {} <<<", manager.current_tick);
            // Geçmişten günümüze gizlice hızlı simülasyon (Fast-forward)
            while manager.current_tick < tick {
                // Simülasyon mantığı (Örn: objeyi hareket ettir)
                let remote_input = manager.input_buffers.get(&2).unwrap().get_or_predict(manager.current_tick);
                if let Some(mut trans) = world.borrow_mut::<Transform>().get_mut(ent.id()) {
                    trans.position.x += remote_input.joystick_x as f32;
                }
                manager.end_frame(&world);
            }
            println!(">>> Rollback Bitti. Güncel Tick: {} <<<", manager.current_tick);
        }

        // 4. Benim (Local) Girdim
        let mut my_input = PlayerInput::empty(tick);
        my_input.joystick_x = 1; // Hep sağa git
        
        // Girdimi ağa yolla
        transport.send_packet(&NetworkPacket::Input(my_input)).unwrap();

        // 5. Güncel Simülasyon
        let remote_input = manager.input_buffers.get(&2).unwrap().get_or_predict(manager.current_tick);
        if let Some(mut trans) = world.borrow_mut::<Transform>().get_mut(ent.id()) {
            trans.position.x += remote_input.joystick_x as f32;
        }

        manager.end_frame(&world);

        // 60 Hz simülasyon gecikmesi
        sleep(Duration::from_millis(16));
    }
}
