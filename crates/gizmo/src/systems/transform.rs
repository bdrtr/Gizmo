use gizmo_core::query::Mut;
use crate::physics::Transform;

/// Her frame render'dan hemen önce çalışan sistem.
/// Tüm Transform bileşenlerinin `local_matrix`'ini position/rotation/scale'den yeniden hesaplar.
/// Bu sayede kullanıcının her yerde `trans.update_local_matrix()` çağırmasına gerek kalmaz.
#[tracing::instrument(skip_all, name = "transform_sync_system")]
pub fn transform_sync_system(world: &gizmo_core::world::World) {
    if let Some(mut q) = world.query::<Mut<Transform>>() {
        for (_, mut trans) in q.iter_mut() {
            trans.update_local_matrix();
        }
    }
}
