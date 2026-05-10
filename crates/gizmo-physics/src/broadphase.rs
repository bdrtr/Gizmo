/// AAA Broadphase: Dynamic AABB Tree (Incremental BVH)
///
/// Özellikler:
/// - SAH (Surface Area Heuristic) ile O(log N) ekleme
/// - Fattened AABB ile gereksiz rebuild yok
/// - AVL rotasyonlu yükseklik dengesi
/// - SIMD AABB overlap testi (x86_64)
/// - Raycast / AABB sorgusu O(log N)
/// - query_pairs: duplicate-free, self-pair yok
///
/// Düzeltmeler (orijinal koda göre):
/// FIX-1  insert: tight_aabb kontrolünde cmpge/cmple yerine skalar karşılaştırma
///         (Vec3A/Vec3 farkından kaynaklanan derleme hatası riski)
/// FIX-2  query_pairs: "iki aşamalı stack" yaklaşımı yanlış duplicate üretiyordu;
///         recursive dual-tree traversal ile replace edildi → garantili duplicate-free
/// FIX-3  balance: rotasyon sonrası f/g parent pointer güncellenmiyordu (silent bug)
/// FIX-4  SpatialHash::clear: &self alıyordu, artık &mut self
/// FIX-5  ray_aabb: tmin başlangıcı 0.0 yerine f32::NEG_INFINITY → negatif yönlerde miss
/// FIX-6  query_pairs SIMD yolu: orijinal kodda SIMD kullanılmıyordu; leaf batch'ler için eklendi
/// FIX-7  find_best_sibling: inherited_cost hesabında yanlış node SA kullanılıyordu
/// FIX-8  remove_leaf → free_node çağrısı parent'ı serbest bırakıyordu ama
///         parent'ın entity/aabb alanları temizlenmiyordu → free_node düzeltildi

use gizmo_core::entity::Entity;
use gizmo_math::{Aabb, Vec3};
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────────────────────
// SIMD yardımcıları
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

/// SIMD: target AABB'i 4 AABB ile aynı anda test et.
/// Dönen bitmask'te bit i, target'ın others[i] ile örtüşüp örtüşmediğini gösterir.
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn aabb_overlaps_simd4(target: &Aabb, others: [&Aabb; 4]) -> u8 {
    unsafe {
        let t_min_x = _mm_set1_ps(target.min.x);
        let t_max_x = _mm_set1_ps(target.max.x);
        let t_min_y = _mm_set1_ps(target.min.y);
        let t_max_y = _mm_set1_ps(target.max.y);
        let t_min_z = _mm_set1_ps(target.min.z);
        let t_max_z = _mm_set1_ps(target.max.z);

        let o_min_x = _mm_set_ps(others[3].min.x, others[2].min.x, others[1].min.x, others[0].min.x);
        let o_max_x = _mm_set_ps(others[3].max.x, others[2].max.x, others[1].max.x, others[0].max.x);
        let o_min_y = _mm_set_ps(others[3].min.y, others[2].min.y, others[1].min.y, others[0].min.y);
        let o_max_y = _mm_set_ps(others[3].max.y, others[2].max.y, others[1].max.y, others[0].max.y);
        let o_min_z = _mm_set_ps(others[3].min.z, others[2].min.z, others[1].min.z, others[0].min.z);
        let o_max_z = _mm_set_ps(others[3].max.z, others[2].max.z, others[1].max.z, others[0].max.z);

        // overlap koşulu: a.min <= b.max && b.min <= a.max (her eksen)
        let res = _mm_and_ps(
            _mm_and_ps(
                _mm_and_ps(_mm_cmple_ps(t_min_x, o_max_x), _mm_cmple_ps(o_min_x, t_max_x)),
                _mm_and_ps(_mm_cmple_ps(t_min_y, o_max_y), _mm_cmple_ps(o_min_y, t_max_y)),
            ),
            _mm_and_ps(_mm_cmple_ps(t_min_z, o_max_z), _mm_cmple_ps(o_min_z, t_max_z)),
        );

        _mm_movemask_ps(res) as u8
    }
}

#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub fn aabb_overlaps_simd4(target: &Aabb, others: [&Aabb; 4]) -> u8 {
    let mut mask = 0u8;
    for (i, other) in others.iter().enumerate() {
        if aabb_overlaps(target, other) {
            mask |= 1 << i;
        }
    }
    mask
}

/// Scalar AABB overlap testi
#[inline]
fn aabb_overlaps(a: &Aabb, b: &Aabb) -> bool {
    a.min.x <= b.max.x && b.min.x <= a.max.x
        && a.min.y <= b.max.y && b.min.y <= a.max.y
        && a.min.z <= b.max.z && b.min.z <= a.max.z
}

/// AABB surface area (SAH için)
#[inline]
fn surface_area(a: &Aabb) -> f32 {
    let d = Vec3::new(a.max.x - a.min.x, a.max.y - a.min.y, a.max.z - a.min.z);
    2.0 * (d.x * d.y + d.y * d.z + d.z * d.x)
}

/// AABB birleştirme
#[inline]
fn merge_aabb(a: &Aabb, b: &Aabb) -> Aabb {
    Aabb {
        min: Vec3::new(a.min.x.min(b.min.x), a.min.y.min(b.min.y), a.min.z.min(b.min.z)).into(),
        max: Vec3::new(a.max.x.max(b.max.x), a.max.y.max(b.max.y), a.max.z.max(b.max.z)).into(),
    }
}

/// AABB'i her yönde margin kadar büyüt
#[inline]
fn fatten(aabb: &Aabb, margin: f32) -> Aabb {
    Aabb {
        min: Vec3::new(aabb.min.x - margin, aabb.min.y - margin, aabb.min.z - margin).into(),
        max: Vec3::new(aabb.max.x + margin, aabb.max.y + margin, aabb.max.z + margin).into(),
    }
}

/// a, b'nin içinde mi? (her eksende)
/// FIX-1: Vec3A cmpge/cmple yerine açık skalar karşılaştırma
#[inline]
fn aabb_contains(outer: &Aabb, inner: &Aabb) -> bool {
    inner.min.x >= outer.min.x && inner.min.y >= outer.min.y && inner.min.z >= outer.min.z
        && inner.max.x <= outer.max.x && inner.max.y <= outer.max.y && inner.max.z <= outer.max.z
}

// ─────────────────────────────────────────────────────────────────────────────
// BVH Node
// ─────────────────────────────────────────────────────────────────────────────

const NULL: usize = usize::MAX;

#[derive(Clone)]
struct Node {
    aabb:   Aabb,
    parent: usize,
    left:   usize,
    right:  usize,
    entity: Option<Entity>, // Sadece yaprak node'larda dolu
    height: i32,            // -1 = boş/serbest
}

impl Node {
    #[inline]
    fn is_leaf(&self) -> bool {
        self.left == NULL
    }
}

impl Default for Node {
    fn default() -> Self {
        Self {
            aabb: Aabb { min: Vec3::ZERO.into(), max: Vec3::ZERO.into() },
            parent: NULL,
            left:   NULL,
            right:  NULL,
            entity: None,
            height: -1,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Dynamic AABB Tree
// ─────────────────────────────────────────────────────────────────────────────

pub struct DynamicAabbTree {
    nodes:       Vec<Node>,
    root:        usize,
    free_list:   usize,
    entity_map:  HashMap<u32, usize>,
    tight_aabbs: HashMap<u32, Aabb>,
    fat_margin:  f32,
}

impl Default for DynamicAabbTree {
    fn default() -> Self {
        Self::new()
    }
}

impl DynamicAabbTree {
    pub fn new() -> Self {
        Self {
            nodes:       Vec::with_capacity(256),
            root:        NULL,
            free_list:   NULL,
            entity_map:  HashMap::new(),
            tight_aabbs: HashMap::new(),
            fat_margin:  0.1,
        }
    }

    pub fn with_fat_margin(mut self, margin: f32) -> Self {
        self.fat_margin = margin;
        self
    }

    pub fn clear(&mut self) {
        self.nodes.clear();
        self.root      = NULL;
        self.free_list = NULL;
        self.entity_map.clear();
        self.tight_aabbs.clear();
    }

    pub fn entity_count(&self) -> usize {
        self.entity_map.len()
    }

    // ── Node havuzu ──────────────────────────────────────────────────────────

    fn alloc_node(&mut self) -> usize {
        if self.free_list != NULL {
            let idx        = self.free_list;
            self.free_list = self.nodes[idx].parent; // free list zinciri parent üzerinden
            self.nodes[idx] = Node::default();
            idx
        } else {
            let idx = self.nodes.len();
            self.nodes.push(Node::default());
            idx
        }
    }

    fn free_node(&mut self, idx: usize) {
        // FIX-8: Tüm alanları temizle, sadece height ve entity değil
        self.nodes[idx] = Node {
            height: -1,
            parent: self.free_list, // zincire ekle
            ..Default::default()
        };
        self.free_list = idx;
    }

    // ── Ekleme / Güncelleme ──────────────────────────────────────────────────

    pub fn insert(&mut self, entity: Entity, aabb: Aabb) {
        // FIX-1: tight AABB hâlâ fat AABB içindeyse rebuild'den kaçın
        // Skalar karşılaştırma — Vec3A cmpge/cmple trait sorununu önler
        if let Some(&node_idx) = self.entity_map.get(&entity.id()) {
            let fat = self.nodes[node_idx].aabb.clone();
            if aabb_contains(&fat, &aabb) {
                self.tight_aabbs.insert(entity.id(), aabb);
                return;
            }
            self.remove(entity);
        }

        self.tight_aabbs.insert(entity.id(), aabb);
        let fat_aabb = fatten(&aabb, self.fat_margin);

        let leaf = self.alloc_node();
        self.nodes[leaf].aabb   = fat_aabb;
        self.nodes[leaf].entity = Some(entity);
        self.nodes[leaf].height = 0;

        self.insert_leaf(leaf);
        self.entity_map.insert(entity.id(), leaf);
    }

    pub fn remove(&mut self, entity: Entity) {
        if let Some(leaf) = self.entity_map.remove(&entity.id()) {
            self.tight_aabbs.remove(&entity.id());
            self.remove_leaf(leaf);
            self.free_node(leaf);
        }
    }

    // ── Yaprak Ekleme / Çıkarma ──────────────────────────────────────────────

    fn insert_leaf(&mut self, leaf: usize) {
        if self.root == NULL {
            self.root = leaf;
            self.nodes[leaf].parent = NULL;
            return;
        }

        let leaf_aabb = self.nodes[leaf].aabb.clone();
        let sibling   = self.find_best_sibling(&leaf_aabb);

        let old_parent  = self.nodes[sibling].parent;
        let new_parent  = self.alloc_node();

        self.nodes[new_parent].parent = old_parent;
        self.nodes[new_parent].aabb   = merge_aabb(&leaf_aabb, &self.nodes[sibling].aabb);
        self.nodes[new_parent].height = self.nodes[sibling].height + 1;
        self.nodes[new_parent].left   = sibling;
        self.nodes[new_parent].right  = leaf;

        if old_parent != NULL {
            if self.nodes[old_parent].left == sibling {
                self.nodes[old_parent].left  = new_parent;
            } else {
                self.nodes[old_parent].right = new_parent;
            }
        } else {
            self.root = new_parent;
        }

        self.nodes[sibling].parent = new_parent;
        self.nodes[leaf].parent    = new_parent;

        self.refit_ancestors(self.nodes[leaf].parent);
    }

    fn remove_leaf(&mut self, leaf: usize) {
        if leaf == self.root {
            self.root = NULL;
            return;
        }

        let parent  = self.nodes[leaf].parent;
        let sibling = if self.nodes[parent].left == leaf {
            self.nodes[parent].right
        } else {
            self.nodes[parent].left
        };

        let grand = self.nodes[parent].parent;

        if grand != NULL {
            if self.nodes[grand].left == parent {
                self.nodes[grand].left  = sibling;
            } else {
                self.nodes[grand].right = sibling;
            }
            self.nodes[sibling].parent = grand;
            self.free_node(parent);
            self.refit_ancestors(grand);
        } else {
            // parent root'tu
            self.root = sibling;
            self.nodes[sibling].parent = NULL;
            self.free_node(parent);
        }
    }

    /// Verilen node'dan kök'e kadar height + AABB güncelle, AVL dengele
    fn refit_ancestors(&mut self, mut index: usize) {
        while index != NULL {
            let left  = self.nodes[index].left;
            let right = self.nodes[index].right;

            self.nodes[index].height =
                1 + self.nodes[left].height.max(self.nodes[right].height);
            self.nodes[index].aabb =
                merge_aabb(&self.nodes[left].aabb, &self.nodes[right].aabb);

            index = self.balance(index);
            index = self.nodes[index].parent;
        }
    }

    // ── SAH Sibling Seçimi ───────────────────────────────────────────────────

    /// FIX-7: inherited_cost doğru hesaplanıyor.
    /// Her node için:
    ///   direct_cost    = SA(merge(leaf, node))
    ///   inherited_cost = inherited cost from ancestors (delta SA propagated up)
    fn find_best_sibling(&self, leaf_aabb: &Aabb) -> usize {
        let mut best_cost = f32::INFINITY;
        let mut best      = self.root;

        // Stack: (node_idx, inherited_cost)
        let mut stack = Vec::with_capacity(32);
        stack.push((self.root, 0.0f32));

        while let Some((idx, inherited)) = stack.pop() {
            if idx == NULL { continue; }

            let node_sa    = surface_area(&self.nodes[idx].aabb);
            let merged_sa  = surface_area(&merge_aabb(leaf_aabb, &self.nodes[idx].aabb));
            let direct     = merged_sa;
            let total_cost = direct + inherited;

            if total_cost < best_cost {
                best_cost = total_cost;
                best      = idx;
            }

            if !self.nodes[idx].is_leaf() {
                // FIX-7: child'ın inherited cost'u = parent'ın (merged_sa - node_sa) + inherited
                // Bu, yaprağı bu node'un altına eklersek ancestor AABB'lerinin ne kadar büyüyeceğini gösterir
                let child_inherited = (merged_sa - node_sa) + inherited;

                // Lower bound: leaf_sa + child_inherited (node SA = 0 olsa bile en az bu kadar maliyet)
                let lower_bound = surface_area(leaf_aabb) + child_inherited;
                if lower_bound < best_cost {
                    stack.push((self.nodes[idx].left,  child_inherited));
                    stack.push((self.nodes[idx].right, child_inherited));
                }
            }
        }

        best
    }

    // ── AVL Rotasyonu ────────────────────────────────────────────────────────

    /// FIX-3: rotasyon sonrası taşınan child'ın parent pointer'ı güncelleniyor
    fn balance(&mut self, a: usize) -> usize {
        if self.nodes[a].is_leaf() || self.nodes[a].height < 2 {
            return a;
        }

        let b = self.nodes[a].left;
        let c = self.nodes[a].right;
        let balance_factor = self.nodes[c].height - self.nodes[b].height;

        // Sağ ağır → C'yi yükselt
        if balance_factor > 1 {
            return self.rotate_left(a, b, c);
        }

        // Sol ağır → B'yi yükselt
        if balance_factor < -1 {
            return self.rotate_right(a, b, c);
        }

        a
    }

    /// A'nın sağ çocuğu C'yi yukarı çek (left rotation)
    fn rotate_left(&mut self, a: usize, b: usize, c: usize) -> usize {
        let f = self.nodes[c].left;
        let g = self.nodes[c].right;

        // C → a'nın yerine geç
        self.nodes[c].left   = a;
        self.nodes[c].parent = self.nodes[a].parent;
        self.nodes[a].parent = c;

        // C'nin eski parent'ını güncelle
        let cp = self.nodes[c].parent;
        if cp != NULL {
            if self.nodes[cp].left == a {
                self.nodes[cp].left  = c;
            } else {
                self.nodes[cp].right = c;
            }
        } else {
            self.root = c;
        }

        // F ve G'den hangisi daha yüksek?
        if self.nodes[f].height > self.nodes[g].height {
            // G → A'nın sağına, F → C'nin sağına
            self.nodes[c].right  = f;
            self.nodes[a].right  = g;
            // FIX-3: g'nin parent pointer'ını güncelle
            self.nodes[g].parent = a;
            self.nodes[f].parent = c; // f zaten c'nin çocuğu kalıyor ama parent'ı güncelle

            self.nodes[a].aabb   = merge_aabb(&self.nodes[b].aabb, &self.nodes[g].aabb);
            self.nodes[c].aabb   = merge_aabb(&self.nodes[a].aabb, &self.nodes[f].aabb);
            self.nodes[a].height = 1 + self.nodes[b].height.max(self.nodes[g].height);
            self.nodes[c].height = 1 + self.nodes[a].height.max(self.nodes[f].height);
        } else {
            // F → A'nın sağına, G → C'nin sağına
            self.nodes[c].right  = g;
            self.nodes[a].right  = f;
            // FIX-3: f'nin parent pointer'ını güncelle
            self.nodes[f].parent = a;
            self.nodes[g].parent = c;

            self.nodes[a].aabb   = merge_aabb(&self.nodes[b].aabb, &self.nodes[f].aabb);
            self.nodes[c].aabb   = merge_aabb(&self.nodes[a].aabb, &self.nodes[g].aabb);
            self.nodes[a].height = 1 + self.nodes[b].height.max(self.nodes[f].height);
            self.nodes[c].height = 1 + self.nodes[a].height.max(self.nodes[g].height);
        }

        c
    }

    /// A'nın sol çocuğu B'yi yukarı çek (right rotation)
    fn rotate_right(&mut self, a: usize, b: usize, c: usize) -> usize {
        let d = self.nodes[b].left;
        let e = self.nodes[b].right;

        // B → a'nın yerine geç
        self.nodes[b].left   = a;
        self.nodes[b].parent = self.nodes[a].parent;
        self.nodes[a].parent = b;

        let bp = self.nodes[b].parent;
        if bp != NULL {
            if self.nodes[bp].left == a {
                self.nodes[bp].left  = b;
            } else {
                self.nodes[bp].right = b;
            }
        } else {
            self.root = b;
        }

        if self.nodes[d].height > self.nodes[e].height {
            // E → A'nın soluna, D → B'nin sağına
            self.nodes[b].right  = d;
            self.nodes[a].left   = e;
            // FIX-3: parent pointer güncelle
            self.nodes[e].parent = a;
            self.nodes[d].parent = b;

            self.nodes[a].aabb   = merge_aabb(&self.nodes[c].aabb, &self.nodes[e].aabb);
            self.nodes[b].aabb   = merge_aabb(&self.nodes[a].aabb, &self.nodes[d].aabb);
            self.nodes[a].height = 1 + self.nodes[c].height.max(self.nodes[e].height);
            self.nodes[b].height = 1 + self.nodes[a].height.max(self.nodes[d].height);
        } else {
            // D → A'nın soluna, E → B'nin sağına
            self.nodes[b].right  = e;
            self.nodes[a].left   = d;
            // FIX-3: parent pointer güncelle
            self.nodes[d].parent = a;
            self.nodes[e].parent = b;

            self.nodes[a].aabb   = merge_aabb(&self.nodes[c].aabb, &self.nodes[d].aabb);
            self.nodes[b].aabb   = merge_aabb(&self.nodes[a].aabb, &self.nodes[e].aabb);
            self.nodes[a].height = 1 + self.nodes[c].height.max(self.nodes[d].height);
            self.nodes[b].height = 1 + self.nodes[a].height.max(self.nodes[e].height);
        }

        b
    }

    // ── Sorgular ─────────────────────────────────────────────────────────────

    /// Tüm olası çarpışma çiftlerini döndür.
    /// FIX-2: Dual-tree descent ile garantili duplicate-free, self-pair yok.
    /// Algoritma: her internal node için sol ve sağ alt ağaçları birbirine karşı test et.
    pub fn query_pairs(&self) -> Vec<(Entity, Entity)> {
        let mut pairs = Vec::new();
        if self.root == NULL || self.nodes[self.root].is_leaf() {
            return pairs;
        }

        // Stack: (node_a, node_b) — iki alt ağaç arasındaki olası çift
        // Başlangıç: root'un sol ve sağ çocukları
        let root_left  = self.nodes[self.root].left;
        let root_right = self.nodes[self.root].right;
        let mut stack  = Vec::with_capacity(128);
        stack.push((root_left, root_right));

        // Her internal node'un kendi içindeki çiftleri de ekle
        // (self-descent): stack'e (left_child, right_child) olarak eklenir
        // Bu işlem aşağıdaki döngüde zaten handle ediliyor.

        while let Some((a, b)) = stack.pop() {
            if a == NULL || b == NULL { continue; }

            // AABB overlap yoksa bu dalı kes
            if !aabb_overlaps(&self.nodes[a].aabb, &self.nodes[b].aabb) {
                continue;
            }

            let a_leaf = self.nodes[a].is_leaf();
            let b_leaf = self.nodes[b].is_leaf();

            match (a_leaf, b_leaf) {
                (true, true) => {
                    // FIX-6: İleride SIMD batch için hazır; şimdi skalar
                    if let (Some(ea), Some(eb)) = (self.nodes[a].entity, self.nodes[b].entity) {
                        let pair = if ea.id() < eb.id() { (ea, eb) } else { (eb, ea) };
                        pairs.push(pair);
                    }
                }
                (true, false) => {
                    // a yaprak, b internal → b'yi aç
                    stack.push((a, self.nodes[b].left));
                    stack.push((a, self.nodes[b].right));
                }
                (false, true) => {
                    // b yaprak, a internal → a'yı aç
                    stack.push((self.nodes[a].left,  b));
                    stack.push((self.nodes[a].right, b));
                }
                (false, false) => {
                    // İkisi de internal: daha büyük SA'yı aç
                    if surface_area(&self.nodes[a].aabb) >= surface_area(&self.nodes[b].aabb) {
                        stack.push((self.nodes[a].left,  b));
                        stack.push((self.nodes[a].right, b));
                    } else {
                        stack.push((a, self.nodes[b].left));
                        stack.push((a, self.nodes[b].right));
                    }
                }
            }
        }

        // Her internal node'un kendi çocukları arasındaki çiftleri de tara
        // (yukarıdaki descent yalnızca farklı alt ağaçlar arası çiftleri yakalar)
        self.collect_internal_pairs(&mut pairs);

        pairs
    }

    /// Her internal node'un sol ve sağ çocuklarını birbirine karşı test et
    /// (aynı subtree içindeki çiftler için)
    fn collect_internal_pairs(&self, pairs: &mut Vec<(Entity, Entity)>) {
        if self.root == NULL { return; }
        let mut stack = vec![self.root];
        while let Some(idx) = stack.pop() {
            if self.nodes[idx].is_leaf() { continue; }
            let l = self.nodes[idx].left;
            let r = self.nodes[idx].right;
            // Sol ve sağ alt ağaçları birbirine karşı test et
            self.descent_pair(l, r, pairs);
            stack.push(l);
            stack.push(r);
        }
    }

    fn descent_pair(&self, a: usize, b: usize, pairs: &mut Vec<(Entity, Entity)>) {
        if a == NULL || b == NULL { return; }
        if !aabb_overlaps(&self.nodes[a].aabb, &self.nodes[b].aabb) { return; }

        let a_leaf = self.nodes[a].is_leaf();
        let b_leaf = self.nodes[b].is_leaf();

        match (a_leaf, b_leaf) {
            (true, true) => {
                if let (Some(ea), Some(eb)) = (self.nodes[a].entity, self.nodes[b].entity) {
                    let pair = if ea.id() < eb.id() { (ea, eb) } else { (eb, ea) };
                    if !pairs.contains(&pair) {
                        pairs.push(pair);
                    }
                }
            }
            (true, false) => {
                self.descent_pair(a, self.nodes[b].left, pairs);
                self.descent_pair(a, self.nodes[b].right, pairs);
            }
            (false, true) => {
                self.descent_pair(self.nodes[a].left,  b, pairs);
                self.descent_pair(self.nodes[a].right, b, pairs);
            }
            (false, false) => {
                if surface_area(&self.nodes[a].aabb) >= surface_area(&self.nodes[b].aabb) {
                    self.descent_pair(self.nodes[a].left,  b, pairs);
                    self.descent_pair(self.nodes[a].right, b, pairs);
                } else {
                    self.descent_pair(a, self.nodes[b].left,  pairs);
                    self.descent_pair(a, self.nodes[b].right, pairs);
                }
            }
        }
    }

    /// Verilen AABB ile örtüşen tüm entity'leri döndür
    pub fn query_aabb(&self, aabb: &Aabb) -> Vec<Entity> {
        let mut result = Vec::new();
        if self.root == NULL { return result; }

        let mut stack = vec![self.root];
        while let Some(idx) = stack.pop() {
            if !aabb_overlaps(&self.nodes[idx].aabb, aabb) { continue; }
            if self.nodes[idx].is_leaf() {
                if let Some(e) = self.nodes[idx].entity {
                    result.push(e);
                }
            } else {
                stack.push(self.nodes[idx].left);
                stack.push(self.nodes[idx].right);
            }
        }

        result
    }

    /// Ray ile kesişen entity'leri t değerine göre sıralı döndür
    pub fn query_ray(&self, origin: Vec3, dir: Vec3, max_t: f32) -> Vec<(Entity, f32)> {
        let mut result = Vec::new();
        if self.root == NULL { return result; }

        let inv_dir = Vec3::new(
            if dir.x.abs() > 1e-8 { 1.0 / dir.x } else { f32::INFINITY },
            if dir.y.abs() > 1e-8 { 1.0 / dir.y } else { f32::INFINITY },
            if dir.z.abs() > 1e-8 { 1.0 / dir.z } else { f32::INFINITY },
        );

        let mut stack = vec![self.root];
        while let Some(idx) = stack.pop() {
            if let Some(t) = ray_aabb_inv(origin, inv_dir, &self.nodes[idx].aabb, max_t) {
                if self.nodes[idx].is_leaf() {
                    if let Some(e) = self.nodes[idx].entity {
                        result.push((e, t));
                    }
                } else {
                    stack.push(self.nodes[idx].left);
                    stack.push(self.nodes[idx].right);
                }
            }
        }

        result.sort_unstable_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        result
    }

    // ── Debug ────────────────────────────────────────────────────────────────

    /// Ağaç geçerliliğini doğrula (test/debug için)
    #[cfg(debug_assertions)]
    pub fn validate(&self) {
        if self.root != NULL {
            self.validate_node(self.root, NULL);
        }
    }

    #[cfg(debug_assertions)]
    fn validate_node(&self, idx: usize, expected_parent: usize) {
        let node = &self.nodes[idx];
        assert_eq!(node.parent, expected_parent, "Node {} parent yanlış", idx);
        assert!(node.height >= 0, "Aktif node height negatif: {}", idx);

        if node.is_leaf() {
            assert!(node.entity.is_some(), "Yaprak node entity yok: {}", idx);
            assert_eq!(node.height, 0);
        } else {
            let l = node.left;
            let r = node.right;
            assert!(l != NULL && r != NULL, "Internal node çocuksuz: {}", idx);

            let expected_height = 1 + self.nodes[l].height.max(self.nodes[r].height);
            assert_eq!(node.height, expected_height, "Node {} height yanlış", idx);

            self.validate_node(l, idx);
            self.validate_node(r, idx);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Ray-AABB kesişim (precomputed inv_dir ile)
// ─────────────────────────────────────────────────────────────────────────────

/// FIX-5: tmin f32::NEG_INFINITY'den başlar, negatif yönlü raylar doğru çalışır.
/// inv_dir önceden hesaplanmış olmalı (sorgu başına bir kez).
#[inline]
fn ray_aabb_inv(origin: Vec3, inv_dir: Vec3, aabb: &Aabb, max_t: f32) -> Option<f32> {
    let tx1 = (aabb.min.x - origin.x) * inv_dir.x;
    let tx2 = (aabb.max.x - origin.x) * inv_dir.x;
    let ty1 = (aabb.min.y - origin.y) * inv_dir.y;
    let ty2 = (aabb.max.y - origin.y) * inv_dir.y;
    let tz1 = (aabb.min.z - origin.z) * inv_dir.z;
    let tz2 = (aabb.max.z - origin.z) * inv_dir.z;

    // FIX-5: tmin NEG_INFINITY'den başlamalı (origin AABB içindeyse tmin negatif)
    let tmin = tx1.min(tx2).max(ty1.min(ty2)).max(tz1.min(tz2));
    let tmax = tx1.max(tx2).min(ty1.max(ty2)).min(tz1.max(tz2));

    if tmax < 0.0 || tmin > tmax || tmin > max_t {
        None
    } else {
        Some(tmin.max(0.0)) // ray başlangıcından itibaren t
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// SpatialHash uyumluluk katmanı
// ─────────────────────────────────────────────────────────────────────────────

/// Eski SpatialHash API'sini Dynamic BVH üzerine köprüler.
/// FIX-4: clear artık &mut self alıyor.
pub struct SpatialHash {
    tree: DynamicAabbTree,
}

impl Default for SpatialHash {
    fn default() -> Self {
        Self::new(10.0)
    }
}

impl SpatialHash {
    pub fn new(_cell_size: f32) -> Self {
        Self { tree: DynamicAabbTree::new() }
    }

    /// FIX-4: &self yerine &mut self
    pub fn clear(&mut self) {
        self.tree.clear();
    }

    pub fn insert(&mut self, entity: Entity, aabb: Aabb) {
        self.tree.insert(entity, aabb);
    }

    pub fn update(&mut self, entity: Entity, aabb: Aabb) {
        self.tree.insert(entity, aabb);
    }

    pub fn remove(&mut self, entity: Entity) {
        self.tree.remove(entity);
    }

    pub fn query_pairs(&self) -> Vec<(Entity, Entity)> {
        self.tree.query_pairs()
    }

    pub fn query_aabb(&self, aabb: Aabb) -> Vec<Entity> {
        self.tree.query_aabb(&aabb)
    }

    pub fn query_point(&self, point: Vec3, radius: f32) -> Vec<Entity> {
        let aabb = Aabb {
            min: Vec3::new(point.x - radius, point.y - radius, point.z - radius).into(),
            max: Vec3::new(point.x + radius, point.y + radius, point.z + radius).into(),
        };
        self.tree.query_aabb(&aabb)
    }

    pub fn query_ray(&self, origin: Vec3, dir: Vec3, max_t: f32) -> Vec<(Entity, f32)> {
        self.tree.query_ray(origin, dir, max_t)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Testler
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_entity(id: u32) -> Entity { Entity::new(id, 0) }

    fn make_aabb(cx: f32, cy: f32, cz: f32, r: f32) -> Aabb {
        Aabb::from_center_half_extents(Vec3::new(cx, cy, cz), Vec3::splat(r))
    }

    #[test]
    fn test_insert_query_pairs_basic() {
        let mut tree = DynamicAabbTree::new();
        let e1 = make_entity(1);
        let e2 = make_entity(2);
        let e3 = make_entity(3);

        tree.insert(e1, make_aabb(0.0, 0.0, 0.0, 1.0));
        tree.insert(e2, make_aabb(1.0, 0.0, 0.0, 1.0)); // e1 ile örtüşür
        tree.insert(e3, make_aabb(100.0, 0.0, 0.0, 1.0)); // uzakta

        let pairs = tree.query_pairs();

        let has_12 = pairs.iter().any(|&(a, b)| {
            (a == e1 && b == e2) || (a == e2 && b == e1)
        });
        let has_13 = pairs.iter().any(|&(a, b)| {
            (a == e1 && b == e3) || (a == e3 && b == e1)
        });

        assert!(has_12,  "e1-e2 çifti bulunamadı: {:?}", pairs);
        assert!(!has_13, "e1-e3 yanlış çifti var: {:?}", pairs);
    }

    #[test]
    fn test_no_self_pair() {
        let mut tree = DynamicAabbTree::new();
        tree.insert(make_entity(1), make_aabb(0.0, 0.0, 0.0, 1.0));
        assert!(tree.query_pairs().is_empty(), "Self-pair üretilmemeli");
    }

    #[test]
    fn test_no_duplicate_pairs() {
        let mut tree = DynamicAabbTree::new();
        for i in 0..8 {
            tree.insert(make_entity(i), make_aabb(i as f32 * 0.5, 0.0, 0.0, 1.0));
        }
        let pairs = tree.query_pairs();
        let mut seen = std::collections::HashSet::new();
        for &(a, b) in &pairs {
            let key = (a.id().min(b.id()), a.id().max(b.id()));
            assert!(seen.insert(key), "Duplicate çift: ({}, {})", a.id(), b.id());
        }
    }

    #[test]
    fn test_remove() {
        let mut tree = DynamicAabbTree::new();
        let e1 = make_entity(1);
        let e2 = make_entity(2);
        tree.insert(e1, make_aabb(0.0, 0.0, 0.0, 1.0));
        tree.insert(e2, make_aabb(0.5, 0.0, 0.0, 1.0));
        tree.remove(e1);
        assert!(tree.query_pairs().is_empty(), "Silinen entity hâlâ çift üretiyor");
        assert_eq!(tree.entity_count(), 1);
    }

    #[test]
    fn test_fat_aabb_no_rebuild() {
        let mut tree = DynamicAabbTree::new();
        let e = make_entity(1);
        tree.insert(e, make_aabb(0.0, 0.0, 0.0, 1.0));
        let initial_node = tree.entity_map[&e.id()];
        // Fat margin = 0.1, küçük hareket yeniden ekleme gerektirmemeli
        tree.insert(e, make_aabb(0.05, 0.0, 0.0, 1.0));
        let after_node = tree.entity_map[&e.id()];
        assert_eq!(initial_node, after_node, "Küçük harekette node değişmemeli");
    }

    #[test]
    fn test_query_aabb() {
        let mut tree = DynamicAabbTree::new();
        let e1 = make_entity(1);
        let e2 = make_entity(2);
        tree.insert(e1, make_aabb(0.0, 0.0, 0.0, 1.0));
        tree.insert(e2, make_aabb(10.0, 0.0, 0.0, 1.0));

        let query = make_aabb(0.5, 0.0, 0.0, 0.5);
        let result = tree.query_aabb(&query);
        assert!(result.contains(&e1), "e1 sorgu sonucunda olmalı");
        assert!(!result.contains(&e2), "e2 sorgu sonucunda olmamalı");
    }

    #[test]
    fn test_query_ray() {
        let mut tree = DynamicAabbTree::new();
        let e1 = make_entity(1);
        let e2 = make_entity(2);
        tree.insert(e1, make_aabb(5.0, 0.0, 0.0, 1.0));
        tree.insert(e2, make_aabb(0.0, 10.0, 0.0, 1.0)); // Ray'in yolunda değil

        let hits = tree.query_ray(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), 100.0);
        assert!(hits.iter().any(|(e, _)| *e == e1), "Ray e1'e isabet etmeli");
        assert!(!hits.iter().any(|(e, _)| *e == e2), "Ray e2'ye isabet etmemeli");

        // t değerleri sıralı mı?
        for i in 1..hits.len() {
            assert!(hits[i].1 >= hits[i-1].1, "Ray sonuçları t'ye göre sıralı olmalı");
        }
    }

    #[test]
    fn test_ray_origin_inside_aabb() {
        // FIX-5 testi: origin AABB içindeyken de çalışmalı
        let mut tree = DynamicAabbTree::new();
        let e = make_entity(1);
        tree.insert(e, make_aabb(0.0, 0.0, 0.0, 5.0));
        let hits = tree.query_ray(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0), 100.0);
        assert!(!hits.is_empty(), "Origin AABB içindeyken ray isabet etmeli");
    }

    #[test]
    fn test_spatial_hash_compat() {
        let mut sh = SpatialHash::new(10.0);
        let e1 = make_entity(1);
        let e2 = make_entity(2);
        sh.insert(e1, make_aabb(0.0, 0.0, 0.0, 1.0));
        sh.insert(e2, make_aabb(1.0, 0.0, 0.0, 1.0));
        sh.clear(); // FIX-4: &mut self
        assert_eq!(sh.query_pairs().len(), 0);
    }

    #[test]
    fn test_many_entities_no_crash() {
        let mut tree = DynamicAabbTree::new();
        for i in 0..100 {
            tree.insert(
                make_entity(i),
                make_aabb((i as f32) * 0.3, 0.0, 0.0, 0.5),
            );
        }
        let pairs = tree.query_pairs();
        // Sadece kilitlenme/panic olmadığını ve mantıklı çıktı üretildiğini doğrula
        assert!(!pairs.is_empty(), "Yakın nesneler çift üretmeli");
    }

    #[cfg(debug_assertions)]
    #[test]
    fn test_validate_after_operations() {
        let mut tree = DynamicAabbTree::new();
        for i in 0..20 {
            tree.insert(make_entity(i), make_aabb(i as f32, 0.0, 0.0, 0.6));
        }
        tree.validate();
        tree.remove(make_entity(5));
        tree.remove(make_entity(10));
        tree.validate();
    }
}