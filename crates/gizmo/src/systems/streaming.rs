//! Uzaklığa dayalı Doku Akış (Texture Streaming) Sistemi
//!
//! Açık dünya oyunlarında VRAM sınırlarını aşmamak için:
//! Kamera objelere uzaktayken kaplamaların yüksek çözünürlüklü versiyonunu tutmaz,
//! yaklaştıkça asenkron olarak (AsyncAssetLoader) yüksek çözünürlüklü dokuları decode edip
//! VRAM'e yükler ve ilgili materyallere uygular.
//!
//! İki aşama, her frame:
//!  1. **Apply**: `asset_server_update_system`'in biriktirdiği bitmiş decode'ları
//!     ([`AssetServer::completed_textures`]) GPU'ya yükle ve entity'lerin
//!     `Material.bind_group`'unu güncelle. (Eskiden bu aşama YOKTU → decode edilen
//!     texture atılıyordu, streaming görsel olarak no-op'tu.)
//!  2. **Request**: birincil kameraya yakın (≤50 m) materyaller için texture yeniden-yükleme
//!     iste (frame başına en çok [`MAX_REQUESTS_PER_FRAME`]).

use gizmo_core::system::{AccessInfo, System};
use gizmo_core::World;
use gizmo_physics_core::Transform;
use gizmo_renderer::components::{Camera, Material};

/// VRAM ani-yüklenmesini sınırlamak için frame başına maksimum yeni istek.
const MAX_REQUESTS_PER_FRAME: usize = 3;
/// Bu mesafenin (m) içindeki dokular yüksek çözünürlüklü yüklenir.
const STREAM_IN_DISTANCE: f32 = 50.0;

/// Texture streaming'i her frame süren sistem (apply + request). [`AssetServerPlugin`]
/// tarafından schedule'a eklenir. Materyalleri (mut) ve `AssetServer`/`Renderer`
/// kaynaklarını kullandığından **exclusive**.
pub struct TextureStreamingSystem;

impl System for TextureStreamingSystem {
    fn access_info(&self) -> AccessInfo {
        let mut info = AccessInfo::new();
        info.is_exclusive = true;
        info
    }

    fn run(&mut self, world: &World, _dt: f32) {
        apply_completed_textures(world);
        request_nearby_textures(world);
    }
}

/// Decode'u biten streaming texture'ları GPU'ya yükle ve entity materyallerine uygula.
/// Kaynak borrow'ları ardışık kapsamlanır (aynı anda çakışan mutable borrow yok).
fn apply_completed_textures(world: &World) {
    // 1) Biriken bitmiş decode'ları al (AssetServer borrow'u burada biter).
    let completions = {
        let Some(mut server) = world.get_resource_mut::<crate::asset_server::AssetServer>() else {
            return;
        };
        if server.completed_textures.is_empty() {
            return;
        }
        std::mem::take(&mut server.completed_textures)
    };

    // 2) Her birini GPU'ya yükle → (entity_ids, bind_group). Renderer borrow'u burada biter.
    let installed: Vec<(Vec<usize>, std::sync::Arc<wgpu::BindGroup>)> = {
        let Some(renderer) = world.get_resource::<gizmo_renderer::Renderer>() else {
            // Renderer yoksa (headless-no-render) uygulanamaz; sessizce bırak.
            return;
        };
        let mut am = match renderer.asset_manager.write() {
            Ok(am) => am,
            Err(poisoned) => poisoned.into_inner(),
        };
        completions
            .into_iter()
            .filter_map(|c| {
                match am.install_decoded_material_texture(
                    &renderer.device,
                    &renderer.queue,
                    &renderer.scene.texture_bind_group_layout,
                    &c.cache_key,
                    &c.rgba,
                    c.width,
                    c.height,
                ) {
                    Ok(bg) => Some((c.entity_ids, bg)),
                    Err(e) => {
                        tracing::warn!(
                            "[streaming] texture install failed ({}): {:?}",
                            c.cache_key,
                            e
                        );
                        None
                    }
                }
            })
            .collect()
    };

    // 3) Yüklenen bind_group'u ilgili entity'lerin materyaline uygula.
    if installed.is_empty() {
        return;
    }
    // SAFETY: exclusive sistem; scheduler bu çalışırken Material'a başka mutable erişim vermez.
    let mut materials = unsafe { world.borrow_mut_unchecked::<Material>() };
    for (entity_ids, bind_group) in installed {
        for eid in entity_ids {
            if let Some(mut mat) = materials.get_mut(eid as u32) {
                mat.bind_group = bind_group.clone();
            }
        }
    }
}

/// Birincil kameraya yakın, `texture_source`'lu materyaller için asenkron yükleme iste.
fn request_nearby_textures(world: &World) {
    // Birincil kamera pozisyonu (yoksa: ilk kamera; hiç kamera yoksa çık).
    let cam_pos = {
        let Some(q) = world.query::<(&Camera, &Transform)>() else {
            return;
        };
        let mut fallback = None;
        let mut primary = None;
        for (_id, (cam, t)) in q.iter() {
            if cam.primary {
                primary = Some(t.position);
                break;
            }
            if fallback.is_none() {
                fallback = Some(t.position);
            }
        }
        match primary.or(fallback) {
            Some(p) => p,
            None => return,
        }
    };

    // AsyncAssetLoader yoksa (AssetServer yok) çık.
    if world
        .get_resource::<crate::asset_server::AssetServer>()
        .is_none()
    {
        return;
    }

    // Aday entity'leri topla (Material read borrow'u ifade sonunda biter), sonra mutasyon.
    let entities: Vec<u32> = world.borrow::<Material>().entities().collect();
    let transforms = world.borrow::<Transform>();
    let hidden = world.borrow::<gizmo_core::component::IsHidden>();
    let server = world
        .get_resource::<crate::asset_server::AssetServer>()
        .expect("just checked present");
    // SAFETY: exclusive sistem; Material başka yerde mutable alias edilmez. Transform/IsHidden
    // ayrı bileşen tipleri (read), AssetServer ayrı kaynak → çakışma yok.
    let mut materials = unsafe { world.borrow_mut_unchecked::<Material>() };

    let mut requests = 0usize;
    for e in entities {
        if requests >= MAX_REQUESTS_PER_FRAME {
            break;
        }
        if hidden.get(e).is_some() {
            continue; // gizli objeler stream edilmez
        }
        let Some(mut mat) = materials.get_mut(e) else {
            continue;
        };
        let Some(path) = mat.texture_source.clone() else {
            continue;
        };
        let Some(t) = transforms.get(e) else {
            continue;
        };
        if cam_pos.distance_squared(t.position) < STREAM_IN_DISTANCE * STREAM_IN_DISTANCE {
            server.loader.request_texture_reload(path, e as usize);
            // Tekrar istek atılmasını engelle; decode bitince apply aşaması uygular.
            mat.texture_source = None;
            requests += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset_server::AssetServer;
    use gizmo_math::Vec3;
    use gizmo_renderer::async_assets::TextureReloadCompletion;
    use gizmo_renderer::Renderer;

    /// GPU adapter yoksa (headless CI) testi atla — Material/Renderer GPU'ya bağlı.
    /// (golden_render_tests ile aynı probe; ekstra `wgpu::Instance` sızdırmaz.)
    fn gpu_available() -> bool {
        pollster::block_on(Renderer::headless_adapter_available())
    }

    /// Headless Renderer + AssetServer + birincil kamera (orijinde) + `dummy.png`
    /// texture_source'lu, kameraya yakın (1 m) bir Material entity. `mat_id` döner.
    fn setup() -> (World, u32) {
        let renderer = pollster::block_on(Renderer::new_headless(64, 64, None));
        let mut world = World::new();

        let cam = world.spawn();
        world.add_component(cam, Camera::new(1.0, 0.1, 100.0, 0.0, 0.0, true));
        world.add_component(cam, Transform::new(Vec3::ZERO));

        let ent = world.spawn();
        let mut mat = Material::new(renderer.create_white_texture());
        mat.texture_source = Some("dummy.png".to_string());
        world.add_component(ent, mat);
        world.add_component(ent, Transform::new(Vec3::new(1.0, 0.0, 0.0)));

        world.insert_resource(renderer);
        world.insert_resource(AssetServer::new());
        (world, ent.id())
    }

    /// Tek test, tek headless Renderer: hem request (yakın materyal → texture_source
    /// temizlenir) hem apply (biten decode → bind_group yeni texture ile değişir +
    /// idempotentlik) yolunu doğrular. NOT: tek testte tutuluyor çünkü test-başına
    /// ekstra headless GPU context'i, aynı süreçteki diğer GPU testleriyle birlikte
    /// amdgpu teardown'ında segfault eşiğini aşıyor; ayrıca `world` sonda
    /// `mem::forget` ile bırakılıyor (wgpu device + AsyncAssetLoader thread teardown'ını
    /// atlar — süreç zaten çıkışta, işletim sistemi geri alır).
    #[test]
    fn texture_streaming_requests_nearby_and_applies_completed() {
        if !gpu_available() {
            eprintln!("skip: GPU adapter yok (headless render GPU ister)");
            return;
        }
        let (world, mat_id) = setup();

        // (1) REQUEST: kameraya yakın + texture_source var → istek atılıp temizlenmeli.
        request_nearby_textures(&world);
        assert!(
            world
                .borrow::<Material>()
                .get(mat_id)
                .is_some_and(|m| m.texture_source.is_none()),
            "yakın materyal için streaming isteği atılıp texture_source None olmalı"
        );

        // (2) APPLY: worker'ın decode'u bitirdiğini simüle et (2×2 kırmızı), uygula.
        let before = world
            .borrow::<Material>()
            .get(mat_id)
            .expect("material var")
            .bind_group
            .clone();
        world
            .get_resource_mut::<AssetServer>()
            .expect("AssetServer resource")
            .completed_textures
            .push(TextureReloadCompletion {
                cache_key: "test-red-2x2".to_string(),
                rgba: vec![
                    255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255,
                ],
                width: 2,
                height: 2,
                entity_ids: vec![mat_id as usize],
            });
        apply_completed_textures(&world);
        let after = world
            .borrow::<Material>()
            .get(mat_id)
            .expect("material var")
            .bind_group
            .clone();
        assert!(
            !std::sync::Arc::ptr_eq(&before, &after),
            "apply, bind_group'u yeni yüklenen texture ile DEĞİŞTİRMELİ"
        );

        // (3) IDEMPOTENTLİK: boş kuyrukta apply materyali değiştirmemeli.
        apply_completed_textures(&world);
        let after2 = world
            .borrow::<Material>()
            .get(mat_id)
            .expect("material var")
            .bind_group
            .clone();
        assert!(
            std::sync::Arc::ptr_eq(&after, &after2),
            "kuyruk boşken apply materyali değiştirmemeli"
        );

        // GPU device + loader thread teardown'ını atla (segfault önleme).
        std::mem::forget(world);
    }
}
