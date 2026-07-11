//! Genel amaçlı GÖRSEL DÖNME bileşeni + sistemi.
//!
//! Bir mesh'i (ECS `Transform`) sabit bir eksende, sabit bir açısal hızla döndürür.
//! Tekerlek, pervane, fan, türbin, gezegen, dönen platform... hepsi için tek çözüm —
//! artık demoların her frame elle `transform.rotation = ...` yazması GEREKMEZ. Bileşeni
//! ekle, [`SpinPlugin`]'i (veya doğrudan [`SpinSystem`]) çalıştır, motor döndürsün.
//!
//! ```ignore
//! world.add_component(wheel_mesh, Spin::new(Vec3::X, 30.0)); // 30 rad/s yuvarlanma
//! app.add_plugin(SpinPlugin);                                // otomatik döner
//! ```

use gizmo_core::world::World;
use gizmo_math::{Quat, Vec3};
use gizmo_physics_core::Transform;

/// Bir `Transform`'u `axis` ekseninde `angular_velocity` (rad/s) hızıyla döndürür.
/// Dönüş `rest_rotation`'ın (modelin yazar-duruşu) ÜZERİNE biner. `angular_velocity`
/// her frame değiştirilebilir (ör. tekerlek hızını araç hızına bağla).
#[derive(Debug, Clone, Copy)]
pub struct Spin {
    /// Dönme ekseni (gövde-yerel), normalize edilir.
    pub axis: Vec3,
    /// Açısal hız, rad/s. Runtime'da değiştirilebilir.
    pub angular_velocity: f32,
    /// Modelin dönmeden önceki (yazar) rotasyonu — dönüş bunun üzerine uygulanır.
    pub rest_rotation: Quat,
    /// Biriken açı (rad) — sistem tarafından yönetilir.
    pub angle: f32,
}

impl Spin {
    /// `axis` ekseninde `angular_velocity` (rad/s) ile dönen bileşen.
    pub fn new(axis: Vec3, angular_velocity: f32) -> Self {
        let axis = if axis.length_squared() > 1e-9 {
            axis.normalize()
        } else {
            Vec3::X
        };
        Self {
            axis,
            angular_velocity,
            rest_rotation: Quat::IDENTITY,
            angle: 0.0,
        }
    }

    /// Modelin yazar-duruş rotasyonunu koru (GLTF tekerleği gibi önceden döndürülmüş
    /// mesh'lerde şart — yoksa duruş bozulur). Zincirlenebilir.
    pub fn with_rest_rotation(mut self, rest: Quat) -> Self {
        self.rest_rotation = rest;
        self
    }
}

gizmo_core::impl_component!(Spin);

/// Her frame tüm [`Spin`]'leri ilerletip `Transform.rotation`'a uygular. [`SpinPlugin`]
/// bunu schedule'a ekler; el ile `SpinSystem.run(world, dt)` da çağrılabilir.
pub struct SpinSystem;

impl gizmo_core::system::System for SpinSystem {
    fn access_info(&self) -> gizmo_core::system::AccessInfo {
        let mut info = gizmo_core::system::AccessInfo::new();
        info.is_exclusive = true; // Spin + Transform'a mutable erişir
        info
    }

    fn run(&mut self, world: &World, dt: f32) {
        // SAFETY: exclusive sistem; Spin ve Transform ayrı bileşen tipleri (disjoint),
        // scheduler bu çalışırken başka mutable alias vermez.
        if let Some(mut q) = unsafe {
            world.query_unchecked::<(
                gizmo_core::query::Mut<Spin>,
                gizmo_core::query::Mut<Transform>,
            )>()
        } {
            for (_id, (mut spin, mut t)) in q.iter_mut() {
                spin.angle += spin.angular_velocity * dt;
                t.rotation = spin.rest_rotation * Quat::from_axis_angle(spin.axis, spin.angle);
                t.update_local_matrix();
            }
        }
    }
}

/// [`SpinSystem`]'i uygulamanın schedule'ına ekler → [`Spin`] bileşenli her mesh
/// otomatik döner.
pub struct SpinPlugin;

impl<State: 'static> crate::app::Plugin<State> for SpinPlugin {
    fn build(&self, app: &mut crate::app::App<State>) {
        app.schedule.add_di_system(
            gizmo_core::system::SystemConfig::new(Box::new(SpinSystem)).label("spin"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gizmo_core::system::System;
    use gizmo_core::world::World;

    #[test]
    fn spin_system_rotates_transform_over_rest() {
        let mut world = World::new();
        let e = world.spawn();
        let rest = Quat::from_rotation_y(0.5);
        world.add_component(e, Transform::new(Vec3::ZERO));
        // 2 rad/s, X ekseni, yazar-duruş korunur.
        world.add_component(e, Spin::new(Vec3::X, 2.0).with_rest_rotation(rest));

        let mut sys = SpinSystem;
        // 1 s topla (dt=1/60 × 60).
        for _ in 0..60 {
            sys.run(&world, 1.0 / 60.0);
        }

        let t = world.borrow::<Transform>();
        let rot = t.get(e.id()).unwrap().rotation;
        // 1 s'de ~2 rad dönmüş olmalı, rest'in üzerine.
        let expected = rest * Quat::from_axis_angle(Vec3::X, 2.0);
        assert!(
            rot.dot(expected).abs() > 0.9999,
            "Spin rest'in üzerine ~2 rad döndürmeli"
        );
        // Spin bileşeninin biriken açısı da ~2.
        let spins = world.borrow::<Spin>();
        assert!((spins.get(e.id()).unwrap().angle - 2.0).abs() < 1e-3);
    }
}
