//! Distance-based Texture Streaming System
//!
//! So as not to exceed VRAM limits in open-world games:
//! while the camera is far from the objects it does not keep the high-resolution version of the
//! textures; as it gets closer it asynchronously (AsyncAssetLoader) decodes the high-resolution
//! textures, uploads them to VRAM and applies them to the relevant materials.
//!
//! Two stages, every frame:
//!  1. **Apply**: upload the finished decodes accumulated by `asset_server_update_system`
//!     ([`AssetServer::completed_textures`]) to the GPU and update the entities'
//!     `Material.bind_group`. (Formerly this stage DID NOT EXIST → the decoded texture was
//!     discarded, streaming was visually a no-op.)
//!  2. **Request**: request a texture reload for materials near (≤50 m) the primary camera
//!     (at most [`MAX_REQUESTS_PER_FRAME`] per frame).

use gizmo_core::system::{AccessInfo, System};
use gizmo_core::World;
use gizmo_physics_core::Transform;
use gizmo_renderer::components::{Camera, Material};

/// Maximum new requests per frame, to limit a sudden VRAM load.
const MAX_REQUESTS_PER_FRAME: usize = 3;
/// Textures within this distance (m) are loaded at high resolution.
const STREAM_IN_DISTANCE: f32 = 50.0;

/// The system that drives texture streaming every frame (apply + request). Added to the
/// schedule by [`AssetServerPlugin`]. **exclusive**, because it uses the materials (mut) and
/// the `AssetServer`/`Renderer` resources.
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

/// Upload the streaming textures whose decode has finished to the GPU and apply them to the
/// entity materials. Resource borrows are scoped consecutively (no clashing mutable borrows at
/// the same time).
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

/// Request an asynchronous load for materials with a `texture_source` that are near the
/// primary camera.
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

    /// If there is no GPU adapter (headless CI) skip the test — Material/Renderer depend on the
    /// GPU. (The same probe as golden_render_tests; it does not leak an extra `wgpu::Instance`.)
    fn gpu_available() -> bool {
        pollster::block_on(Renderer::headless_adapter_available())
    }

    /// Headless Renderer + AssetServer + a primary camera (at the origin) + a Material entity
    /// with a `dummy.png` texture_source, close (1 m) to the camera. Returns `mat_id`.
    fn setup() -> (World, u32) {
        let renderer = pollster::block_on(crate::test_gpu::headless_renderer(64, 64));
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

    /// One test, one headless Renderer: verifies both the request path (nearby material →
    /// texture_source is cleared) and the apply path (finished decode → bind_group changes to
    /// the new texture + idempotence). NOTE: it is kept in a single test because an extra
    /// headless GPU context per test, together with the other GPU tests in the same process,
    /// crosses the segfault threshold in amdgpu teardown; also, `world` is released at the end
    /// with `mem::forget` (skips the wgpu device + AsyncAssetLoader thread teardown — the
    /// process is exiting anyway, the operating system reclaims it).
    #[test]
    fn texture_streaming_requests_nearby_and_applies_completed() {
        // Bu test kendi headless `Renderer`'ını (tam bir wgpu cihazı) kuruyor ve uzun süre
        // golden testlerle EŞZAMANLI koşuyordu: `gpu_lock` `golden_render_tests` içinde
        // private'dı, buradan erişilemiyordu. İki canlı cihaz, ölçülen 2-iyi/4-ölümcül
        // eşiğinin tam sınırında — FIXPLAN'ın "~12 koşuda 2 çöküş" artığının açıklaması bu.
        // Guard `gpu_available()` probe'undan da ÖNCE alınıyor: o probe da bir Instance kuruyor.
        let _gpu = crate::test_gpu::gpu_lock();
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
