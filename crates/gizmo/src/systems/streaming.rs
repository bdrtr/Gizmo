//! Uzaklığa dayalı Doku Akış (Texture Streaming) Sistemi
//!
//! Açık dünya oyunlarında VRAM sınırlarını aşmamak için:
//! Kamera objelere uzaktayken kaplamaların düşük çözünürlüklü versiyonlarını (veya hiç) tutar,
//! yaklaştıkça asenkron olarak (AsyncAssetLoader) yüksek çözünürlüklü dokuları VRAM'e yazar.

use gizmo_core::World;
use gizmo_math::Vec3;
use gizmo_physics_core::Transform;
use gizmo_renderer::async_assets::AsyncAssetLoader;
use gizmo_renderer::components::Material;

/// Objelerin kameraya uzaklığına göre asenkron texture yüklemesini yönetir.
pub fn texture_streaming_system(world: &mut World, cam_pos: Vec3) {
    let loader_opt = world.get_resource::<AsyncAssetLoader>();
    let async_loader = if let Some(loader) = loader_opt {
        loader
    } else {
        return;
    };

    let entities: Vec<u32> = world.borrow::<Material>().entities().collect();
    let transforms = world.borrow::<Transform>();
    // SAFETY: exclusive `&mut World`; Material is a distinct component type from the
    // read-only Transform/IsHidden queries, so this mutable query never aliases them.
    let mut materials = unsafe { world.borrow_mut_unchecked::<Material>() };
    let hidden = world.borrow::<gizmo_core::component::IsHidden>();

    // VRAM kilitlenmesini engellemek için her frame max yükleme limiti (Agresif Streaming)
    let mut requests_this_frame = 0;
    const MAX_REQUESTS_PER_FRAME: usize = 3;

    // Tüm materyalleri döngüye al
    for e in entities {
        if hidden.get(e).is_some() {
            continue; // Gizli objeler stream edilmez
        }

        let mut mat = if let Some(m) = materials.get_mut(e) {
            m
        } else {
            continue;
        };

        // Eğer texture tanımlıysa uzaklık kontrolü yap
        if let Some(texture_path) = mat.texture_source.clone() {
            if let Some(t) = transforms.get(e) {
                let dist_sq = cam_pos.distance_squared(t.position);

                // 50 metre içindeki dokuları yüksek çözünürlüklü yükle
                let is_close = dist_sq < 50.0 * 50.0;

                if is_close && requests_this_frame < MAX_REQUESTS_PER_FRAME {
                    // Yükleme işlemini asenkron arka plan thread'ine (I/O) gönder
                    async_loader.request_texture_reload(texture_path.clone(), e as usize);
                    mat.texture_source = None; // DİKKAT: Tekrar istek atılmasını engelle!
                    requests_this_frame += 1;
                }
            }
        }
    }
}
