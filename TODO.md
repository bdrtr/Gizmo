# Yelbegen Engine — Yapılacaklar Listesi

## ✅ Tamamlananlar
- [x] ECS (Entity Component System) — SparseSet + HashMap optimizasyonu
- [x] wgpu Renderer — PBR materyaller, point light, skybox
- [x] Fizik Sistemi — Yerçekimi, AABB çarpışma, zıplama
- [x] Matematik Kütüphanesi — Vec2/3/4, Mat4, Quat, Ray
- [x] Ses Sistemi — rodio tabanlı ses yükleme/çalma
- [x] Raycast — Fare tıklama ile obje seçimi (wgpu NDC uyumlu)
- [x] Editör UI (egui) — Inspector, hiyerarşi, spawn/delete
- [x] Gizmo — XYZ eksenleri, sürükle-bırak taşıma
- [x] Sahne Kaydetme — scene.json serializasyon/deserializasyon
- [x] Prelude — `use yelbegen::prelude::*` tek satır import
- [x] Küre Mesh — `AssetManager::create_sphere()` programatik UV sphere
- [x] Kütüphane API — Feature flag'ler (audio, editor), 3. parti re-export
- [x] Bellek Optimizasyonu — SparseSet sparse Vec → HashMap (%90 tasarruf)
- [x] Gölge Haritalama — `shadow.wgsl` pipeline'a entegre edilmeli

---

## 🔴 Kritik Öncelik (Hemen Yapılacaklar)
- [x] Dinamik Texture — Sadece `stone_tiles` değil, runtime'da doku yüklenmeli ve değiştirme
- [x] Rotation/Scale Gizmo — Döndürme ve ölçekleme gizmo'su

## 🟡 Orta Öncelik
- [x] Instancing — Aynı mesh için tek draw call (GPU instancing)
- [x] Frustum Culling — Kamera dışı objeleri atla
- [x] Scene Graph / Parent-Child — Obje hiyerarşisi (bağlı objeler)
- [x] Input Soyutlama — `input.is_key_pressed`, `mouse_delta` gibi temiz API eklendi
- [x] Query API — Bevy tarzı tuple iterator yapısı eklendi (`world.query_mut_mut`)
- [x] Event System — Component değişiklik olayları
- [x] Resource System — Global kaynakları (dt, window size) ECS içine al
- [x] Profiler — Frame time, draw call, entity sayısı overlay'i eklendi

## 🟢 Düşük Öncelik (Gelecek Vizyonu)
- [x] Sprite / 2D Renderer — Sprite component, Camera2D ortografik, create_sprite_quad
- [x] Skeletal Animation — Kemik tabanlı animasyon sistemi eklendi (GLTF)
- [x] Scripting — Lua 5.4 runtime (mlua, ScriptEngine, hot-reload, vec3 helpers)
- [x] Asset Pipeline — Hot-reload (notify file watcher, texture otomatik yeniden yükleme)
- [ ] Networking — Multiplayer altyapısı
- [x] Oyun İçi UI — UiCanvas, Button, Slider, ProgressBar, Text, Anchor sistemi
- [ ] Editör (Ayrı Repo) — Unity benzeri tam editör arayüzü
- [x] Post-Processing — Bloom, ACES Tone Mapping (HDR pipeline)
- [x] LOD (Level of Detail) — Mesafeye göre mesh detay seviyesi (LodGroup component)

---

## 🏎️ Gerçekçi Fizik Motoru (Yelbegen-Physics 2.0 Yol Haritası)
- [x] Açısal Hız (Angular Velocity) ve Quat Rotasyon Entegrasyonu
- [x] Ters Eylemsizlik Temsili (Inverse Inertia Tensor) ve Küp Matematiği
- [x] Çarpışma Manifoldlarına (Collision Manifold) Contact Point eklenmesi
- [x] İtme Gücü (Impulse) ve Tork (Torque) kullanarak dönme tepkisinin hesaplanması
- [x] Sphere-Sphere ve AABB-Sphere arasındaki Impulse çözünürlüğü tam doğrulaması (Takla atma testi yapıldı!)
- [x] Çoklu temas noktası (Multi-point contact / GJK-EPA algoritması) arayışı
- [x] Eklemler ve Kısıtlayıcılar (Ball Socket, Hinge, Distance, Spring + Baumgarte solver)
- [x] Sürtünme modelinin geliştirilmesi (Coulomb: Statik ve Kinetik sürtünme)

---

## 🚀 Maksimum Performans & Profesyonel Fizik (Physics 3.0)
- [ ] Broad-Phase Collision (Geniş Faz): Octree, Dynamic AABB Tree (BVH) veya Sweep & Prune algoritması. Her karede tüm objelerin birbiriyle (O(N^2)) kontrol edilmesini engeller!
- [ ] Island Sleeping (Uyku Sistemi): Hareket etmeyen (hızı ve açısı değişmeyen) objeleri "uyku" (sleep) moduna çekerek gereksiz kuvvet/çarpışma algoritmalarından (GJK-EPA) muaf tutma. Temas halinde uyanma.
- [ ] Fixed Time Stamp (Sabit Fizik Adımı): Fiziğin `dt` (Render Frame Time) yerine bağımsız ve sabit (ör: saniyede 60-120 kare) alt-adımlarla (sub-stepping) hesaplanması. Patlamaları ve jitter'i engeller.
- [ ] CCD (Continuous Collision Detection): Mermi gibi çok hızlı giden objelerin, ince duvarların içerisinden "tünelleme" yaparak (tunneling) geçip gitmesini engellemek için zamansal tarama (Sweep) yapılması.
- [ ] Paralelleştirme (Multi-threading): `Rayon` gibi kütüphanelerle farklı adalardaki fizik hesaplamalarını çoklu CPU çekirdeğine yayma.
- [ ] Karmaşık Şekiller (Convex Hull & Mesh Collider): GJK-EPA'yı geliştirip motora OBB (Oriented Bounding Box), Kapsül ve rastgele Convex (dışbükey) modeller girmek.

> Son güncelleme: 2026-04-05
