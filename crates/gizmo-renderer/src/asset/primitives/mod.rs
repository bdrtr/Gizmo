//! Prosedürel mesh üreticileri (god-file Tier 3 round-2 bölmesi).
//! Tüm üreticiler `AssetManager` üzerinde inherent metod; kategori
//! başına ayrı `impl` blokları alt-modüllerde tutulur.

mod cuboid;
mod flat;
mod round;
mod terrain;

#[cfg(test)]
mod winding_tests {
    //! Prosedürel mesh'lerin üçgen sarımı (winding) ile declared yüzey normalleri
    //! TUTARLI olmalı: geometrik (sağ-el) normal, declared outward normal ile aynı
    //! yöne bakmalı (dot > 0). Aksi halde Ccw + Back-cull pipeline'ında (bkz.
    //! pipeline.rs:579-580, deferred.rs:336-337) yüzeyler back-face sayılıp culllanır
    //! → şekil "içi-dışına" / görünmez render olur. (Bu testin yakaladığı bug buydu.)
    //!
    //! Saf `*_data` fonksiyonları üzerinde çalışır — GPU device gerekmez.

    use crate::asset::AssetManager;
    use crate::gpu_types::Vertex;
    use gizmo_math::Vec3;

    /// Bir non-indexed üçgen listesinin sarım + normal geçerliliğini doğrula.
    fn assert_outward(name: &str, verts: &[Vertex]) {
        assert!(
            !verts.is_empty() && verts.len().is_multiple_of(3),
            "{name}: vertex sayısı pozitif ve 3'ün katı olmalı, {}",
            verts.len()
        );
        let mut checked = 0usize;
        for tri in verts.chunks_exact(3) {
            for v in tri {
                let n = Vec3::from(v.normal);
                assert!(
                    n.is_finite() && (n.length() - 1.0).abs() < 1e-3,
                    "{name}: normal birim/sonlu değil: {:?}",
                    v.normal
                );
            }
            let p0 = Vec3::from(tri[0].position);
            let p1 = Vec3::from(tri[1].position);
            let p2 = Vec3::from(tri[2].position);
            let geo = (p1 - p0).cross(p2 - p0);
            // Dejenere üçgenleri (ör. kapsül yarıküre kutupları) atla.
            if geo.length() < 1e-9 {
                continue;
            }
            let n_avg = Vec3::from(tri[0].normal) + Vec3::from(tri[1].normal) + Vec3::from(tri[2].normal);
            let d = geo.normalize().dot(n_avg.normalize());
            assert!(
                d > 0.0,
                "{name}: üçgen sarımı declared normal'in TERSİNE (geo·n={d} ≤ 0) → \
                 Ccw+Back-cull'da içi-dışına culllanır. p0={p0:?} p1={p1:?} p2={p2:?}"
            );
            checked += 1;
        }
        assert!(checked > 0, "{name}: dejenere olmayan üçgen yok");
    }

    #[test]
    fn plane_winding_faces_up() {
        assert_outward("plane", &AssetManager::plane_data(2.0));
    }

    #[test]
    fn circle_winding_faces_up() {
        assert_outward("circle", &AssetManager::circle_data(1.0, 16));
    }

    #[test]
    fn sphere_winding_is_outward() {
        for &(stacks, slices) in &[(3, 3), (8, 12), (16, 24)] {
            assert_outward(
                &format!("sphere({stacks},{slices})"),
                &AssetManager::sphere_data(1.5, stacks, slices),
            );
        }
    }

    #[test]
    fn sphere_has_no_degenerate_triangles() {
        // Kutup dejenereleri kaldırıldı: hiçbir üçgenin iki köşesi çakışmamalı.
        let v = AssetManager::sphere_data(1.0, 8, 12);
        for tri in v.chunks_exact(3) {
            let geo = (Vec3::from(tri[1].position) - Vec3::from(tri[0].position))
                .cross(Vec3::from(tri[2].position) - Vec3::from(tri[0].position));
            assert!(geo.length() > 1e-9, "sphere dejenere üçgen üretti");
        }
    }

    #[test]
    fn cylinder_winding_is_outward() {
        assert_outward("cylinder", &AssetManager::cylinder_data(1.0, 2.0, 16));
    }

    #[test]
    fn cone_winding_is_outward() {
        assert_outward("cone", &AssetManager::cone_data(1.0, 2.0, 16));
    }

    #[test]
    fn capsule_winding_is_outward() {
        assert_outward("capsule", &AssetManager::capsule_data(0.5, 1.0, 8, 12));
    }

    #[test]
    fn tetrahedron_winding_is_outward() {
        assert_outward("tetrahedron", &AssetManager::tetrahedron_data(1.0));
    }

    #[test]
    fn conical_frustum_winding_is_outward() {
        assert_outward(
            "conical_frustum",
            &AssetManager::conical_frustum_data(1.0, 0.5, 2.0, 16),
        );
    }
}

#[cfg(test)]
mod geometry_tests {
    //! Metric invariants for the procedural `*_data` generators: vertices must sit
    //! on the parametric surface (right radius / height / extent) and analytic
    //! normals must be unit-length and point outward. Pure CPU — no GPU device.

    use crate::asset::AssetManager;
    use gizmo_math::Vec3;

    fn radial(p: [f32; 3]) -> f32 {
        (p[0] * p[0] + p[2] * p[2]).sqrt()
    }

    #[test]
    fn sphere_vertices_lie_on_the_radius_with_outward_unit_normals() {
        let r = 2.5;
        let verts = AssetManager::sphere_data(r, 12, 20);
        assert!(!verts.is_empty() && verts.len().is_multiple_of(3));
        for v in &verts {
            let p = Vec3::from(v.position);
            assert!((p.length() - r).abs() < 1e-3, "vertex off the sphere: {:?}", v.position);
            let n = Vec3::from(v.normal);
            assert!((n.length() - 1.0).abs() < 1e-3, "normal not unit: {:?}", v.normal);
            // On a sphere the outward normal is the position direction.
            assert!(p.normalize().dot(n) > 0.99, "normal not outward at {:?}", v.position);
        }
    }

    #[test]
    fn sphere_clamps_degenerate_resolution() {
        // stacks/slices below 3 are raised to 3, still yielding whole triangles.
        let v = AssetManager::sphere_data(1.0, 0, 0);
        assert!(!v.is_empty() && v.len().is_multiple_of(3));
    }

    #[test]
    fn cylinder_fits_inside_its_radius_and_height_and_has_both_caps() {
        let (r, h) = (1.5, 4.0);
        let half = h / 2.0;
        let verts = AssetManager::cylinder_data(r, h, 24);
        for v in &verts {
            assert!(radial(v.position) <= r + 1e-3, "outside radius: {:?}", v.position);
            assert!(v.position[1].abs() <= half + 1e-3, "outside height: {:?}", v.position);
        }
        // Both end caps are generated (a vertex touches each extreme y).
        assert!(verts.iter().any(|v| (v.position[1] - half).abs() < 1e-4), "top cap missing");
        assert!(verts.iter().any(|v| (v.position[1] + half).abs() < 1e-4), "bottom cap missing");
    }

    #[test]
    fn cone_has_apex_at_top_and_base_within_radius() {
        let (r, h) = (1.0, 3.0);
        let half = h / 2.0;
        let verts = AssetManager::cone_data(r, h, 20);
        // Apex sits on the axis at +half_h.
        assert!(
            verts.iter().any(|v| (v.position[1] - half).abs() < 1e-4 && radial(v.position) < 1e-4),
            "apex not found at (0,{half},0)"
        );
        for v in &verts {
            assert!(v.position[1] <= half + 1e-4 && v.position[1] >= -half - 1e-4);
            assert!(radial(v.position) <= r + 1e-3);
        }
    }

    #[test]
    fn capsule_stays_within_radius_and_capped_height() {
        let (r, d) = (0.5, 2.0);
        let max_y = d / 2.0 + r; // hemisphere caps extend r past each tube end
        let verts = AssetManager::capsule_data(r, d, 8, 12);
        assert!(!verts.is_empty());
        for v in &verts {
            assert!(v.position[1].abs() <= max_y + 1e-3, "capsule too tall: {:?}", v.position);
            assert!(radial(v.position) <= r + 1e-3, "capsule too wide: {:?}", v.position);
        }
    }

    #[test]
    fn plane_and_circle_span_their_declared_extent_in_the_xz_plane() {
        let plane = AssetManager::plane_data(4.0); // spans [−2, 2] in x and z
        for v in &plane {
            assert!(v.position[1].abs() < 1e-6, "plane not flat: {:?}", v.position);
            assert!(v.position[0].abs() <= 2.0 + 1e-6 && v.position[2].abs() <= 2.0 + 1e-6);
        }
        assert!(plane.iter().any(|v| (v.position[0] - 2.0).abs() < 1e-6));
        assert!(plane.iter().any(|v| (v.position[0] + 2.0).abs() < 1e-6));

        let circle = AssetManager::circle_data(1.5, 16);
        for v in &circle {
            assert!(v.position[1].abs() < 1e-6, "circle not flat: {:?}", v.position);
            assert!(radial(v.position) <= 1.5 + 1e-4, "circle exceeds radius: {:?}", v.position);
        }
    }
}
