# Gizmo Engine Kullanım Rehberi

Gizmo Engine, Rust ile tamamen sıfırdan geliştirilmiş, WGPU tabanlı yüksek performanslı bir 3D oyun ve fizik motorudur. Bu rehber, motorun yeteneklerini ve editörü nasıl tam verimle kullanabileceğinizi adım adım açıklamaktadır.

---

## 1. Editörü Başlatmak ve Arayüz
Gizmo Studio (Editör) modunu başlatmak için projenin kök dizininde aşağıdaki komutu çalıştırınız:
```bash
cargo run --bin gizmo-studio
```
Motor açıldığında karşınıza ortasında 3D bir sahne, sağ tarafında ise "Hierarchy" (Hiyerarşi) paneli olan bir geliştirme arayüzü çıkar.

### Kamera ve Navigasyon Kontrolleri
* **Kameraya Yön Verme (Bakış):** Farenin **SAĞ tıkılına** basılı tutup fareyi sürükleyerek etrafınıza bakabilirsiniz.
* **Serbest Uçuş (Fly Camera):** Sağ tık basılıyken klavyede **W, A, S, D** tuşlarını kullanarak sahne içerisinde uçabilirsiniz. 

### Obje Seçimi ve Gizmo Kullanımı
* **Seçim Yapmak:** İster 3 boyutlu sahnede farenin sol tıkıyla doğrudan objenin üstüne tıklayın, isterseniz de sağ menüdeki hiyerarşi listesinden ismine tıklayın. Obje seçildiğinde etrafında şeffaf bir vurgu çerçevesi belirecektir.
* **Objeleri Taşımak/Döndürmek:** Seçili bir objenin üzerinde kırmızı (X), yeşil (Y) ve mavi (Z) oklardan oluşan manipülasyon araçları (Gizmo) belirir. Okların ucundan farenin sol kliğiyle tutup sürükleyerek objeyi hareket ettirebilirsiniz.

---

## 2. Entity Component System (ECS) Nedir?
Gizmo Engine klasik "OOP (Nesne Yönelimli Programlama)" kullanmaz. Bunun yerine modern kod mimarisi olan ECS kullanır. 
* **Entity**: Sadece basit bir numaradan ibarettir (Örn: `id: 208`). Kendine ait kodları veya davranışları yoktur.
* **Component**: Veridir. Örneğin `Transform` objenin pozisyonunu, `Collider` fiziksel sınırlarını saklar.
* **System**: Komponentleri işleyen motor fonksiyonlarıdır. (Örn: Tüm `RigidBody` ve `Collider` içeren objeleri işleyip düşmelerini sağlayan fizik sistemi).

### Temel Component'ler
- `Transform`: Objenin dünyadaki `position` (konum), `rotation` (açı), ve `scale` (büyüklük) verisini tutar.
- `MeshRenderer`: Objenin sahnede WGPU tarafından 3 boyutlu modelinin çizilmesini sağlar.
- `Material`: PBR (Fiziksel tabanlı) renderlama için renk ve doku/texture ayarlarını tutar.
- `Collider`: Çarpışma kutularıdır (`Aabb`, `Sphere`, `Capsule`). Işın testleri ve fizik çarpışmaları bu sınırlara göre yapılır.
- `RigidBody`: Fizik motorunun motorudur. Yerçekiminden etkilenme, zıplama ve ivmelenme hesapları için eklenir.

---

## 3. Kod Üzerinden Sahneye Obje Eklemek
Geliştirdiğiniz oyunlara veya projelere kod ile dinamik objeler eklemek için `World` objesini kullanırsınız. Örnek bir küp oluşturma kodu:

```rust
// 1. Yeni bir Entity ID üret
let cube_entity = world.spawn();

// 2. İsmini ver
world.add_component(cube_entity, EntityName("Benim Küpüm".to_string()));

// 3. Başlangıç pozisyonunu ata
world.add_component(cube_entity, Transform::new(Vec3::new(0.0, 5.0, 0.0)));

// 4. Mesh (Model) ekle
world.add_component(cube_entity, AssetManager::create_cube(&renderer.device));

// 5. Görsellik/Renk ekle
world.add_component(cube_entity, Material::new(texture).with_unlit(Vec4::new(1.0, 0.0, 0.0, 1.0))); // Kırmızı

// 6. Fizik çarpışma kutusu (Collider) ekle
world.add_component(cube_entity, Collider::new_aabb(1.0, 1.0, 1.0));

// 7. Yerçekimine tepki vermesi için RigidBody ekle
world.add_component(cube_entity, RigidBody::new_dynamic(1.0)); // Kütle: 1.0
```

---

## 4. Lua Scripting Sistemi (Oyun Mantığı)
Oyun mantığınızı Rust'ı tekrar derlemeden, hızlıca **Lua betik dili** ile yazabilirsiniz. Gizmo Engine, `mlua` vasıtasıyla kodunuzdaki objelerle Lua dilini birleştirir.

**Örnek bir Lua oyun kodu:** (Objeyi W-A-S-D ile yürütme ve Boşluk ile zıplatma)
```lua
function update(dt)
    -- Klavyeden girdi okuma API'si
    local speed = 10.0 * dt
    local cur_pos = Transform.get_position(entity_id)
    
    if Input.is_key_down("W") then
        cur_pos.z = cur_pos.z - speed
    end
    if Input.is_key_down("S") then
        cur_pos.z = cur_pos.z + speed
    end
    
    Transform.set_position(entity_id, cur_pos)
    
    if Input.is_key_pressed("Space") then
        -- Fizik API'si ile havaya zıplama gücü ekleme
        Physics.apply_impulse(entity_id, {x = 0.0, y = 5.0, z = 0.0})
    end
end
```
*Lua API'leri:* `Input`, `Transform`, `Physics`, `Audio` vb. çeşitli global kütüphaneler Gizmo tarafından otomatik olarak Lua koduna sunulur.

---

## 5. İpuçları ve En İyi Pratikler
* **Görünmez objeler/Sorunlar:** Objenize bir `MeshRenderer` ekleyebilirsiniz ancak `Transform` eklemeyi unutursanız motor onu ekranda nerede çizeceğini bilemez, hata vermez ama obje sahnede de gözükmez. Tüm `MeshRenderer`'ların `Transform` gerektirdiğini unutmayın.
* **Işın(Raycast) tıklama sorunları:** Eğer oluşturduğunuz objelere mause ile tıklanamadığını fark ederseniz `Collider` component'ini eksik bırakmış olabilirsiniz. Ekrandaki ışınlar görsellere(Mesh) değil, sadece Collider (matematiksel fizik şekli) verisine isabet edebilir.
* **Scale Etkisi:** `Transform.scale` aracılığı ile modeli 5 kat büyütürseniz, AABB (Axis-Aligned Bounding Box) sınırları kod tabanında otomatik olarak ölçekte büyüyecek ve fare tıklamanız sorunsuz stabil çalışacaktır.

Gizmo Engine ile harika oyunlar ve simülasyon fizikleri (örn: Pachinko) yapmaya hazırsınız. Yeni sistemler eklemek istediğinizde Rust üzerindeki ECS mimarisi üzerinden yepyeni "Component"ler uydurma konusunda özgür olduğunuzu unutmayın!
