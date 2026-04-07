# Gizmo Engine - Development Plan

## Phase 1: Foundation Achieved (Completed)
- [x] Basic Entity Component System
- [x] 3D Renderer Setup (Custom PBR shaders, Bind Bindings)
- [x] Custom Math Component Library (Quat, Vectors, Mat4)
- [x] Broad-Phase Spatial Partitioning (3D AABB Sweep & Prune)
- [x] Narrow-Phase Collision (Sphere/Capsule/AABB/ConvexHull with GJK-EPA)
- [x] Angular Velocity & Jacobian Multi-Body Joint Constraints (Ball-Socket)

## Phase 2: Advanced Physics (Completed)
- [x] **Warm-Starting & Constraint Caching**
- [x] **Rayon Multi-threading (Island Parallelization)**
- [x] **Friction Model Update (Coulomb)**
- [ ] **Continuous Collision Detection (CCD):** Hızlı objeler için tünelleme önleyici.

## Phase 3: Vehicle Dynamics (Completed)
- [x] Raycast-based Spring Damper Suspension system.
- [x] Engine torque, Ackermann steering, slip-based friction.

## Phase 4: Graphics Optimization (Completed)
- [x] GPU Instancing & Hardware Batching.
- [x] Dynamic shadow maps & Post-Processing (Bloom/HDR/Vignette).

## Phase 5: Editor & Workflow (Completed)
- [x] Dynamic Asset Browser (Drag & Drop Prefabs).
- [x] Component Inspector (Runtime visual tweaking with Egui).

## Phase 6: Spatial Audio Engine (Completed)
- [x] RAM-cached I/O optimized sound manager.
- [x] 3D SpatialSink (Rodio) with Doppler & Distance Attenuation.

## Phase 7: Next Frontier (Draft Roadmap)
- [ ] **Skeletal Animation Integration:** Tam teşekküllü (Bone/Joint) karakter animasyonları ve State Machine mimarisi.
- [x] **Particle Systems (Visual FX):** Patlama, toz, drift dumanı gibi efektler için GPU/Compute tabanlı parçacık işleyicisi.
- [ ] **Continuous Collision Detection (CCD):** Yüksek hızlı mermiler (Kurşun) için Swept-Volume tünel önleyicisi.
- [ ] **Scene Serialization & Prefabs:** Tasarlanan dünyaları JSON/Bincode olarak sabit diske kaydedip sonradan yükleyebilme yeteneği.
