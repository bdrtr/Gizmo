# Gizmo Engine - Development Plan

## Phase 1: Foundation Achieved (Completed)
- [x] Basic Entity Component System
- [x] 3D Renderer Setup (Custom PBR shaders, Bind Bindings)
- [x] Custom Math Component Library (Quat, Vectors, Mat4)
- [x] Broad-Phase Spatial Partitioning (3D AABB Sweep & Prune)
- [x] Narrow-Phase Collision (Sphere/Capsule/AABB/ConvexHull with GJK-EPA)
- [x] Angular Velocity & Jacobian Multi-Body Joint Constraints (Ball-Socket)

## Phase 2: Next Steps
- [ ] **Warm-Starting & Constraint Caching:** Kısıtlayıcılarda bir önceki karenin (frame) `lambda` değerini ön-bellekte (cache) tutup, baştan başlamak yerine oradan iterasyona başlatmak. (Büyük ağlıklar/uzun zincirler için esneme önleyici).
- [ ] **Rayon Multi-threading:** Fizik çözücüsünde, birbirine temas etmeyen (Island) ayrı adaları tespit edip, paralel iş parçacıklarında (multi-threading) çözme (Island Island Parallellization).
- [ ] **Continuous Collision Detection (CCD):** Kurşun veya çok hızlı meteorlar gibi cisimlerin objelerin içinden tünelleme ile geçmesini (Tunneling) kapsüle dayalı ray-cast formülü ile önleme.
- [ ] **Friction Model Update:** Coulomb Sürtünme Modelinin (kinetik ve statik) kısıtlayıcı iterasyonlara eksiksiz yedirilmesi. Objelerin rampa üzerinde kaymadan tam stabil durabilmesi.
