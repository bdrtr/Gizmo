#[cfg(test)]
mod tests {
    use crate::gpu_physics::fem::{GpuFemSystem, GpuSoftBodyNode, GpuTetrahedron, GpuFemParams};
    use wgpu::util::DeviceExt;

    // Helper to setup a headless wgpu device
    async fn setup_headless_gpu() -> (wgpu::Device, wgpu::Queue) {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });
        
        let adapter = instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }).await.expect("Failed to find wgpu adapter for tests");

        adapter.request_device(&wgpu::DeviceDescriptor::default(), None).await.expect("Failed to create wgpu device for tests")
    }

    // Helper to read back a buffer
    async fn read_buffer<T: bytemuck::Pod>(device: &wgpu::Device, queue: &wgpu::Queue, buffer: &wgpu::Buffer) -> Vec<T> {
        let size = buffer.size();
        let staging_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Test Staging Buffer"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        encoder.copy_buffer_to_buffer(buffer, 0, &staging_buffer, 0, size);
        queue.submit(Some(encoder.finish()));

        let buffer_slice = staging_buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |v| sender.send(v).unwrap());

        device.poll(wgpu::Maintain::Wait);
        receiver.recv().unwrap().unwrap();

        let data = buffer_slice.get_mapped_range();
        let result = bytemuck::cast_slice(&data).to_vec();
        drop(data);
        staging_buffer.unmap();
        
        result
    }

    #[test]
    fn test_fem_struct_sizes() {
        // Assert memory layouts are EXACTLY 16-byte aligned and size-matched for WGSL
        assert_eq!(std::mem::size_of::<GpuSoftBodyNode>(), 48, "Node size must be 48 bytes");
        assert_eq!(std::mem::size_of::<GpuTetrahedron>(), 80, "Tetrahedron size must be 80 bytes");
        assert_eq!(std::mem::size_of::<GpuFemParams>(), 48, "FEM Params size must be 48 bytes");
    }

    #[test]
    fn test_fem_compute_clear_forces() {
        pollster::block_on(async {
            let (device, queue) = setup_headless_gpu().await;

            // Create 1 dummy node with 10.0 mass
            let nodes = vec![GpuSoftBodyNode {
                position_mass: [0.0, 0.0, 0.0, 10.0],
                velocity_fixed: [0.0, 0.0, 0.0, 0.0],
                forces: [500, 500, 500, 0], // Pre-polluted forces
            }];
            let elements = vec![GpuTetrahedron {
                indices: [0, 0, 0, 0],
                inv_rest_col0: [0.0; 4],
                inv_rest_col1: [0.0; 4],
                inv_rest_col2: [0.0; 4],
                rest_volume_pad: [0.0; 4],
            }];
            
            // Gravity is -9.81 in Y.
            let params = GpuFemParams {
                properties: [0.001, 1.0, 1.0, 1.0],
                gravity: [0.0, -9.81, 0.0, 0.0],
                counts: [1, 0, 0, 0],
            };

            let fem_system = GpuFemSystem::new(&device, &nodes, &elements, &[], &params);
            
            // Dispatch clear forces compute pass ONLY
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
            {
                let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: None,
                    timestamp_writes: None,
                });
                cpass.set_bind_group(0, &fem_system.compute_bind_group, &[]);
                cpass.set_pipeline(&fem_system.pipeline_clear);
                cpass.dispatch_workgroups(1, 1, 1);
            }
            queue.submit(Some(encoder.finish()));

            // Readback
            let result_nodes: Vec<GpuSoftBodyNode> = read_buffer(&device, &queue, &fem_system.nodes_buffer).await;
            
            // Verify gravity was applied correctly: mass (10.0) * gravity.y (-9.81) * 100000.0
            let expected_fy = (10.0 * -9.81 * 100000.0) as i32;
            
            assert_eq!(result_nodes[0].forces[0], 0);
            assert!((result_nodes[0].forces[1] - expected_fy).abs() <= 10, "Y force mismatch: got {}, expected {}", result_nodes[0].forces[1], expected_fy);
            assert_eq!(result_nodes[0].forces[2], 0);
        });
    }

    #[test]
    fn test_fem_compute_integration_and_collision() {
        pollster::block_on(async {
            let (device, queue) = setup_headless_gpu().await;

            // Düşen bir obje (Y=2.0) ve zemine geçmiş bir obje (Y=-1.0)
            let nodes = vec![
                GpuSoftBodyNode {
                    position_mass: [0.0, 2.0, 0.0, 1.0],
                    velocity_fixed: [0.0, -10.0, 0.0, 0.0],
                    forces: [0, 0, 0, 0], 
                },
                GpuSoftBodyNode {
                    position_mass: [0.0, -1.0, 0.0, 1.0], // Zeminin altında
                    velocity_fixed: [5.0, -10.0, 5.0, 0.0], // Hem aşağı hem yana gidiyor
                    forces: [0, 0, 0, 0],
                }
            ];
            
            // Boş eleman, sadece integrasyon test edilecek
            let elements = vec![GpuTetrahedron {
                indices: [0, 0, 0, 0],
                inv_rest_col0: [0.0; 4],
                inv_rest_col1: [0.0; 4],
                inv_rest_col2: [0.0; 4],
                rest_volume_pad: [0.0; 4],
            }];
            
            let params = GpuFemParams {
                properties: [0.1, 1.0, 1.0, 0.9], // dt=0.1, damping=0.9
                gravity: [0.0, 0.0, 0.0, 0.0], // Yerçekimi yok (sadece hızı test ediyoruz)
                counts: [2, 0, 0, 0],
            };

            let fem_system = GpuFemSystem::new(&device, &nodes, &elements, &[], &params);
            
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
            {
                let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: None,
                    timestamp_writes: None,
                });
                cpass.set_bind_group(0, &fem_system.compute_bind_group, &[]);
                cpass.set_pipeline(&fem_system.pipeline_integrate);
                cpass.dispatch_workgroups(1, 1, 1);
            }
            queue.submit(Some(encoder.finish()));

            let result_nodes: Vec<GpuSoftBodyNode> = read_buffer(&device, &queue, &fem_system.nodes_buffer).await;
            
            // --- Node 0 Test (Serbest Düşüş) ---
            let n0 = &result_nodes[0];
            // Beklenen hız: v = v * damping = -10.0 * 0.9 = -9.0
            assert!((n0.velocity_fixed[1] - (-9.0)).abs() < 0.001);
            // Beklenen pos: p = p + v * dt = 2.0 + (-9.0 * 0.1) = 1.1
            assert!((n0.position_mass[1] - 1.1).abs() < 0.001);

            // --- Node 1 Test (Zemin Çarpışması) ---
            let n1 = &result_nodes[1];
            // İlk hız Y=-10. Çarpışmadan dolayı: v.y *= -0.5 => 5.0. Sonra damping => 5.0 * 0.9 = 4.5
            // Wait: shader önce damping yapıyor mu, yoksa çarptıktan sonra mı?
            // Shader: velocity *= damping. future_pos = pos + velocity * dt. 
            // Eğer Y < 0 ise velocity.y *= -0.5, velocity.x *= 0.9.
            // Y= -1.0, velocity.y = -10.0 * 0.9 = -9.0.
            // future_pos = -1.0 + (-9.0 * 0.1) = -1.9 < 0.
            // velocity.y *= -0.5 => 4.5.
            assert!((n1.velocity_fixed[1] - 4.5).abs() < 0.001);
            
            // X ve Z için sürtünme test: velocity.x = (5.0 * 0.9) * 0.9 = 4.05
            assert!((n1.velocity_fixed[0] - 4.05).abs() < 0.001);
            
            // Pozisyon Y = 0.0 olmalı (zemine sıfırlanmalı)
            assert_eq!(n1.position_mass[1], 0.0);
        });
    }

    #[test]
    fn test_fem_compute_stress() {
        pollster::block_on(async {
            let (device, queue) = setup_headless_gpu().await;

            // 1 Tetrahedron. Rest pozisyonunda bir küpün köşesi gibi (dik üçgen piramit).
            // P0 = (0,0,0), P1 = (1,0,0), P2 = (0,1,0), P3 = (0,0,1)
            let nodes = vec![
                GpuSoftBodyNode { position_mass: [0.0, 0.0, 0.0, 1.0], velocity_fixed: [0.0; 4], forces: [0; 4] },
                GpuSoftBodyNode { position_mass: [2.0, 0.0, 0.0, 1.0], velocity_fixed: [0.0; 4], forces: [0; 4] }, // X yönünde 2 kat uzamış (Deforme olmuş!)
                GpuSoftBodyNode { position_mass: [0.0, 1.0, 0.0, 1.0], velocity_fixed: [0.0; 4], forces: [0; 4] },
                GpuSoftBodyNode { position_mass: [0.0, 0.0, 1.0, 1.0], velocity_fixed: [0.0; 4], forces: [0; 4] },
            ];

            // Dm (Rest Matrix)
            // e1 = (1,0,0), e2 = (0,1,0), e3 = (0,0,1)
            // Dm = Identity. Dm^-1 = Identity.
            let elements = vec![GpuTetrahedron {
                indices: [0, 1, 2, 3],
                inv_rest_col0: [1.0, 0.0, 0.0, 0.0],
                inv_rest_col1: [0.0, 1.0, 0.0, 0.0],
                inv_rest_col2: [0.0, 0.0, 1.0, 0.0],
                rest_volume_pad: [1.0 / 6.0, 0.0, 0.0, 0.0], // Volume
            }];
            
            let params = GpuFemParams {
                properties: [0.1, 1000.0, 1000.0, 1.0], // dt, mu, lambda
                gravity: [0.0, 0.0, 0.0, 0.0],
                counts: [4, 1, 0, 0],
            };

            let fem_system = GpuFemSystem::new(&device, &nodes, &elements, &[], &params);
            
            let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
            {
                let mut cpass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                    label: None,
                    timestamp_writes: None,
                });
                cpass.set_bind_group(0, &fem_system.compute_bind_group, &[]);
                cpass.set_pipeline(&fem_system.pipeline_stress);
                cpass.dispatch_workgroups(1, 1, 1);
            }
            queue.submit(Some(encoder.finish()));

            let result_nodes: Vec<GpuSoftBodyNode> = read_buffer(&device, &queue, &fem_system.nodes_buffer).await;
            
            // X yönünde uzama olduğu için, P1 (Node 1) geri çekilmek istenmeli (Negatif X kuvveti)
            // P0 (Node 0) sağa çekilmek istenmeli (Pozitif X kuvveti)
            let f1_x = result_nodes[1].forces[0];
            let f0_x = result_nodes[0].forces[0];

            assert!(f1_x < 0, "Node 1 should feel restorative force in -X direction. Got {}", f1_x);
            assert!(f0_x > 0, "Node 0 should feel restorative force in +X direction. Got {}", f0_x);
            
            // Dengede kalmalı, tüm kuvvetlerin toplamı ~0 olmalı (float hassasiyeti)
            let sum_fx = result_nodes[0].forces[0] + result_nodes[1].forces[0] + result_nodes[2].forces[0] + result_nodes[3].forces[0];
            assert!(sum_fx.abs() <= 10, "Forces must sum up to zero for equilibrium");
        });
    }
}
