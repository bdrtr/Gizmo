Gizmo Engine — Derin Kod İnceleme Raporu 🔬
Toplam incelenen: system.rs (853 sat.), collision.rs (420), integration.rs (132), constraints.rs (365), gjk.rs (267), epa.rs (427), shape.rs (196), vehicle.rs (345), character.rs (318).

🔴 KRİTİK HATALAR

1. EPA generate_face_contacts — Her iki if dalı özdeş
epa.rs'de Sutherland-Hodgman kırpması sonrasında:
rustif extent_a <= extent_b {
    if dist <= 0.05 { final_contacts.push((*pt, -dist.min(0.0))); }
} else {
    if dist <= 0.05 { final_contacts.push((*pt, -dist.min(0.0))); }
}
Her iki dal tamamen aynı kod. "A reference face" ile "B reference face" arasındaki fark uygulanmıyor — normal yönü tersine çevrilmediği için B referans yüzey seçildiğinde temas noktalarının penetrasyon derinliği yanlış işaretli çıkabilir. Gerçekte B reference olduğunda dist hesabı ref_face[0] yani B'nin bir köşesine göre yapılmalı ve normal ters olduğundan dist işareti de dönmeli.

2. HeightField GJK/EPA'da AABB gibi davranıyor — Sessizce yanlış temas üretiyor
shape.rs support_point:
rustColliderShape::HeightField { width, max_height, depth, .. } => {
    let ly = if dir.y >= 0.0 { *max_height } else { 0.0 };
    // ...
}
max_height tüm yüksekliğin üst sınırıdır ama HeightField'ın gerçek yüzeyi o noktada çok daha alçak olabilir. GJK bu support fonksiyonunu kullanarak çarpışma yaptığında gerçek terrain yüzeyiyle değil, terrain'in düz üst tavanıyla çarpışıyor. Yani bir top düz zemine sahipmiş gibi yüksek bouncing yapar, terrain şeklini hiç izlemez. vehicle.rs'deki özel HeightField raycast kodu bu yüzden zorunlu hale gelmiş — ama CharacterController HeightField'a karşı hiç çalışmıyor (None döndürüyor, karakter terrain'den geçer).

3. GJK Triangle handler'da kalan path yok sayılan return false
gjk.rs handle_simplex size=3 kolunda: AB dışında değiliz, AC dışında değiliz, üçgenin içindeyiz — bu durumda dir güncelleniyor ama fonksiyon false döndürüyor. Mantıksal olarak doğru (simplex henüz tetrahedron değil), ama üçgen üzerinde tam orijin olan degenerate durumda bir sonraki iterasyonda dir sıfır uzunluklu olabilir. dir.length_squared() < 0.0001 kontrolü gjk_intersect'in başında sadece ilk support için var, sonraki iterasyonlarda bu guard yok. Sıfır dir ile support_point içinde dir.normalize() NaN üretir → tüm simplex NaN'a dönüşür → sonsuz döngü veya yanlış false.

4. vehicle.rs — AABB raycast'te t_near sıfıra clamp ediliyor, içerideyken yanlış
rusthit_t = if t_near > 0.0 { t_near } else { 0.0 };
Tekerlek zaten AABB içindeyken (t_near < 0, t_far > 0) hit_t = 0 oluyor. Bu durumda hit_t <= suspension_rest_length + wheel_radius her zaman doğru, yani tekerlek her zaman grounded sayılır. Araç büyük statik bir kutu içinde sıkışırsa süspansiyon sonsuz basınç üretir ve araç fırlar.

5. character.rs — resolve_capsule_collisions O(N) tüm collider taraması
rustfor other_entity in colliders.entity_dense.iter() {
Sahnede 10.000 collider varsa karakter her frame 10.000 çarpışma testi yapıyor × 3 slide iterasyonu = 30.000 test/frame. Karakter için broadphase yok. Vehicle sistemi bile sadece statik objeleri önceden ayırıyor.

🟠 ÖNEMLİ SORUNLAR

6. epa.rs find_support_face — ConvexHull için tolerance = 0.05 sabit eşik hatalı
rustlet tolerance = 0.05;
let verts: Vec<Vec3> = world_verts.iter()
    .filter(|(_, p)| max_proj - p < tolerance)
0.05 metre = 5 cm eşiği, küçük bir ConvexHull için bir yüzün tüm köşelerini yakalar, büyük bir hull için yanlış köşeleri de yakalayabilir. Eşik hull boyutuna göre orantılı olmalı. 10 metre uzunluğundaki bir hull için 0.05 metre eşiği temas yüzeyini tek köşeye indirir.

7. epa.rs — add_edge_if_unique O(N) linear search
rustif let Some(pos) = edges.iter().position(|&(ea, eb)| ea == b && eb == a) {
    edges.remove(pos);
Her kenar ekleme işleminde tüm edge listesini tarar. Karmaşık ConvexHull ile EPA genişlerken polytope yüzlerce kenar biriktirebilir. HashSet<(usize,usize)> kullanılmalıydı.

8. constraints.rs BallSocket — vels.get_mut aynı entity için iki kez çağrılıyor
rustif let Some(v_a) = vels.get_mut(joint.entity_a) { v_a.linear = va_lin; ... }
if let Some(v_b) = vels.get_mut(joint.entity_b) { v_b.linear = vb_lin; ... }
Eğer entity_a == entity_b (kendi kendine joint) olursa önceki değer ikinci yazmada ezilebilir. Büyük ihtimalle kullanım senaryosunda yok ama guard da yok.

9. constraints.rs solve_constraints — 15 iterasyon sabit hardcoded ama PhysicsSolverState.solver_iterations var
system.rs'de çözücü iterasyon sayısı world.get_resource::<PhysicsSolverState>() üzerinden okunuyor. Ama constraints.rs'de:
rustlet iterations = 15;
Sabit. İki çözücü tutarsız sayıda iterasyon yapıyor — collision 8 iter., joint 15 iter. Joint solver bitmeden velocity'ler değişiyor mu değişmiyor mu belirsiz. Aynı solver_iterations resource'unu kullanmalı.

10. vehicle.rs — Frenleme yönü her zaman forward.signum() ile belirleniyor
rustlet brake_impulse = forward * (-forward_speed.signum() * brake_force / num_wheels) * dt;
Araç durmak üzereyken forward_speed çok küçük ama sıfır değil. Her frame signum() küçük geri-ileri değer üretir → araç durduğunda titreşir. forward_speed.abs() < 0.01 ise fren uygulanmamalı.

11. vehicle.rs — HeightField terrain raycast bilinear interpolasyon yapmıyor
rustlet grid_x = (normalized_x * (*segments_x as f32 - 1.0)).round() as u32;
round() ile en yakın grid noktasına snap ediliyor. Tekerlek iki grid noktası arasındayken keskin adımlar yaşanır. Araç terrain üzerinde sürüyorsa titreklik gözlemlenir. Bilinear interpolasyon yapılmalı:
h = lerp(lerp(h00, h10, fx), lerp(h01, h11, fx), fz)

12. character.rs — Zemin tespiti normal.y > 0.7 sabit eşiği slope_limit ile tutarsız
rustif normal.y < -0.7 { is_grounded = true; ... }
else if normal.y > 0.7 { is_grounded = true; ... }
Sabit 0.7 ≈ 45.57 derece. Ama cc.slope_limit varsayılan 45 derece. Kullanıcı slope_limit = 30 derse karakter 30-45 derece arasındaki eğimlerde hem grounded sayılır hem de slope kontrolünde kayar — tutarsız davranış.

13. character.rs — Basamak çıkma (step climbing) yanlış pozisyon kullanıyor
rustlet step_test_pos = Vec3::new(
    t.position.x + horizontal_move.x,  // ← t.position, new_pos değil!
    t.position.y + cc.step_height,
    ...
);
t.position bu frame'in başlangıç pozisyonu. Slide sonrası new_pos'tan test edilmeli. Eğer slide geri ittiyse step test yanlış yerde yapılır.

🟡 ORTA SEVIYE

14. gjk.rs — Tetrahedron handler BCD yüzeyini kontrol etmiyor
Klasik GJK implementasyonunda 4 yüzey (ABC, ACD, ADB, BCD) kontrol edilir. Bu implementasyon sadece ABC, ACD, ADB'yi kontrol ediyor. BCD yüzeyini atlamak teorik olarak hatalı — pratikte GJK'nın push metodu her zaman yeni noktayı points[0]'a koyduğu için A her zaman en son eklenen nokta ve BCD'nin orijini içermesi çok nadir ama matematiksel olarak garantili değil.

15. integration.rs — Sleep threshold speed_sq < 0.01 her iki eksen ayrı değerlendiriliyor
rustlet speed_sq = v.linear.length_squared() + v.angular.length_squared();
length_squared() birimsiz toplam. 0.01 m²/s² linear hız (≈10 cm/s) çok büyük — nesneler hâlâ yavaş hareket ederken uyuyabilir. Bunun da ötesinde linear ve angular threshold'ları birbirine ekleniyor ama birimleri farklı (m/s vs rad/s). Ayrı eşiklerle kontrol edilmeli.

16. epa.rs — Polytope genişleme sırasında faces.remove(i) O(N) shift
rustfaces.remove(i + 2);
faces.remove(i + 1);
faces.remove(i);
Her Vec::remove sonraki tüm elemanları sola kaydırır. EPA her iterasyonda birden fazla yüzeyi kaldırıyorsa bu O(N²)'ye gider. swap_remove + index yönetimi veya ayrı to_remove listesi daha verimli olur.

17. system.rs — CCD bisection t_low'un güncellenmesi sırasında sweep vektörü kayıyor
İlk analizimde belirtmiştim ama şimdi kodu satır satır okuyunca daha net: t_low güncellendikten sonra sweep_mid = rel_v * (t_mid - t_low) hesabında yeni t_low değeri kullanılıyor. Bu doğru ama pa_low = pos_a + v_a_lin * t_low ile birlikte sweep başlangıç noktasının kaymasına yol açar. 16. iterasyona gelindiğinde t_low ≈ t_high olduğu için son sweep vektörü neredeyse sıfır ve GJK degenerate simplex üretebilir.

🔵 KÜÇÜKLERİ / TEMIZLIK

18. epa.rs — _shape_b, _pos_b, _rot_b parametreleri generate_face_contacts'ta kullanılıyor ama underscore ile işaretli
rustfn generate_face_contacts(
    shape_a: &ColliderShape, pos_a: Vec3, rot_a: Quat,
    _shape_b: &ColliderShape, _pos_b: Vec3, _rot_b: Quat,
Ama fonksiyon içinde find_support_face(_shape_b, _pos_b, _rot_b, ...) çağrılıyor. Underscore prefix yanıltıcı — kullanılmıyor izlenimi veriyor.
19. gjk.rs testlerinde sadece intersect/disjoint kontrol ediliyor, simplex doğruluğu kontrol edilmiyor
EPA için simplex'in kalitesi kritik ama test yok.
20. vehicle.rs — trans_storage borrow_mut olarak alınıyor ama sadece get (immutable) kullanılıyor
world.borrow_mut::<Transform>() çağrısı yapılmış ama trans_storage.get(entity) şeklinde immutable okunuyor. borrow::<Transform>() yeterli — mutable borrow gereksiz yere diğer sistemleri bloke eder.

Öncelik Sırası Özeti
#DosyaSorunEtki1epa.rsİki if dalı özdeş — normal yönü yanlışYanlış collision response2shape.rs+character.rsHeightField GJK yanlış, karakter geçerTerrain kullanılamaz3gjk.rsSıfır dir NaN üretebilirCrash / NaN yayılımı4vehicle.rsAABB içindeyken hit_t=0Araç fırlar5vehicle.rsHeightField bilinear yokTitreklik6character.rsStep climbing yanlış t.positionBasamak çıkma bozuk7character.rsZemin eşiği slope_limit'le tutarsızEğimde tutarsız davranış8constraints.rsHinge limit entity_a'ya uygulanmıyorNewton 3 ihlali9character.rsBroadphase yok, O(N) taramaPerformans10epa.rsadd_edge_if_unique O(N)Performans
