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
//!     (at most [`MAX_REQUESTS_PER_FRAME`] per frame). What stops it asking twice is
//!     [`AssetServer::streaming_requested`](crate::asset_server::AssetServer::streaming_requested)
//!     — it used to be "clear the material's `texture_source`", which put the request stage in the
//!     business of deleting the field a scene file saves. See `should_request`.

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

    // Aday entity'leri topla (Material read borrow'u ifade sonunda biter).
    let entities: Vec<u32> = world.borrow::<Material>().entities().collect();
    let transforms = world.borrow::<Transform>();
    let hidden = world.borrow::<gizmo_core::component::IsHidden>();
    let Some(mut server) = world.get_resource_mut::<crate::asset_server::AssetServer>() else {
        return;
    };
    // Read-only now. It used to be `borrow_mut_unchecked` because the loop CLEARED
    // `texture_source` as its "already asked" marker — see `should_request`, and
    // `AssetServer::streaming_requested` for what that cost.
    let materials = world.borrow::<Material>();

    let mut requests = 0usize;
    for e in entities {
        if requests >= MAX_REQUESTS_PER_FRAME {
            break;
        }
        if hidden.get(e).is_some() {
            continue; // gizli objeler stream edilmez
        }
        let Some(mat) = materials.get(e) else {
            continue;
        };
        let Some(path) = mat.texture_source.clone() else {
            continue;
        };
        let Some(t) = transforms.get(e) else {
            continue;
        };
        if cam_pos.distance_squared(t.position) < STREAM_IN_DISTANCE * STREAM_IN_DISTANCE
            && should_request(&mut server.streaming_requested, e, &path)
        {
            server.loader.request_texture_reload(path, e as usize);
            requests += 1;
        }
    }
}

/// Has this `(entity, path)` pair been asked for yet? Records it if not.
///
/// **The whole defect was in this one decision, and it used to be made destructively.** The marker
/// for "already asked" was `Material::texture_source = None`, and nothing put the path back: the
/// apply stage writes only `bind_group`. `material_sync` copies the material into a `MaterialDesc`
/// every frame, that is what a scene file carries, and on load it overrides `MaterialSource` — so
/// opening a textured scene, waiting a few seconds and saving quietly deleted every albedo path
/// the author had assigned, and the scene reopened white. The picture never changed while the
/// editor was open, which is why it went unnoticed: the bind group was still there.
///
/// Split out from the loop because it needs no GPU, and the GPU is why the test around it is
/// skipped on a headless machine — the rule itself is assertable everywhere.
fn should_request(
    requested: &mut std::collections::HashSet<(u32, String)>,
    entity: u32,
    path: &str,
) -> bool {
    requested.insert((entity, path.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::asset_server::AssetServer;
    use gizmo_math::Vec3;
    use gizmo_renderer::async_assets::TextureReloadCompletion;
    use gizmo_renderer::Renderer;

    /// **The dedup rule, without a GPU** — and that is the point of splitting it out.
    ///
    /// The test below needs a real `wgpu` device (a `Material` owns a bind group) and skips itself
    /// on a machine without an adapter, which is most CI. So the rule that replaced the
    /// destructive marker would have had no coverage exactly where regressions land. This has it:
    /// ask once per `(entity, path)`, and treat a re-assigned texture as a new question rather
    /// than a permanently suppressed one.
    #[test]
    fn a_pair_is_asked_for_once_and_a_new_path_is_a_new_question() {
        let mut requested = std::collections::HashSet::new();

        assert!(should_request(&mut requested, 7, "rock.png"), "the first frame asks");
        assert!(!should_request(&mut requested, 7, "rock.png"), "and no frame after it does");

        assert!(
            should_request(&mut requested, 8, "rock.png"),
            "a different entity is a different question — they get separate bind groups"
        );
        assert!(
            should_request(&mut requested, 7, "moss.png"),
            "and re-assigning the texture in the inspector must be asked again, or the new one \
             never streams in"
        );
    }

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
        // eşiğinin tam sınırında — kampanya kaydındaki "~12 koşuda 2 çöküş" artığının açıklaması bu.
        // Guard `gpu_available()` probe'undan da ÖNCE alınıyor: o probe da bir Instance kuruyor.
        let _gpu = crate::test_gpu::gpu_lock();
        if !gpu_available() {
            eprintln!("skip: GPU adapter yok (headless render GPU ister)");
            return;
        }
        let (world, mat_id) = setup();

        // (1) REQUEST: kameraya yakın + texture_source var → istek atılmalı ve YOL KALMALI.
        //
        // Bu iddia eskiden tam tersiydi ("texture_source None olmalı") ve hatayı sabitliyordu:
        // istek işareti olarak yolu SİLMEK, `material_sync` → `MaterialDesc` → sahne dosyası
        // zinciriyle kullanıcının atadığı doku yollarını sessizce siliyordu.
        request_nearby_textures(&world);
        assert_eq!(
            world
                .borrow::<Material>()
                .get(mat_id)
                .and_then(|m| m.texture_source.clone()),
            Some("dummy.png".to_string()),
            "istek atmak yolu silmemeli — sahne dosyasının kaydettiği alan bu"
        );
        assert!(
            world
                .get_resource::<AssetServer>()
                .expect("AssetServer resource")
                .streaming_requested
                .contains(&(mat_id, "dummy.png".to_string())),
            "istek, yolu bozmadan kaydedilmiş olmalı"
        );

        // …ve ikinci kare yeniden istemez: tekrarı durduran şey artık ayrı bir küme.
        request_nearby_textures(&world);
        assert_eq!(
            world
                .get_resource::<AssetServer>()
                .expect("AssetServer resource")
                .streaming_requested
                .len(),
            1,
            "aynı (entity, yol) çifti için ikinci istek atılmamalı"
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
