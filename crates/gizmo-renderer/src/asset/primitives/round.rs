use crate::components::Mesh;
use crate::renderer::Vertex;
use gizmo_math::Vec3;
use std::sync::Arc;
use wgpu::util::DeviceExt;

impl crate::asset::AssetManager {
    /// Programatik UV Küre (Sphere) üretir.
    /// UV-küre köşeleri (dış-yüzey CCW sarımlı, kutup dejenereleri atlanmış). Saf veri.
    pub(crate) fn sphere_data(radius: f32, stacks: u32, slices: u32) -> Vec<Vertex> {
        let stacks = stacks.max(3);
        let slices = slices.max(3);
        let mut vertices = Vec::new();
        let pi = std::f32::consts::PI;

        for i in 0..stacks {
            let theta1 = (i as f32 / stacks as f32) * pi;
            let theta2 = ((i + 1) as f32 / stacks as f32) * pi;

            for j in 0..slices {
                let phi1 = (j as f32 / slices as f32) * 2.0 * pi;
                let phi2 = ((j + 1) as f32 / slices as f32) * 2.0 * pi;

                // 4 köşe noktası
                let p1 = [
                    radius * theta1.sin() * phi1.cos(),
                    radius * theta1.cos(),
                    radius * theta1.sin() * phi1.sin(),
                ];
                let p2 = [
                    radius * theta2.sin() * phi1.cos(),
                    radius * theta2.cos(),
                    radius * theta2.sin() * phi1.sin(),
                ];
                let p3 = [
                    radius * theta2.sin() * phi2.cos(),
                    radius * theta2.cos(),
                    radius * theta2.sin() * phi2.sin(),
                ];
                let p4 = [
                    radius * theta1.sin() * phi2.cos(),
                    radius * theta1.cos(),
                    radius * theta1.sin() * phi2.sin(),
                ];

                let n1 = [
                    theta1.sin() * phi1.cos(),
                    theta1.cos(),
                    theta1.sin() * phi1.sin(),
                ];
                let n2 = [
                    theta2.sin() * phi1.cos(),
                    theta2.cos(),
                    theta2.sin() * phi1.sin(),
                ];
                let n3 = [
                    theta2.sin() * phi2.cos(),
                    theta2.cos(),
                    theta2.sin() * phi2.sin(),
                ];
                let n4 = [
                    theta1.sin() * phi2.cos(),
                    theta1.cos(),
                    theta1.sin() * phi2.sin(),
                ];

                let uv1 = [
                    if i == 0 {
                        (j as f32 + 0.5) / slices as f32
                    } else {
                        j as f32 / slices as f32
                    },
                    i as f32 / stacks as f32,
                ];
                let uv2 = [
                    if i + 1 == stacks {
                        (j as f32 + 0.5) / slices as f32
                    } else {
                        j as f32 / slices as f32
                    },
                    (i + 1) as f32 / stacks as f32,
                ];
                let uv3 = [
                    if i + 1 == stacks {
                        (j as f32 + 0.5) / slices as f32
                    } else {
                        (j + 1) as f32 / slices as f32
                    },
                    (i + 1) as f32 / stacks as f32,
                ];
                let uv4 = [
                    if i == 0 {
                        (j as f32 + 0.5) / slices as f32
                    } else {
                        (j + 1) as f32 / slices as f32
                    },
                    i as f32 / stacks as f32,
                ];

                let def_j = [0; 4];
                let def_w = [0.0; 4];
                let vtx = |position: [f32; 3], normal: [f32; 3], tex_coords: [f32; 2]| Vertex {
                    position,
                    color: [1.0; 3],
                    normal,
                    tex_coords,
                    joint_indices: def_j,
                    joint_weights: def_w,
                    ..Default::default()
                };

                // Sağ-el (CCW dıştan) sarım: geometrik normal = dış yüzey normali, böylece
                // Ccw+Back-cull pipeline'ında küre dışarıdan görünür. (Eskiden p1,p2,p3 /
                // p1,p3,p4 sırası geometrik normali İÇE veriyordu → küre içi-dışına culllanıyordu.)
                // Ayrıca kutup satırlarında iki köşesi çakışan DEJENERE üçgen atlanıyor.

                // Üçgen 1: p1 → p3 → p2  (güney kutbu satırında p2==p3 → dejenere)
                if i != stacks - 1 {
                    vertices.push(vtx(p1, n1, uv1));
                    vertices.push(vtx(p3, n3, uv3));
                    vertices.push(vtx(p2, n2, uv2));
                }
                // Üçgen 2: p1 → p4 → p3  (kuzey kutbu satırında p1==p4 → dejenere)
                if i != 0 {
                    vertices.push(vtx(p1, n1, uv1));
                    vertices.push(vtx(p4, n4, uv4));
                    vertices.push(vtx(p3, n3, uv3));
                }
            }
        }

        vertices
    }

    pub fn create_sphere(device: &wgpu::Device, radius: f32, stacks: u32, slices: u32) -> Mesh {
        let vertices = Self::sphere_data(radius, stacks, slices);
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Sphere VBuf"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        Mesh::new(
            device,
            Arc::new(vbuf),
            &vertices,
            Vec3::ZERO,
            format!("sphere_{}_{}_{}", radius, stacks, slices),
        )
    }

    /// Silindir köşeleri (yan + iki kapak), dış-yüzey CCW sarımlı. Saf veri.
    pub(crate) fn cylinder_data(radius: f32, height: f32, radial_segments: u32) -> Vec<Vertex> {
        let radial_segments = radial_segments.max(3);
        let mut vertices = Vec::new();
        let pi = std::f32::consts::PI;
        let half_h = height / 2.0;
        let def_j = [0; 4]; let def_w = [0.0; 4]; let col = [1.0; 3];
        let vtx = |position: [f32; 3], normal: [f32; 3], tex_coords: [f32; 2]| Vertex {
            position, color: col, normal, tex_coords, joint_indices: def_j, joint_weights: def_w, ..Default::default()
        };

        // Tube
        for i in 0..radial_segments {
            let t1 = (i as f32 / radial_segments as f32) * 2.0 * pi;
            let t2 = ((i + 1) as f32 / radial_segments as f32) * 2.0 * pi;

            let u1 = i as f32 / radial_segments as f32;
            let u2 = (i + 1) as f32 / radial_segments as f32;

            let p1_top = [radius * t1.cos(), half_h, radius * t1.sin()];
            let p1_bot = [radius * t1.cos(), -half_h, radius * t1.sin()];
            let p2_top = [radius * t2.cos(), half_h, radius * t2.sin()];
            let p2_bot = [radius * t2.cos(), -half_h, radius * t2.sin()];

            let n1 = [t1.cos(), 0.0, t1.sin()];
            let n2 = [t2.cos(), 0.0, t2.sin()];

            // Yan yüzler: dıştan CCW (sağ-el normali dışa). (Eskiden ters → culllanıyordu.)
            // Tri 1: p1_top → p2_bot → p1_bot
            vertices.push(vtx(p1_top, n1, [u1, 0.0]));
            vertices.push(vtx(p2_bot, n2, [u2, 1.0]));
            vertices.push(vtx(p1_bot, n1, [u1, 1.0]));
            // Tri 2: p1_top → p2_top → p2_bot
            vertices.push(vtx(p1_top, n1, [u1, 0.0]));
            vertices.push(vtx(p2_top, n2, [u2, 0.0]));
            vertices.push(vtx(p2_bot, n2, [u2, 1.0]));

            // Üst kapak: center → p2_top → p1_top (sağ-el normali +Y).
            vertices.push(vtx([0.0, half_h, 0.0], [0.0, 1.0, 0.0], [0.5, 0.5]));
            vertices.push(vtx(p2_top, [0.0, 1.0, 0.0], [0.5 + 0.5 * t2.cos(), 0.5 + 0.5 * t2.sin()]));
            vertices.push(vtx(p1_top, [0.0, 1.0, 0.0], [0.5 + 0.5 * t1.cos(), 0.5 + 0.5 * t1.sin()]));

            // Alt kapak: center → p1_bot → p2_bot (sağ-el normali -Y).
            vertices.push(vtx([0.0, -half_h, 0.0], [0.0, -1.0, 0.0], [0.5, 0.5]));
            vertices.push(vtx(p1_bot, [0.0, -1.0, 0.0], [0.5 + 0.5 * t1.cos(), 0.5 + 0.5 * t1.sin()]));
            vertices.push(vtx(p2_bot, [0.0, -1.0, 0.0], [0.5 + 0.5 * t2.cos(), 0.5 + 0.5 * t2.sin()]));
        }
        vertices
    }

    pub fn create_cylinder(device: &wgpu::Device, radius: f32, height: f32, radial_segments: u32) -> Mesh {
        let vertices = Self::cylinder_data(radius, height, radial_segments);
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("Cylinder VBuf"), contents: bytemuck::cast_slice(&vertices), usage: wgpu::BufferUsages::VERTEX });
        Mesh::new(device, Arc::new(vbuf), &vertices, Vec3::ZERO, format!("cylinder_{}_{}", radius, height))
    }

    /// Koni köşeleri (yan + taban), dış-yüzey CCW sarımlı. Saf veri.
    pub(crate) fn cone_data(radius: f32, height: f32, radial_segments: u32) -> Vec<Vertex> {
        let radial_segments = radial_segments.max(3);
        let mut vertices = Vec::new();
        let pi = std::f32::consts::PI;
        let half_h = height / 2.0;
        let def_j = [0; 4]; let def_w = [0.0; 4]; let col = [1.0; 3];
        let vtx = |position: [f32; 3], normal: [f32; 3], tex_coords: [f32; 2]| Vertex {
            position, color: col, normal, tex_coords, joint_indices: def_j, joint_weights: def_w, ..Default::default()
        };

        let slant = (radius * radius + height * height).sqrt();
        let ny = radius / slant;
        let n_xz = height / slant;

        for i in 0..radial_segments {
            let t1 = (i as f32 / radial_segments as f32) * 2.0 * pi;
            let t2 = ((i + 1) as f32 / radial_segments as f32) * 2.0 * pi;

            let p1_bot = [radius * t1.cos(), -half_h, radius * t1.sin()];
            let p2_bot = [radius * t2.cos(), -half_h, radius * t2.sin()];
            let apex = [0.0, half_h, 0.0];

            let n1 = [n_xz * t1.cos(), ny, n_xz * t1.sin()];
            let n2 = [n_xz * t2.cos(), ny, n_xz * t2.sin()];
            let navg = [n_xz * ((t1+t2)/2.0).cos(), ny, n_xz * ((t1+t2)/2.0).sin()];

            let u1 = i as f32 / radial_segments as f32;
            let u2 = (i + 1) as f32 / radial_segments as f32;
            let umid = (u1 + u2) / 2.0;

            // Yan üçgen: apex → p2_bot → p1_bot (dıştan CCW). (Eskiden ters → culllanıyordu.)
            vertices.push(vtx(apex, navg, [umid, 0.0]));
            vertices.push(vtx(p2_bot, n2, [u2, 1.0]));
            vertices.push(vtx(p1_bot, n1, [u1, 1.0]));

            // Taban kapağı: center → p1_bot → p2_bot (sağ-el normali -Y).
            vertices.push(vtx([0.0, -half_h, 0.0], [0.0, -1.0, 0.0], [0.5, 0.5]));
            vertices.push(vtx(p1_bot, [0.0, -1.0, 0.0], [0.5 + 0.5 * t1.cos(), 0.5 + 0.5 * t1.sin()]));
            vertices.push(vtx(p2_bot, [0.0, -1.0, 0.0], [0.5 + 0.5 * t2.cos(), 0.5 + 0.5 * t2.sin()]));
        }
        vertices
    }

    pub fn create_cone(device: &wgpu::Device, radius: f32, height: f32, radial_segments: u32) -> Mesh {
        let vertices = Self::cone_data(radius, height, radial_segments);
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("Cone VBuf"), contents: bytemuck::cast_slice(&vertices), usage: wgpu::BufferUsages::VERTEX });
        Mesh::new(device, Arc::new(vbuf), &vertices, Vec3::ZERO, format!("cone_{}_{}", radius, height))
    }

    pub fn create_torus(device: &wgpu::Device, radius: f32, tube_radius: f32, radial_segments: u32, tubular_segments: u32) -> Mesh {
        let radial_segments = radial_segments.max(3);
        let tubular_segments = tubular_segments.max(3);
        let mut vertices = Vec::new();
        let pi = std::f32::consts::PI;

        for i in 0..radial_segments {
            for j in 0..tubular_segments {
                let u1 = i as f32 / radial_segments as f32;
                let u2 = (i + 1) as f32 / radial_segments as f32;
                let v1 = j as f32 / tubular_segments as f32;
                let v2 = (j + 1) as f32 / tubular_segments as f32;

                let t1 = u1 * 2.0 * pi;
                let t2 = u2 * 2.0 * pi;
                let p1 = v1 * 2.0 * pi;
                let p2 = v2 * 2.0 * pi;

                let pos = |t: f32, p: f32| {
                    [(radius + tube_radius * p.cos()) * t.cos(), tube_radius * p.sin(), (radius + tube_radius * p.cos()) * t.sin()]
                };
                let norm = |t: f32, p: f32| {
                    [p.cos() * t.cos(), p.sin(), p.cos() * t.sin()]
                };

                let p_00 = pos(t1, p1); let n_00 = norm(t1, p1);
                let p_10 = pos(t2, p1); let n_10 = norm(t2, p1);
                let p_01 = pos(t1, p2); let n_01 = norm(t1, p2);
                let p_11 = pos(t2, p2); let n_11 = norm(t2, p2);

                let def_j = [0; 4]; let def_w = [0.0; 4];
                let col = [1.0; 3];

                vertices.push(Vertex { position: p_00, normal: n_00, tex_coords: [u1, v1], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
                vertices.push(Vertex { position: p_01, normal: n_01, tex_coords: [u1, v2], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
                vertices.push(Vertex { position: p_10, normal: n_10, tex_coords: [u2, v1], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });

                vertices.push(Vertex { position: p_10, normal: n_10, tex_coords: [u2, v1], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
                vertices.push(Vertex { position: p_01, normal: n_01, tex_coords: [u1, v2], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
                vertices.push(Vertex { position: p_11, normal: n_11, tex_coords: [u2, v2], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
            }
        }

        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("Torus VBuf"), contents: bytemuck::cast_slice(&vertices), usage: wgpu::BufferUsages::VERTEX });
        Mesh::new(device, Arc::new(vbuf), &vertices, Vec3::ZERO, format!("torus_{}_{}", radius, tube_radius))
    }

    /// Kapsül köşeleri (tüp + iki yarıküre), dış-yüzey CCW sarımlı. Saf veri.
    pub(crate) fn capsule_data(radius: f32, depth: f32, latitudes: u32, longitudes: u32) -> Vec<Vertex> {
        let latitudes = latitudes.max(4);
        let longitudes = longitudes.max(4);
        let mut vertices = Vec::new();
        let pi = std::f32::consts::PI;
        let half_d = depth / 2.0;

        for i in 0..=latitudes {
            let u1 = i as f32 / latitudes as f32;
            let u2 = (i + 1) as f32 / latitudes as f32;
            let theta1 = u1 * pi;
            let theta2 = u2 * pi;

            let y_offset1 = if u1 < 0.5 { half_d } else if u1 > 0.5 { -half_d } else { 0.0 };
            let y_offset2 = if u2 < 0.5 { half_d } else if u2 > 0.5 { -half_d } else { 0.0 };
            
            // To properly insert a tube, we duplicate the equator loop
            let is_equator = i == latitudes / 2;

            if is_equator {
                // Tube segment
                for j in 0..longitudes {
                    let v1 = j as f32 / longitudes as f32;
                    let v2 = (j + 1) as f32 / longitudes as f32;
                    let phi1 = v1 * 2.0 * pi;
                    let phi2 = v2 * 2.0 * pi;

                    let p1_top = [radius * phi1.cos(), half_d, radius * phi1.sin()];
                    let p1_bot = [radius * phi1.cos(), -half_d, radius * phi1.sin()];
                    let p2_top = [radius * phi2.cos(), half_d, radius * phi2.sin()];
                    let p2_bot = [radius * phi2.cos(), -half_d, radius * phi2.sin()];

                    let n1 = [phi1.cos(), 0.0, phi1.sin()];
                    let n2 = [phi2.cos(), 0.0, phi2.sin()];

                    let def_j = [0; 4]; let def_w = [0.0; 4]; let col = [1.0; 3];

                    // Tüp dıştan CCW (eskiden ters → culllanıyordu).
                    // Tri 1: p1_top → p2_bot → p1_bot
                    vertices.push(Vertex { position: p1_top, normal: n1, tex_coords: [v1, 0.5], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
                    vertices.push(Vertex { position: p2_bot, normal: n2, tex_coords: [v2, 0.5], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
                    vertices.push(Vertex { position: p1_bot, normal: n1, tex_coords: [v1, 0.5], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });

                    // Tri 2: p1_top → p2_top → p2_bot
                    vertices.push(Vertex { position: p1_top, normal: n1, tex_coords: [v1, 0.5], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
                    vertices.push(Vertex { position: p2_top, normal: n2, tex_coords: [v2, 0.5], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
                    vertices.push(Vertex { position: p2_bot, normal: n2, tex_coords: [v2, 0.5], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
                }
            }

            if i < latitudes {
                for j in 0..longitudes {
                    let v1 = j as f32 / longitudes as f32;
                    let v2 = (j + 1) as f32 / longitudes as f32;
                    let phi1 = v1 * 2.0 * pi;
                    let phi2 = v2 * 2.0 * pi;

                    let p1 = [radius * theta1.sin() * phi1.cos(), radius * theta1.cos() + y_offset1, radius * theta1.sin() * phi1.sin()];
                    let p2 = [radius * theta2.sin() * phi1.cos(), radius * theta2.cos() + y_offset2, radius * theta2.sin() * phi1.sin()];
                    let p3 = [radius * theta2.sin() * phi2.cos(), radius * theta2.cos() + y_offset2, radius * theta2.sin() * phi2.sin()];
                    let p4 = [radius * theta1.sin() * phi2.cos(), radius * theta1.cos() + y_offset1, radius * theta1.sin() * phi2.sin()];

                    let n1 = [theta1.sin() * phi1.cos(), theta1.cos(), theta1.sin() * phi1.sin()];
                    let n2 = [theta2.sin() * phi1.cos(), theta2.cos(), theta2.sin() * phi1.sin()];
                    let n3 = [theta2.sin() * phi2.cos(), theta2.cos(), theta2.sin() * phi2.sin()];
                    let n4 = [theta1.sin() * phi2.cos(), theta1.cos(), theta1.sin() * phi2.sin()];

                    let def_j = [0; 4]; let def_w = [0.0; 4]; let col = [1.0; 3];

                    // Yarıküre bantları dıştan CCW (eskiden ters → culllanıyordu).
                    // Kutup satırlarında çakışan köşeli DEJENERE üçgen atlanır (sphere ile aynı).
                    // Tri 1: p1 → p3 → p2  (güney kutbu satırında p2==p3 → dejenere)
                    if i != latitudes - 1 {
                        vertices.push(Vertex { position: p1, normal: n1, tex_coords: [v1, u1], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
                        vertices.push(Vertex { position: p3, normal: n3, tex_coords: [v2, u2], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
                        vertices.push(Vertex { position: p2, normal: n2, tex_coords: [v1, u2], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
                    }

                    // Tri 2: p1 → p4 → p3  (kuzey kutbu satırında p1==p4 → dejenere)
                    if i != 0 {
                        vertices.push(Vertex { position: p1, normal: n1, tex_coords: [v1, u1], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
                        vertices.push(Vertex { position: p4, normal: n4, tex_coords: [v2, u1], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
                        vertices.push(Vertex { position: p3, normal: n3, tex_coords: [v2, u2], color: col, joint_indices: def_j, joint_weights: def_w, ..Default::default() });
                    }
                }
            }
        }

        vertices
    }

    pub fn create_capsule(device: &wgpu::Device, radius: f32, depth: f32, latitudes: u32, longitudes: u32) -> Mesh {
        let vertices = Self::capsule_data(radius, depth, latitudes, longitudes);
        let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some("Capsule VBuf"), contents: bytemuck::cast_slice(&vertices), usage: wgpu::BufferUsages::VERTEX });
        Mesh::new(device, Arc::new(vbuf), &vertices, Vec3::ZERO, format!("capsule_{}_{}", radius, depth))
    }

}
