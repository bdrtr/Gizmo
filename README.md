# 🐉 Yelbegen Engine

Yelbegen, tamamen Rust ile ve **sıfırdan (from-scratch)** inşa edilmiş modüler bir 3D oyun motorudur. Üçüncü parti ağır donanımlara veya monolitik kütüphanelere bel bağlamadan; kendi fiziği, kendi matematiği ve kendi ECS (Varlık-Bileşen-Sistem) mimarisi ile donatılmıştır. 

AAA kalitesinde bir rendering altyapısına ulaşmak için grafik backend'i olarak `wgpu` API'sini (WebGPU standardı) kullanmaktadır.

## 🌟 Modüler Ekosistem (Crates)

- 🧮 **`yelbegen-math`**: Dışarıdan (`glam`, `cgmath` vb.) kullanmak yerine tamamen kendi hesapladığımız Matris dönüşümleri (`Mat4`) ve Vektör hesaplamaları.
- ⚙️ **`yelbegen-core`**: Bellek yönetimine (Cache-Locality) son derece duyarlı **Sparse Set** veri mimarisi üzerine kurulu bağımsız ECS.
- 🌌 **`yelbegen-physics`**: Optimizasyonu sağlanmış AABB çarpışma denetleyicisi. Sürtünme, Yerçekimi ve İtme (Restitution/Impulse) algoritmalarıyla tamamen native fizik hesaplayıcısı.
- 🎨 **`yelbegen-renderer`**: Z-Buffer derinlik haritalandırmaları, Gerçekçi Gölge & Aydınlatma hesaplamaları (Phong/Lambert) ve Gerçek (PNG/JPG) model dokusu haritalandırması.
- 🛠️ **`yelbegen-editor`**: Egui üzerinden entegre edilen, oyun motoru içi canlı (runtime) Inspector paneli. (Ağırlık, İvme manipülasyonu)

## 🧩 Kurulum ve Demo

Projeyi derlemek ve test simülasyonunu başlatmak için:

```bash
# Sadece Demo uygulamasını çalıştırmak için
cargo run -p demo
```

## 🔥 Gelecek Planları
* Modüler Oyun Döngüsü **(App Builder)** yapısı.
* Gelişmiş donanım hızlandırmalı Ray-Casting algoritmaları.
* `Obj` model import yeteneğinin geliştirilmesi.

Daha fazla detay için projenin her aşaması özenle dokümante edilmiştir.
