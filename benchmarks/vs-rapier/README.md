# Gizmo ↔ Rapier3D

Motorun sert-cisim fiziğini, **aynı sahnelerde ve aynı `dt` ile**, Rapier3D ile
karşılaştırır. Workspace'in dışındadır: `cargo build --workspace` bunu görmez ve
`rapier3d` motorun bağımlılık grafiğine girmez.

```bash
cd benchmarks/vs-rapier && cargo run --release
```

## Ölçtükleri

| # | sahne | soru |
|---|---|---|
| 1 | eşit kütle, `e=1`, kafa kafaya | çarpışma **doğru** mu çözülüyor (analitik cevabı var) |
| 2 | 20 kutuluk kule, 600 adım | yığın **duruyor** mu |
| 3 | 1000 kutu / 1000 küre, 300 kare | **ne kadara** mal oluyor, ve hangi fazda |

## Okurken

**Alt-adım.** Gizmo `step(1/60)` başına içeride dört adım atar (`PHYSICS_HZ = 240`,
"sub-stepping ile mükemmel çarpışma tespiti"); Rapier bir. "Kare başına maliyet"
karşılaştırması kullanıcı seviyesinde dürüsttür — bir oyun kare başına bunu öder — ama
"adım başına iş" olarak okunmamalıdır.

**Simetri.** Rapier varsayılanda tek çekirdeklidir; `parallel` özelliği bu yüzden açık.
Gizmo broadphase'de Rayon kullandığı için eşitlemeden ölçmek kendimizi kayırmak olurdu —
ve ölçüldü: Rapier'a iş parçacığı verilince kare süresi 0,208 → 0,125 ms'ye iner ve
çözücüde bizim lehimize görünen fark kaybolur.

**Malzeme.** Varsayılanlara güvenmek kıyası bozar: Gizmo `restitution = 0,3` ile gelir,
Rapier `0,0` ile. Sahneler malzemeyi açıkça yazar.

**Uyku ve yerleşme.** Her hız satırı yanında kaç cismin uyanık bittiğini, yığının
ortalama yüksekliğini, yayılma yarıçapını ve en derin girişimi basar. Bunlar süs değil:
bir motor sahneyi erken uyutursa "hızlı" görünür, ve iki sahne aynı yere yerleşmediyse
ölçülen şey hız değil iki farklı simülasyondur.
