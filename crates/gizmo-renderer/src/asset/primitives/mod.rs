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
            !verts.is_empty() && verts.len() % 3 == 0,
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
