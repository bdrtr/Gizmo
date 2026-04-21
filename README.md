# Gizmo Engine

<div align="center">
  <img src="media/logo.png" alt="Gizmo Engine Logo" width="200" />
</div>

Gizmo Engine, Rust programlama dili kullanılarak sıfırdan geliştirilen; yüksek performanslı, veri odaklı (Data-Driven) ve tamamıyla modüler bir **3D Oyun Motoru ve Fizik Simülasyonu** çatısıdır. Gizmo Engine, performansın kritik olduğu geniş ölçekli fizik simülasyonları, modern araç içi dinamikleri ve gelişmiş 3D Rendering işlemleri için özel olarak inşa edilmiştir.

## 🚀 Motorun Yetenekleri (Neler Yapabilir?)

Gizmo Engine salt bir görüntüleyici olmanın ötesinde endüstri standardı özellikler sunan tam teşekküllü bir sistemdir. Motorun temel bileşenleri ve öne çıkan kabiliyetleri şunlardır:

### 🧩 Archetype Tabanlı Pürüzsüz ECS (Entity Component System)
Motorun kalbinde, tüm nesnelerin ve mantıksal sistemlerin veri-odaklı (DOD) olarak ayrıştırıldığı *Archetype* tabanlı, sütun (columnar) yapılı modern bir ECS miramisi yatar. `mimalloc` gibi global bellek tahsis edicilerle ve SIMD-uyumlu ArrayVec yapılarıyla bellek yükü (Allocation Overhead) ve Cache Miss oranları minimuma indirilmiştir. Çoklu sistemler darboğaz yaşamadan on binlerce "entity" güncelleyebilir.

### 🌌 Vektörel Gizmo Fizik Motoru
Üçüncü parti bir fizik API'si (Jolt, Rapier vb.) kullanılmadan **tamamen matematiksel vektör hesabı** ile inşa edilmiş, multi-body yapılar için destek sunan özel fizik çözücüsü:
* **Sweep and Prune (3D Broad-Phase):** 10.000'den fazla hareket eden nesne arasındaki olası çarpışmaları bulmak için Rayon destekli Çoklu-iş parçacığı (Multi-thread) kaba eleme algoritması.
* **Narrow-Phase & GJK/EPA:** Karmaşık poligon, küre, kapsül ve Convex Hull geometriler için kusursuz temas ve penetrasyon hesaplamaları. Çarpışma manifoldları (Collision Manifolds) *sıfır-tahsisli* (zero-allocation) `ArrayVec` yapısıyla saniyede binlerce çarpışmayı GC (Garbage Collection) vuruşu yaşamadan çözer.
* **Angular Jacobian Solver:** Eklemlerde (Ball-Socket, Hinge) açısal ivme ve tork üzerinden sequential impulse (iteratif vuruş) uygulayan pürüzsüz joint mekaniği.
* **Coulomb Sürtünme & Moment of Inertia:** Gerçekçi statik/dinamik sürtünme modellerine sahip, nesnenin atalet (eylemsizlik) momentini dikkate alan kusursuz fizik iterasyonları.

### 🏎️ Component Tabanlı Araç (Vehicle) Fiziği
Araçlara özel Raycast-tabanlı spring-damper süspansiyon sistemi. Anti-roll bar hesaplamaları, drift (yanlama) fizikleri için kayma ve tutunma grafikleri ve bağımsız FWD (Önden Çekiş), RWD (Arkadan İtiş) veya 4WD (Dört Çeker) tork asistanı ile çok esnek bir araç simülasyon dinamiği.

### 🎨 GPU Instancing & PBR Rendering
Vulkan/WGSL altyapısı sayesinde devasa sahneleri belleğe tek seferde kopyalayıp "Tek Draw Call" ile yüksek ekran yenileme hızında çizen instanced rendering mimarisi.
* **GLTF PBR Material Desteği:** Albedo, Normal Map, Metallic ve Roughness haritalarını otomatik harmanlayan, real-time ışıklandırmalı modern shader algoritmaları.
* **Dynamic Shadows & Post-Processing:** Gerçek zamanlı yönlü ışık gölgeleri, Bloom parlaması, HDR ton haritalama (Tone Mapping) ve Vignette gibi atmosferik iyileştirmeler.
* **Particle System & FX:** Karakterler veya drift dumanları gibi parçacık efektlerini draw-call yaratmadan üreten sistemler.

### 📐 Özel Matematik ve SIMD Mimarlığı
Üçüncü parti matematik veya vektör kütüphanesine bağlanmayan sıfırdan yazılmış geometri alt yapısı; Slab ve Möller–Trumbore gibi gelişmiş "Raycasting" kesişim algoritmalarını ve vektörel SIMD-tabanlı AABB/Frustum örtüşme testlerini içerir.
### 🎧 3D Uzamsal (Spatial) Ses Motoru
Karakterlerin veya motor seslerinin, ana kameraya veya oyuncuya olan uzaklığına/yakınlığına göre şiddeti azalıp artan, objenin yönüne bağlı panoramik (Örn: Motor solda çalışıyorsa sol kulaklıktan gelmesi) ortam üreten RAM-cache optimizasyonlu sistem. Doppler efekti ve mesafe zayıflatması (Distance Attenuation) desteki.

### 🛠️ Gelişmiş Editör ve Workflow
Sahneyi gerçek zamanlı denetlemek için oyuna gömülü (In-Game) çalışan UI panelleri:
* Gizli dosyaları ve nesneleri bulabileceğiniz dinamik hierarchy (Entity Ağacı).
* Pozisyon, rotasyon ve özellikleri anlık olarak değiştirebileceğiniz Inspector.
* Sürükle-bırak destekli "Prefab" sistemi ve sahne yönetim hiyerarşisi.

## 📊 Endüstri Standartları Teknik Değerlendirmesi
Gizmo Engine'in güncel mimarisi, modern AAA teknolojilerine (Unreal, Unity, Bevy/Flecs) kıyasla değerlendirildiğinde motorun gücü ve geliştirilme yol haritası (roadmap) şu şekildedir:

* **ECS ve Bellek Mimarisi (4.5 / 5):** Archetype (Sütun/Columnar) tabanlı veri yapısıyla en güncel endüstri standartlarındadır. Yüksek performanslı Global Allocator (`mimalloc`) kullanımıyla Cache-Locality zirvededir. Lock-free (RwLock) veri okuma/yazma erişimleri kusursuzdur.
  * **TODO (AAA Seviyesi Hedefler):**
    * **Command Buffer:** Multithread sistemler çalışırken veri kilitlenmesi yaşamadan (Deadlock-free) Entity ekleme/silme yapılabilmesi için ertelenmiş komut kuyruğu.
    * **System Dependency Graph:** Bileşen okuma/yazma gereksinimlerine göre sistemleri (`System`) otomatik paralelleştiren Yönlü Döngüsüz Grafik (DAG) zamanlayıcısı.
    * **Change Detection (Tick):** Sadece o karede değişmiş (mutasyon geçirmiş) komponentleri sorgulayabilme (`Changed<T>`) zekası.
    * **İlişkisel Archetype Hiyerarşisi:** Archetype dizilimlerinin Parent-Child hafıza kaydırmalarına entegre edilmesi.
* **Fizik Motoru (4.0 / 5):** GJK/EPA dar fazı (narrow-phase), Broad-phase algoritması ve Gauss-Seidel Sıralı İmpuls (Sequential Impulse) mekaniği doğrudan PhysX standardıdır. İlerleyen safhalarda, Kumaş (Soft-Body) fizikleri eklenecektir.
* **Grafik ve Render (3.5 / 5):** Vulkan (`wgpu`) tabanlı yapı; PBR render, Compute Shader parçacıkları (Particles) ve Dinamik gölgeler (CSM) ile başarılıdır. İleri seviye standardizasyon için Mesh Shader tabanlı GPU-Culling ve Temporal uzamsal filtrelemeler hedeflenmektedir.
* **Editör ve Tooling (3.5 / 5):** `egui` tabanlı anlık editör, docking ve sahne yönetim özelliği esnek bir zemin sunar. Tam teşekküllü bir AAA stüdyo deneyimi için Görsel Profilci (Flamegraphs & GPU Profiler) ve Geri Al (Undo Command Pattern) hedefler arasındadır.
* **Ses ve Math SIMD (3.5 / 5):** Gerçek zamanlı Doppler özellikli 3D Uzamsal (Spatial) Ses motoru ve SIMD-destekli (Slab, Möller-Trumbore) özel Culling/Raycast matematik çekirdeği ile bağımsızlığını kanıtlar.

## 📸 Motordan Görüntüler

Motor gücünü test etmek için hazırlanan araç / render senkronizasyon karelerinden bazıları:



![Gizmo City High-Res Rendering & GPU Particles](media/gizmo_city_demo.jpg)

![Volkswagen Test Scene](media/gizmo_engine_showcase.png)

![Gizmo Engine Demo](media/demo_racetrack.jpg)


## 📚 Dokümantasyon & Teknik Loglar
Motorun çekirdek yapısı sıfırdan geliştirilirken karşılaşılan kernel ve GPU driver düzeyindeki problemleri nasıl teşhis edip, motor seviyesinde nasıl çözdüğümüzü okuyabileceğiniz inceleme logları:
* [📖 Olay İncelemesi: WGSL Mesa `pow(0.0)` Linux Sürücü Hatası ve PBR Render Çökmesi](WGSL_MESA_BUG.md)

## 🎮 Motoru Derlemek ve Çalıştırmak
Sistemin becerilerini test etmek, geniş bir haritada aracı sürmek ve devasa fizik simülasyonunu görmek için:

```bash
cargo run --release --bin demo
```

> **Önemli Not:** Sistem on binlerce objenin fizik ve kaba eleme (Broad-Phase) hesaplamasını tek saniyede çözmek üzerine optimize edildiği için `--release` profili haricinde derlenmesi performans düşüklüğüne (Darboğaz) yol açacaktır! Mutlaka release build kullanın.
