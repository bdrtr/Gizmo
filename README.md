# Gizmo Engine

Gizmo Engine, Rust ile tamamen sıfırdan geliştirilen bağımsız, esnek ve modüler bir Oyun Motoru ve Fizik simülasyon iskeletidir. Çok oyunculu, performans kritik ve karmaşık simülasyon sistemleri geliştirmek için inşa edilmiştir.

## Core Features (Özellikler)

*   **Pürüzsüz ECS (Entity Component System):** Her şey veri odaklı (Data-Driven) ve modüler bir mimari olan Entity-Component-System etrafında şekillenmiştir. Modüller arası bağımlılık en aza indirilmiştir.
*   **Gizmo Physics:** 
    *   **Angular Jacobian Solver:** Kısıtlayıcılarda (örneğin Ball-Socket gibi eklemlerde) Tork ve Açısal Hız tabanlı Sequential Impulse iterasyon hesaplamaları yer alır. Ragdoll yapılarını ve sarkaçları mükemmel çözümler.
    *   **Sweep and Prune (3D Broad-Phase):** Motor, 10.000'den fazla hareketli objeyi N^2 darboğazından kurtarmak için yüksek performanslı 3D AABB kaba eleme sistemine sahiptir.
    *   **Narrow-phase GJK/EPA:** İmkansız geometrik formlar için anlık çarpışma ve penetrasyon tespiti.
*   **Gizmo Renderer (GPU Instancing):** Donanım tabanlı instancing desteği ile binlerce nesneyi tek draw-call altında çok yüksek FPS ile bastırma özelliği. Vulkan mimarisi ile güçlü entegrasyon.
*   **Component Tabanlı Araç (Vehicle) Fiziği:** Raycast tabanlı süspansiyon ve anti-roll sistemleriyle karmaşık araç yapıları simüle edilebilir.

## Çalıştırma

Motoru denemek adına kapsamlı bir asteroid/fizik testini çalıştırmak için:

```bash
cargo run --release --bin demo
```

> **Not:** 10,000 objedeki fizikleri en az işlemde tamamlaması için `--release` profili kullanılması kritik öneme sahiptir.
