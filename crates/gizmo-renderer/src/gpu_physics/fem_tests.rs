#[cfg(test)]
mod tests {
    use crate::gpu_physics::fem::{GpuFemParams, GpuFemSystem, GpuSoftBodyNode, GpuTetrahedron};

    // Helper to setup a headless wgpu device
    async fn setup_headless_gpu() -> Option<(wgpu::Device, wgpu::Queue)> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            flags: wgpu::InstanceFlags::default(),
            memory_budget_thresholds: Default::default(),
            backend_options: Default::default(),
            display: None,
        });

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .ok()?;

        adapter
            .request_device(&wgpu::DeviceDescriptor::default())
            .await
            .ok()
    }

    // Helper to read back a buffer
    async fn read_buffer<T: bytemuck::Pod>(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        buffer: &wgpu::Buffer,
    ) -> Vec<T> {
        let size = buffer.size();
        let staging_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Test Staging Buffer"),
            size,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mut encoder =
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
        encoder.copy_buffer_to_buffer(buffer, 0, &staging_buffer, 0, size);
        queue.submit(Some(encoder.finish()));

        let buffer_slice = staging_buffer.slice(..);
        let (sender, receiver) = std::sync::mpsc::channel();
        buffer_slice.map_async(wgpu::MapMode::Read, move |v| sender.send(v).unwrap());

        let _ = device.poll(wgpu::PollType::Wait {
            submission_index: None,
            timeout: None,
        });
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
        assert_eq!(
            std::mem::size_of::<GpuSoftBodyNode>(),
            48,
            "Node size must be 48 bytes"
        );
        assert_eq!(
            std::mem::size_of::<GpuTetrahedron>(),
            80,
            "Tetrahedron size must be 80 bytes"
        );
        assert_eq!(
            std::mem::size_of::<GpuFemParams>(),
            48,
            "FEM Params size must be 48 bytes"
        );
    }

    #[test]
    fn test_fem_compute_clear_forces() {
        pollster::block_on(async {
            let Some((device, queue)) = setup_headless_gpu().await else {
                tracing::info!("Skipping GPU test: no wgpu adapter found");
                return;
            };

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
            let mut encoder =
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
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
            let result_nodes: Vec<GpuSoftBodyNode> =
                read_buffer(&device, &queue, &fem_system.nodes_buffer).await;

            // Verify gravity was applied correctly: mass (10.0) * gravity.y (-9.81) * 100000.0
            let expected_fy = (10.0 * -9.81 * 100000.0) as i32;

            assert_eq!(result_nodes[0].forces[0], 0);
            assert!(
                (result_nodes[0].forces[1] - expected_fy).abs() <= 10,
                "Y force mismatch: got {}, expected {}",
                result_nodes[0].forces[1],
                expected_fy
            );
            assert_eq!(result_nodes[0].forces[2], 0);
        });
    }

    #[test]
    fn test_fem_compute_integration_and_collision() {
        pollster::block_on(async {
            let Some((device, queue)) = setup_headless_gpu().await else {
                tracing::info!("Skipping GPU test: no wgpu adapter found");
                return;
            };

            // Düşen bir obje (Y=2.0) ve zemine geçmiş bir obje (Y=-1.0)
            let nodes = vec![
                GpuSoftBodyNode {
                    position_mass: [0.0, 2.0, 0.0, 1.0],
                    velocity_fixed: [0.0, -10.0, 0.0, 0.0],
                    forces: [0, 0, 0, 0],
                },
                GpuSoftBodyNode {
                    position_mass: [0.0, -1.0, 0.0, 1.0],   // Zeminin altında
                    velocity_fixed: [5.0, -10.0, 5.0, 0.0], // Hem aşağı hem yana gidiyor
                    forces: [0, 0, 0, 0],
                },
            ];

            // Boş eleman, sadece integrasyon test edilecek
            let elements = vec![GpuTetrahedron {
                indices: [0, 0, 0, 0],
                inv_rest_col0: [0.0; 4],
                inv_rest_col1: [0.0; 4],
                inv_rest_col2: [0.0; 4],
                rest_volume_pad: [0.0; 4],
            }];

            // Y=0 zemin düzlemi collider'ı ekliyoruz
            use crate::gpu_physics::fem::GpuFemCollider;
            let colliders = vec![GpuFemCollider {
                shape_type: 0, // Plane
                radius: 0.0,
                _pad0: 0,
                _pad1: 0,
                position: [0.0, 0.0, 0.0, 0.0], // Düzlem üzerindeki bir nokta
                normal: [0.0, 1.0, 0.0, 0.0],    // Yukarı bakan normal
            }];

            let params = GpuFemParams {
                properties: [0.1, 1.0, 1.0, 0.9], // dt=0.1, damping=0.9
                gravity: [0.0, 0.0, 0.0, 0.0],    // Yerçekimi yok (sadece hızı test ediyoruz)
                counts: [2, 0, 1, 0],              // 2 node, 0 element, 1 collider
            };

            let fem_system = GpuFemSystem::new(&device, &nodes, &elements, &colliders, &params);

            let mut encoder =
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
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

            let result_nodes: Vec<GpuSoftBodyNode> =
                read_buffer(&device, &queue, &fem_system.nodes_buffer).await;

            // --- Node 0 Test (Serbest Düşüş, Y=2.0 → zeminin üstünde) ---
            let n0 = &result_nodes[0];
            // Beklenen: velocity *= damping → -10.0 * 0.9 = -9.0
            assert!((n0.velocity_fixed[1] - (-9.0)).abs() < 0.001,
                "Node 0 velocity Y: expected -9.0, got {}", n0.velocity_fixed[1]);
            // Beklenen: pos = 2.0 + (-9.0 * 0.1) = 1.1
            assert!((n0.position_mass[1] - 1.1).abs() < 0.001,
                "Node 0 position Y: expected 1.1, got {}", n0.position_mass[1]);

            // --- Node 1 Test (Zemin Çarpışması, Y=-1.0) ---
            let n1 = &result_nodes[1];
            // Shader akışı:
            //   velocity *= damping → (5*0.9, -10*0.9, 5*0.9) = (4.5, -9.0, 4.5)
            //   future_pos = (-1) + (-9.0 * 0.1) = -1.9 (< 0 → çarpışma!)
            //   future_pos.y -= normal * dist = -1.9 - (0,1,0) * (-1.9) = 0.0
            //   v_dot_n = -9.0 < 0 → çarpışma yanıtı:
            //     normal_vel = (0, -9.0, 0)
            //     tangent_vel = (4.5, 0, 4.5)
            //     velocity = tangent * 0.8 - normal * 0.2 = (3.6, 1.8, 3.6)
            assert!((n1.velocity_fixed[1] - 1.8).abs() < 0.01,
                "Node 1 velocity Y: expected 1.8 (bounce), got {}", n1.velocity_fixed[1]);
            assert!((n1.velocity_fixed[0] - 3.6).abs() < 0.01,
                "Node 1 velocity X: expected 3.6 (friction), got {}", n1.velocity_fixed[0]);
            assert!((n1.velocity_fixed[2] - 3.6).abs() < 0.01,
                "Node 1 velocity Z: expected 3.6 (friction), got {}", n1.velocity_fixed[2]);
            // Pozisyon Y = 0.0 olmalı (zemine sıfırlanmalı)
            assert!((n1.position_mass[1] - 0.0).abs() < 0.01,
                "Node 1 position Y: expected 0.0, got {}", n1.position_mass[1]);
        });
    }

    #[test]
    fn test_fem_compute_stress() {
        pollster::block_on(async {
            let Some((device, queue)) = setup_headless_gpu().await else {
                tracing::info!("Skipping GPU test: no wgpu adapter found");
                return;
            };

            // 1 Tetrahedron. Rest pozisyonunda bir küpün köşesi gibi (dik üçgen piramit).
            // P0 = (0,0,0), P1 = (1,0,0), P2 = (0,1,0), P3 = (0,0,1)
            let nodes = vec![
                GpuSoftBodyNode {
                    position_mass: [0.0, 0.0, 0.0, 1.0],
                    velocity_fixed: [0.0; 4],
                    forces: [0; 4],
                },
                GpuSoftBodyNode {
                    position_mass: [2.0, 0.0, 0.0, 1.0],
                    velocity_fixed: [0.0; 4],
                    forces: [0; 4],
                }, // X yönünde 2 kat uzamış (Deforme olmuş!)
                GpuSoftBodyNode {
                    position_mass: [0.0, 1.0, 0.0, 1.0],
                    velocity_fixed: [0.0; 4],
                    forces: [0; 4],
                },
                GpuSoftBodyNode {
                    position_mass: [0.0, 0.0, 1.0, 1.0],
                    velocity_fixed: [0.0; 4],
                    forces: [0; 4],
                },
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

            let mut encoder =
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
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

            let result_nodes: Vec<GpuSoftBodyNode> =
                read_buffer(&device, &queue, &fem_system.nodes_buffer).await;

            // X yönünde uzama olduğu için, P1 (Node 1) geri çekilmek istenmeli (Negatif X kuvveti)
            // P0 (Node 0) sağa çekilmek istenmeli (Pozitif X kuvveti)
            let f1_x = result_nodes[1].forces[0];
            let f0_x = result_nodes[0].forces[0];

            assert!(
                f1_x < 0,
                "Node 1 should feel restorative force in -X direction. Got {}",
                f1_x
            );
            assert!(
                f0_x > 0,
                "Node 0 should feel restorative force in +X direction. Got {}",
                f0_x
            );

            // Dengede kalmalı, tüm kuvvetlerin toplamı ~0 olmalı (float hassasiyeti)
            let sum_fx = result_nodes[0].forces[0]
                + result_nodes[1].forces[0]
                + result_nodes[2].forces[0]
                + result_nodes[3].forces[0];
            assert!(
                sum_fx.abs() <= 10,
                "Forces must sum up to zero for equilibrium"
            );
        });
    }

    // Regression for the box_contacts under-allocation (physics_compute.wgsl
    // `BoxContacts`). The WGSL std430 element stride is 352 bytes, not 336 — the
    // 4-byte alignment gap before `normals` (array<vec4<f32>,8>) and the 12-byte
    // gap after `count` were previously omitted, under-allocating the buffer by
    // 16 B/box so high-index bodies indexed out of bounds and got no contacts.
    #[test]
    fn test_box_contacts_std430_size() {
        assert_eq!(
            std::mem::size_of::<crate::gpu_physics::types::GpuBoxContacts>(),
            352,
            "BoxContacts std430 stride must be 352 bytes (16-byte alignment for the \
             vec4 arrays); a smaller value under-allocates box_contacts_buffer"
        );
    }

    // Regression for the FEM inversion (det F < 0) handling. The old code clamped
    // J with `max(J, 0.01)` — forcing it positive — which flipped the sign (and
    // 100x-inflated the magnitude) of F^-T = cofactor(F)/J for inverted elements,
    // so the Piola stress pointed the wrong way and overflowed the fixed-point
    // accumulator. The sign-preserving clamp keeps F^-T correctly oriented.
    #[test]
    fn test_fem_inverted_element_signed_stress() {
        pollster::block_on(async {
            let Some((device, queue)) = setup_headless_gpu().await else {
                tracing::info!("Skipping GPU test: no wgpu adapter found");
                return;
            };

            // Same rest tet as test_fem_compute_stress (Dm = I) but node 1 is
            // pushed to x = -0.5, INVERTING the element (F = diag(-0.5, 1, 1),
            // J = -0.5 < 0).
            let nodes = vec![
                GpuSoftBodyNode {
                    position_mass: [0.0, 0.0, 0.0, 1.0],
                    velocity_fixed: [0.0; 4],
                    forces: [0; 4],
                },
                GpuSoftBodyNode {
                    position_mass: [-0.5, 0.0, 0.0, 1.0],
                    velocity_fixed: [0.0; 4],
                    forces: [0; 4],
                },
                GpuSoftBodyNode {
                    position_mass: [0.0, 1.0, 0.0, 1.0],
                    velocity_fixed: [0.0; 4],
                    forces: [0; 4],
                },
                GpuSoftBodyNode {
                    position_mass: [0.0, 0.0, 1.0, 1.0],
                    velocity_fixed: [0.0; 4],
                    forces: [0; 4],
                },
            ];
            let elements = vec![GpuTetrahedron {
                indices: [0, 1, 2, 3],
                inv_rest_col0: [1.0, 0.0, 0.0, 0.0],
                inv_rest_col1: [0.0, 1.0, 0.0, 0.0],
                inv_rest_col2: [0.0, 0.0, 1.0, 0.0],
                rest_volume_pad: [1.0 / 6.0, 0.0, 0.0, 0.0],
            }];
            let params = GpuFemParams {
                properties: [0.1, 1000.0, 1000.0, 1.0], // dt, mu, lambda, damping
                gravity: [0.0, 0.0, 0.0, 0.0],
                counts: [4, 1, 0, 0],
            };

            let fem_system = GpuFemSystem::new(&device, &nodes, &elements, &[], &params);
            let mut encoder =
                device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
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

            let result: Vec<GpuSoftBodyNode> =
                read_buffer(&device, &queue, &fem_system.nodes_buffer).await;

            let f1_x = result[1].forces[0];
            // With the correct signed-J, F^-T = diag(-2,1,1) and the node-1 force
            // is small and NEGATIVE-x (~ -481 N → ~-4.8e7 encoded). The old
            // positive clamp produced F^-T = cofactor/0.01, giving f1_x ≈ +93500 N
            // (→ +9.35e9, which overflowed the i32 to a large POSITIVE saturated
            // value). So the SIGN cleanly discriminates the fix.
            assert!(
                f1_x < 0,
                "inverted element: node-1 force_x must be negative with signed-J \
                 (got {}); a positive value means the old max(J,0.01) sign flip",
                f1_x
            );
            // And every force stays comfortably inside the i32 fixed-point range —
            // the old /0.01 blow-up saturated it.
            for (n, node) in result.iter().enumerate() {
                for (c, &f) in node.forces.iter().enumerate() {
                    assert!(
                        f.abs() < 1_000_000_000,
                        "force[{}][{}] = {} should be bounded (no /0.01 blow-up)",
                        n,
                        c,
                        f
                    );
                }
            }
        });
    }
}
