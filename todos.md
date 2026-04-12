# Gizmo Engine — TODO

**Durum: 20/20 tamamlandı ✅**

## 🔴 Kritik Hatalar

- [x] **~~EPA: İki `if` dalı özdeş — normal yönü yanlış~~** (`epa.rs`) ✅
- [x] **~~HeightField GJK/EPA + CharacterController~~** (`shape.rs`, `character.rs`) ✅
- [x] **~~GJK Triangle handler NaN~~** (`gjk.rs`) ✅
- [x] **~~Vehicle AABB raycast~~** (`vehicle.rs`) ✅
- [x] **~~CharacterController O(N) taraması~~** (`character.rs`) ✅

## 🟠 Önemli Sorunlar

- [x] **~~EPA tolerance sabit eşik~~** (`epa.rs`) ✅
- [x] **~~EPA edge search O(N)~~** (`epa.rs`) ✅
- [x] **~~BallSocket self-joint guard~~** (`constraints.rs`) — `entity_a == entity_b` guard eklendi ✅
- [x] **~~Solver iterasyon tutarsızlığı~~** (`constraints.rs`) ✅
- [x] **~~Vehicle frenleme titreşimi~~** (`vehicle.rs`) ✅
- [x] **~~Vehicle HeightField bilinear~~** (`vehicle.rs`) ✅
- [x] **~~CharacterController slope_limit~~** (`character.rs`) ✅
- [x] **~~CharacterController step climbing~~** (`character.rs`) ✅

## 🟡 Orta Seviye

- [x] **~~GJK Tetrahedron BCD yüzeyi~~** (`gjk.rs`) — outward normal flip ile eklendi ✅
- [x] **~~Drag sabitleri~~** (`integration.rs`) — `k=0.02` / `k=0.05` ✅
- [x] **~~EPA faces.remove O(N)~~** (`epa.rs`) ✅
- [x] **~~CCD bisection sweep drift~~** (`system.rs`) — GJK NaN guard ile korunuyor ✅

## 🔵 Küçükler / Temizlik

- [x] **~~EPA underscore parametreleri~~** (`epa.rs`) ✅
- [x] **~~GJK simplex testleri~~** (`gjk.rs`) — 5 yeni test eklendi ✅
- [x] **~~Vehicle borrow_mut~~** (`vehicle.rs`) ✅

## ✅ Bu Oturumda Yapılan Düzeltmeler

- [x] `create_cube` UV koordinatları eklendi (`asset.rs`)
- [x] Drag sabitleri düzeltildi (`integration.rs`)
- [x] EPA contact dist sign düzeltildi (`epa.rs`)
- [x] GJK NaN koruma eklendi (`gjk.rs`)
- [x] HeightField support_point O(N²) → O(1) (`shape.rs`)
- [x] Domino görsel/collider boyut uyumsuzluğu (`main.rs`)
- [x] BallSocket self-joint guard (`constraints.rs`)
- [x] GJK BCD yüzey kontrolü (`gjk.rs`)
- [x] GJK simplex kalite testleri: 5 yeni test (`gjk.rs`)
