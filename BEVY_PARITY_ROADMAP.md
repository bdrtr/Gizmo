# Gizmo Engine: Architecture Modernization Roadmap

Bu belge, Gizmo Engine'i profesyonel bir seviyeye ve tam "Bevy-parity" standardına taşımak için entegre edilmesi planlanan 5 temel mimari özelliği listeler.

## 1. Reflection ve Sahne Serileştirme (`gizmo-reflect` & `gizmo-scene`) [ŞU ANKİ AŞAMA]
**Amaç:** Rust'ın statik yapısına rağmen çalışma zamanında tip bilgisini okuyabilmek (Reflection).
* **Görevler:**
  * `bevy_reflect` altyapısının core motor içerisine tam entegrasyonu.
  * Bileşenlere (Component) `#[derive(Reflect)]` yeteneği kazandırmak.
  * Dinamik UI Inspector entegrasyonu (`egui` üzerinden seçili nesnenin özelliklerini özel kod yazmadan otomatik olarak listeleme ve düzenleme).
  * Sahne Serileştirme (Scene Serialization): Dünyadaki Entity'leri ve Component'leri `.ron` dosyasına kaydedip geri yükleme (DynamicScene yapısı).

## 2. Çok İş parçacıklı (Multithreaded) ECS Executor & System Sets
**Amaç:** Motorun performans dar boğazlarını kaldırarak sistemleri bağımsız CPU çekirdeklerine dağıtmak.
* **Görevler:**
  * Component okuma/yazma (Read/Write) erişimlerini analiz eden akıllı Executor.
  * `SystemSet` altyapısı ile sistemleri modüler gruplara ayırma.
  * Sistemler arası çalışma önceliği belirleme (`.before()`, `.after()`).
  * Gelişmiş koşullu çalıştırma (Run Conditions, `.run_if()`).

## 3. Asenkron Asset Yönetimi ve Hot-Reloading (`gizmo-asset`)
**Amaç:** Yükleme sürelerini kısaltmak, frame-drop'ları engellemek ve canlı iterasyon hızını artırmak.
* **Görevler:**
  * `Handle<T>` tabanlı referans sayımlı (ref-counted) asenkron yükleme mimarisi.
  * Arka planda çalışan Asset İşçi Thread'leri.
  * Hot-Reloading özelliği (Oyun açıkken diskteki `.wgsl` veya `.png` dosyasını değiştirdiğinde motorun anında fark edip oyuna yansıtması).

## 4. ECS Tabanlı Flexbox Arayüz (`gizmo-ui`)
**Amaç:** Debug arayüzü (`egui`) haricinde, oyun-içi (In-Game) UI tasarımı için performanslı ve esnek bir yapı kurmak.
* **Görevler:**
  * `Taffy` kütüphanesi kullanılarak CSS Flexbox benzeri yerleşim motoru entegrasyonu.
  * UI ağaç yapısı (`NodeBundle`, `TextBundle`, `ButtonBundle`).
  * ECS üzerinden UI ile etkileşimi yöneten Event mekanizması (Hover, Click durumları).

## 5. İskelet Animasyon Sistemi (`gizmo-animation`)
**Amaç:** GLTF gibi formatlardan okunan karakter animasyonlarını oynatabilmek.
* **Görevler:**
  * `AnimationClip` veri modeli (zaman çizelgesi ve keyframeler).
  * Hiyerarşik animasyon oynatıcısı (`AnimationPlayer`).
  * Kemikler (Joint/Bone) arasındaki dönüşüm interpolasyonunu (Lerp/Slerp) her frame'de hesaplayan sistem.
