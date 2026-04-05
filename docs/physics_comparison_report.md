# Yelbegen Physics vs. Unity (PhysX) Analiz Raporu

Yelbegen Engine için şu ana kadar kurduğumuz temel sağlam olsa da, Unity (PhysX) gibi ticari seviyede bir "tam" fizik motoru olabilmesi için eksik olan kritik özellikleri önem sırasına göre aşağıda listeledim.

---

## 🟥 1. Kritik Eksikler (Olmazsa Olmazlar)

### ⛓️ Eklemler ve Kısıtlamalar (Constraints & Joints)
**Unity Karşılığı:** `Hinge Joint`, `Fixed Joint`, `Spring Joint`, `Character Joint`.
*   **Nedeni:** Şu an Yelbegen'de sadece çarpışma (collision) var. Kapıların menteşeden dönmesi, arabaların fiziksel tekerlekle bağlanması veya bir "Ragdoll" (insan vücudu) yapılması için objelerin bir noktadan birbirine bağlanması gerekir.
*   **Çözüm:** Pozisyonel ve açısal kısıtlamaları çözen "Constraint Solver" (PBD veya Impulse-based) eklenmeli.

### 🛡️ Kapsül ve Konveks Şekiller (Capsules & Convex Hulls)
**Unity Karşılığı:** `Capsule Collider`, `Mesh Collider`.
*   **Nedeni:** Şu an sadece Kutu ve Küre var. Ancak karakterler için en ideal şekil `Capsule`'dür (basamak çıkmak için). Ayrıca karmaşık modeller için verimli `Convex Hull` algoritmaları eksik.
*   **Çözüm:** GJK algoritmasını Kapsül ve Konveks meshler için genişletmek.

### 🏃‍♂️ Karakter Kontrolcüsü (Kinematic Character Controller)
**Unity Karşılığı:** `CharacterController` bileşeni.
*   **Nedeni:** Oyuncular genellikle tam fizik yasalarına (yuvarlanma bounciness vb.) uymak istemez. Eğimlerden çıkarken kaymama, merdiven tırmanma ve "smooth" hareket için özel bir kinematik fizik katmanı gerekir.

### ⚡ Sürekli Çarpışma Tespiti (Continuous Collision Detection - CCD)
**Unity Karşılığı:** `CollisionDetectionMode.Continuous`.
*   **Nedeni:** Mermi gibi çok hızlı giden objeler, şu anki "Discrete" (kare kare) sistemde duvarın içinden geçip gidebilir (Tunneling).
*   **Çözüm:** Ray-casting tabanlı veya Sweep-sphere tabanlı zaman dilimi kontrolleri.

---

## 🟧 2. İşlevsellik ve Kullanılabilirlik (Workflow)

### 🛰️ Raycast ve Shape-Cast API
**Unity Karşılığı:** `Physics.Raycast`, `Physics.SphereCast`, `Physics.OverlapSphere`.
*   **Nedeni:** Oyun içinde "Önümde düşman var mı?" veya "Yer neresi?" diye sormak için genel bir sorgulama sistemi lazım. Şu an sadece motorun içinde süspansiyon için özel ray-cast var.

### 🏷️ Çarpışma Katmanları (Collision Layers & Matrix)
**Unity Karşılığı:** `Layer Collision Matrix`.
*   **Nedeni:** Mermilerin birbirine çarpmamasını ama düşmana çarpmasını, veya kameranın oyuncu içinden geçebilmesini sağlamak için katman bazlı filtreleme şart.

### 🛎️ Tetikleyiciler (Triggers)
**Unity Karşılığı:** `Collider.isTrigger`.
*   **Nedeni:** Bir kapının önüne gelindiğinde olay tetiklemek için kullanılan, fiziksel çarpma (itme) yapmayan ama orada olduğumuzu bilen hayalet colliderlar.

---

## 🟩 3. Optimizasyon ve Ölçeklenebilirlik

### 🌳 Gelişmiş Geniş Faz (Broadphase BVH)
**Unity Karşılığı:** `Bounding Volume Hierarchy (BVH)`.
*   **Nedeni:** Şu anki `Sweep and Prune` sistemimiz çok fazla obje tek bir eksende toplandığında yavaşlar. Dinamik bir BVH veya Octree binlerce objeyi 60 FPS'te yönetmemizi sağlar.

### 💤 Island-Based Sleeping (Ada Bazlı Uyku)
**Nedeni:** Birbirine dokunan objelerin bir "ada" oluşturup, hareket etmediklerinde topluca uyuması. Şu an her obje kendi başına uyumaya çalışıyor, bu da stabiliteyi bozar.

### 🧵 Paralel Çözücü (Multithreaded Solver)
**Nedeni:** Rayon gibi kütüphanelerle fizik adalarını farklı CPU çekirdeklerinde çözmek.

---

## 🟦 4. İleri Seviye (Premium) Özellikler

1.  **Soft Body (Yumuşak Cisim):** Jöle, bez ve deforme olan kumaş fiziği.
2.  **Fluid Simulation (Sıvı):** Su akışkanlığı ve duman (SPH tabanlı).
3.  **Destruction (Parçalanma):** Meshlerin darbe anında fiziksel olarak kırılması.

---

### 🏁 Özet: İlk Önce Neyi Yapmalıyız?
Eğer Unity kalitesinde bir motor istiyorsak, **Önem Sırası** şudur:
1.  **Raycast API** (Gameplay için şart)
2.  **Joints/Constraints** (Mekanizmalar için şart)
3.  **Capsule Collider** (Karakter hareketi için şart)
4.  **Collision Layers** (Proje düzeni için şart)

Yelbegen şu an bir **"Fizik Çekirdeği"** (Core) seviyesinde. Bu listedeki ilk 3 maddeyi eklediğimizde gerçek bir **"Fizik Motoru"** haline gelecektir.
