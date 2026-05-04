/// AAA-kalitesi Broadphase: Dynamic AABB Tree (Incremental BVH)
///
/// Spatial Hash'den farkı:
/// - Heterojen boyutlu nesnelerde O(log N) ekleme/silme
/// - Sürekli güncellenen "fattened AABB" ile gereksiz rebuild yok
/// - Raycast / AABB sorgusu O(log N)
/// - Çok sayıda static obje ile verimli çalışır (static/dynamic ayrımı)
use gizmo_core::entity::Entity;
use gizmo_math::{Aabb, Vec3};
use std::collections::HashMap;

// ─────────────────────────────────────────────────────────────────────────────
// AABB Yardımcıları
// ─────────────────────────────────────────────────────────────────────────────

// Helper functions removed, using native Aabb methods.

#[cfg(target_arch = "x86_64")]
use std::arch::x86_64::*;

/// SIMD: Test one target AABB against 4 other AABBs simultaneously.
/// Returns a bitmask where bit i is 1 if target overlaps others[i], 0 otherwise.
#[cfg(target_arch = "x86_64")]
#[inline]
pub fn aabb_overlaps_simd4(target: &Aabb, others: [&Aabb; 4]) -> u8 {
    unsafe {
        // Load target min/max
        let t_min_x = _mm_set1_ps(target.min.x);
        let t_max_x = _mm_set1_ps(target.max.x);
        let t_min_y = _mm_set1_ps(target.min.y);
        let t_max_y = _mm_set1_ps(target.max.y);
        let t_min_z = _mm_set1_ps(target.min.z);
        let t_max_z = _mm_set1_ps(target.max.z);

        // Load others min/max
        let o_min_x = _mm_set_ps(others[3].min.x, others[2].min.x, others[1].min.x, others[0].min.x);
        let o_max_x = _mm_set_ps(others[3].max.x, others[2].max.x, others[1].max.x, others[0].max.x);
        let o_min_y = _mm_set_ps(others[3].min.y, others[2].min.y, others[1].min.y, others[0].min.y);
        let o_max_y = _mm_set_ps(others[3].max.y, others[2].max.y, others[1].max.y, others[0].max.y);
        let o_min_z = _mm_set_ps(others[3].min.z, others[2].min.z, others[1].min.z, others[0].min.z);
        let o_max_z = _mm_set_ps(others[3].max.z, others[2].max.z, others[1].max.z, others[0].max.z);

        // a.min <= b.max
        let c1x = _mm_cmple_ps(t_min_x, o_max_x);
        let c1y = _mm_cmple_ps(t_min_y, o_max_y);
        let c1z = _mm_cmple_ps(t_min_z, o_max_z);

        // a.max >= b.min  => b.min <= a.max
        let c2x = _mm_cmple_ps(o_min_x, t_max_x);
        let c2y = _mm_cmple_ps(o_min_y, t_max_y);
        let c2z = _mm_cmple_ps(o_min_z, t_max_z);

        // AND everything together
        let res_x = _mm_and_ps(c1x, c2x);
        let res_y = _mm_and_ps(c1y, c2y);
        let res_z = _mm_and_ps(c1z, c2z);
        let res_xy = _mm_and_ps(res_x, res_y);
        let res = _mm_and_ps(res_xy, res_z);

        // Extract bitmask (top bit of each 32-bit float)
        _mm_movemask_ps(res) as u8
    }
}

// Fallback for non-x86_64
#[cfg(not(target_arch = "x86_64"))]
#[inline]
pub fn aabb_overlaps_simd4(target: &Aabb, others: [&Aabb; 4]) -> u8 {
    let mut mask = 0;
    for i in 0..4 {
        if target.intersects(*others[i]) {
            mask |= 1 << i;
        }
    }
    mask
}

/// Objenin AABB'ini fat_margin kadar büyüt (sık rebuild'i önler)
fn fatten(aabb: &Aabb, margin: f32) -> Aabb {
    Aabb {
        min: Vec3::new(aabb.min.x - margin, aabb.min.y - margin, aabb.min.z - margin).into(),
        max: Vec3::new(aabb.max.x + margin, aabb.max.y + margin, aabb.max.z + margin).into(),
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// BVH Node
// ─────────────────────────────────────────────────────────────────────────────

const NULL: usize = usize::MAX;

#[derive(Clone)]
struct Node {
    aabb:    Aabb,
    parent:  usize,
    left:    usize,
    right:   usize,
    entity:  Option<Entity>, // Sadece yaprak node'da dolu
    height:  i32,            // -1 = boş
}

impl Node {
    fn is_leaf(&self) -> bool { self.left == NULL }
}

impl Default for Node {
    fn default() -> Self {
        Self {
            aabb: Aabb {
                min: Vec3::ZERO.into(),
                max: Vec3::ZERO.into(),
            },
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
    nodes:      Vec<Node>,
    root:       usize,
    free_list:  usize,
    /// Entity → node index
    entity_map: HashMap<u32, usize>,
    /// Nesnenin gerçek (fat olmayan) AABB'i — re-insert kararı için
    tight_aabbs: HashMap<u32, Aabb>,
    fat_margin:  f32,
}

impl DynamicAabbTree {
    pub fn new() -> Self {
        Self {
            nodes:       Vec::with_capacity(256),
            root:        NULL,
            free_list:   NULL,
            entity_map:  HashMap::new(),
            tight_aabbs: HashMap::new(),
            fat_margin:  0.1, // 10cm padding
        }
    }

    pub fn clear(&mut self) {
        self.nodes.clear();
        self.root       = NULL;
        self.free_list  = NULL;
        self.entity_map.clear();
        self.tight_aabbs.clear();
    }

    // ── Node Allocation ───────────────────────────────────────────────────────

    fn alloc_node(&mut self) -> usize {
        if self.free_list != NULL {
            let idx        = self.free_list;
            self.free_list = self.nodes[idx].parent; // free_list zinciri parent'tan geliyor
            self.nodes[idx] = Node::default();
            idx
        } else {
            let idx = self.nodes.len();
            self.nodes.push(Node::default());
            idx
        }
    }

    fn free_node(&mut self, idx: usize) {
        self.nodes[idx].height = -1;
        self.nodes[idx].entity = None;
        self.nodes[idx].parent = self.free_list; // Zincire ekle
        self.free_list         = idx;
    }

    // ── Ekleme / Güncelleme ───────────────────────────────────────────────────

    /// Ağaca entity ekle ya da güncelle
    pub fn insert(&mut self, entity: Entity, aabb: Aabb) {
        // Eğer zaten varsa, AABB hâlâ fat AABB içindeyse hiçbir şey yapma
        if let Some(&node_idx) = self.entity_map.get(&entity.id()) {
            let fat = &self.nodes[node_idx].aabb;
            if aabb.intersects(*fat)
                && aabb.min.cmpge(fat.min).all()
                && aabb.max.cmple(fat.max).all()
            {
                self.tight_aabbs.insert(entity.id(), aabb);
                return; // Hâlâ fat içinde, rebuild gerekmez
            }
            self.remove(entity);
        }

        self.tight_aabbs.insert(entity.id(), aabb);
        let fat_aabb = fatten(&aabb, self.fat_margin);
        let leaf     = self.alloc_node();
        self.nodes[leaf].aabb   = fat_aabb;
        self.nodes[leaf].entity = Some(entity);
        self.nodes[leaf].height = 0;
        self.insert_leaf(leaf);
        self.entity_map.insert(entity.id(), leaf);
    }

    /// Entity'yi ağaçtan çıkar
    pub fn remove(&mut self, entity: Entity) {
        if let Some(leaf) = self.entity_map.remove(&entity.id()) {
            self.tight_aabbs.remove(&entity.id());
            self.remove_leaf(leaf);
            self.free_node(leaf);
        }
    }

    // ── Yaprak Ekleme / Çıkarma ───────────────────────────────────────────────

    fn insert_leaf(&mut self, leaf: usize) {
        if self.root == NULL {
            self.root = leaf;
            self.nodes[leaf].parent = NULL;
            return;
        }

        // En iyi sibling'i Surface Area Heuristic (SAH) ile bul
        let leaf_aabb = self.nodes[leaf].aabb.clone();
        let sibling   = self.find_best_sibling(&leaf_aabb);

        // Yeni internal node oluştur
        let old_parent = self.nodes[sibling].parent;
        let new_parent = self.alloc_node();
        self.nodes[new_parent].parent = old_parent;
        self.nodes[new_parent].aabb   = leaf_aabb.merge(self.nodes[sibling].aabb);
        self.nodes[new_parent].height = self.nodes[sibling].height + 1;

        if old_parent != NULL {
            // sibling kök değildi
            if self.nodes[old_parent].left == sibling {
                self.nodes[old_parent].left  = new_parent;
            } else {
                self.nodes[old_parent].right = new_parent;
            }
        } else {
            self.root = new_parent;
        }

        self.nodes[new_parent].left  = sibling;
        self.nodes[new_parent].right = leaf;
        self.nodes[sibling].parent   = new_parent;
        self.nodes[leaf].parent      = new_parent;

        // Yukarı yükselerek AABB'leri güncelle + AVL rotasyonu
        let mut index = self.nodes[leaf].parent;
        while index != NULL {
            let left  = self.nodes[index].left;
            let right = self.nodes[index].right;
            self.nodes[index].height = 1 + self.nodes[left].height.max(self.nodes[right].height);
            self.nodes[index].aabb   = self.nodes[left].aabb.merge(self.nodes[right].aabb);
            index = self.balance(index);
            index = self.nodes[index].parent;
        }
    }

    fn remove_leaf(&mut self, leaf: usize) {
        if leaf == self.root {
            self.root = NULL;
            return;
        }

        let parent   = self.nodes[leaf].parent;
        let sibling  = if self.nodes[parent].left == leaf {
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

            // AABB'leri yukarı doğru güncelle
            let mut index = grand;
            while index != NULL {
                let left  = self.nodes[index].left;
                let right = self.nodes[index].right;
                self.nodes[index].height = 1 + self.nodes[left].height.max(self.nodes[right].height);
                self.nodes[index].aabb   = self.nodes[left].aabb.merge(self.nodes[right].aabb);
                index = self.balance(index);
                index = self.nodes[index].parent;
            }
        } else {
            self.root = sibling;
            self.nodes[sibling].parent = NULL;
            self.free_node(parent);
        }
    }

    // ── Surface Area Heuristic Sibling Seçimi ────────────────────────────────

    fn find_best_sibling(&self, leaf_aabb: &Aabb) -> usize {
        let mut best        = self.root;
        let mut best_cost   = leaf_aabb.merge(self.nodes[self.root].aabb).surface_area();
        let leaf_sa         = leaf_aabb.surface_area();

        let mut stack = Vec::with_capacity(32);
        stack.push((self.root, 0.0f32));

        while let Some((idx, inherited_cost)) = stack.pop() {
            if idx == NULL { continue; }
            let combined_sa = leaf_aabb.merge(self.nodes[idx].aabb).surface_area();
            let direct_cost = combined_sa + inherited_cost;

            if direct_cost < best_cost {
                best_cost = direct_cost;
                best      = idx;
            }

            if !self.nodes[idx].is_leaf() {
                let child_inherited = (combined_sa - self.nodes[idx].aabb.surface_area()) + inherited_cost;
                let lower_bound     = leaf_sa + child_inherited;
                if lower_bound < best_cost {
                    stack.push((self.nodes[idx].left,  child_inherited));
                    stack.push((self.nodes[idx].right, child_inherited));
                }
            }
        }
        best
    }

    // ── AVL Rotasyon ─────────────────────────────────────────────────────────

    fn balance(&mut self, a: usize) -> usize {
        if self.nodes[a].is_leaf() || self.nodes[a].height < 2 {
            return a;
        }

        let b = self.nodes[a].left;
        let c = self.nodes[a].right;

        let balance = self.nodes[c].height - self.nodes[b].height;

        if balance > 1 {
            // Sağa döndür (C'yi yükselt)
            let f = self.nodes[c].left;
            let g = self.nodes[c].right;
            self.nodes[c].left  = a;
            self.nodes[c].parent = self.nodes[a].parent;
            self.nodes[a].parent = c;

            if self.nodes[c].parent != NULL {
                let cp = self.nodes[c].parent;
                if self.nodes[cp].left == a {
                    self.nodes[cp].left  = c;
                } else {
                    self.nodes[cp].right = c;
                }
            } else {
                self.root = c;
            }

            if self.nodes[f].height > self.nodes[g].height {
                self.nodes[c].right = f;
                self.nodes[a].right = g;
                self.nodes[g].parent = a;
                self.nodes[a].aabb   = self.nodes[b].aabb.merge(self.nodes[g].aabb);
                self.nodes[c].aabb   = self.nodes[a].aabb.merge(self.nodes[f].aabb);
                self.nodes[a].height = 1 + self.nodes[b].height.max(self.nodes[g].height);
                self.nodes[c].height = 1 + self.nodes[a].height.max(self.nodes[f].height);
            } else {
                self.nodes[c].right = g;
                self.nodes[a].right = f;
                self.nodes[f].parent = a;
                self.nodes[a].aabb   = self.nodes[b].aabb.merge(self.nodes[f].aabb);
                self.nodes[c].aabb   = self.nodes[a].aabb.merge(self.nodes[g].aabb);
                self.nodes[a].height = 1 + self.nodes[b].height.max(self.nodes[f].height);
                self.nodes[c].height = 1 + self.nodes[a].height.max(self.nodes[g].height);
            }
            return c;
        }

        if balance < -1 {
            // Sola döndür (B'yi yükselt)
            let d = self.nodes[b].left;
            let e = self.nodes[b].right;
            self.nodes[b].left  = a;
            self.nodes[b].parent = self.nodes[a].parent;
            self.nodes[a].parent = b;

            if self.nodes[b].parent != NULL {
                let bp = self.nodes[b].parent;
                if self.nodes[bp].left == a {
                    self.nodes[bp].left  = b;
                } else {
                    self.nodes[bp].right = b;
                }
            } else {
                self.root = b;
            }

            if self.nodes[d].height > self.nodes[e].height {
                self.nodes[b].right = d;
                self.nodes[a].left  = e;
                self.nodes[e].parent = a;
                self.nodes[a].aabb   = self.nodes[c].aabb.merge(self.nodes[e].aabb);
                self.nodes[b].aabb   = self.nodes[a].aabb.merge(self.nodes[d].aabb);
                self.nodes[a].height = 1 + self.nodes[c].height.max(self.nodes[e].height);
                self.nodes[b].height = 1 + self.nodes[a].height.max(self.nodes[d].height);
            } else {
                self.nodes[b].right = e;
                self.nodes[a].left  = d;
                self.nodes[d].parent = a;
                self.nodes[a].aabb   = self.nodes[c].aabb.merge(self.nodes[d].aabb);
                self.nodes[b].aabb   = self.nodes[a].aabb.merge(self.nodes[e].aabb);
                self.nodes[a].height = 1 + self.nodes[c].height.max(self.nodes[d].height);
                self.nodes[b].height = 1 + self.nodes[a].height.max(self.nodes[e].height);
            }
            return b;
        }

        a
    }

    // ── Sorgular ─────────────────────────────────────────────────────────────

    /// Tüm olası çarpışma çiftlerini döndür (yaprak-yaprak çiftleri, AABB overlap)
    pub fn query_pairs(&self) -> Vec<(Entity, Entity)> {
        let mut pairs = Vec::new();
        if self.root == NULL { return pairs; }
        
        let mut single_stack = Vec::with_capacity(64);
        single_stack.push(self.root);
        
        let mut pair_stack = Vec::with_capacity(128);
        
        while let Some(node_idx) = single_stack.pop() {
            let node = &self.nodes[node_idx];
            if !node.is_leaf() {
                single_stack.push(node.left);
                single_stack.push(node.right);
                pair_stack.push((node.left, node.right));
            }
        }
        
        while let Some((a_idx, b_idx)) = pair_stack.pop() {
            if a_idx == NULL || b_idx == NULL { continue; }
            let a_node = &self.nodes[a_idx];
            let b_node = &self.nodes[b_idx];
            
            if !a_node.aabb.intersects(b_node.aabb) { continue; }
            
            if a_node.is_leaf() && b_node.is_leaf() {
                if let (Some(ea), Some(eb)) = (a_node.entity, b_node.entity) {
                    let pair = if ea.id() < eb.id() { (ea, eb) } else { (eb, ea) };
                    pairs.push(pair);
                }
                continue;
            }
            
            if b_node.is_leaf() || (!a_node.is_leaf() && a_node.aabb.surface_area() >= b_node.aabb.surface_area()) {
                pair_stack.push((a_node.left, b_idx));
                pair_stack.push((a_node.right, b_idx));
            } else {
                pair_stack.push((a_idx, b_node.left));
                pair_stack.push((a_idx, b_node.right));
            }
        }
        
        pairs
    }

    /// Bir AABB ile kesişen tüm entity'leri döndür
    pub fn query_aabb(&self, aabb: &Aabb) -> Vec<Entity> {
        let mut result = Vec::new();
        if self.root == NULL { return result; }
        
        let mut stack = Vec::with_capacity(64);
        stack.push(self.root);
        
        while let Some(idx) = stack.pop() {
            if self.nodes[idx].aabb.intersects(*aabb) {
                if self.nodes[idx].is_leaf() {
                    if let Some(e) = self.nodes[idx].entity { result.push(e); }
                } else {
                    stack.push(self.nodes[idx].left);
                    stack.push(self.nodes[idx].right);
                }
            }
        }
        
        result
    }

    /// Bir ray ile kesişen en yakın entity'yi döndür (t değeri ile)
    pub fn query_ray(&self, origin: Vec3, dir: Vec3, max_t: f32) -> Vec<(Entity, f32)> {
        let mut result = Vec::new();
        if self.root == NULL { return result; }
        let mut stack = vec![self.root];
        while let Some(idx) = stack.pop() {
            if let Some(t) = ray_aabb(origin, dir, &self.nodes[idx].aabb, max_t) {
                if self.nodes[idx].is_leaf() {
                    if let Some(e) = self.nodes[idx].entity { result.push((e, t)); }
                } else {
                    stack.push(self.nodes[idx].left);
                    stack.push(self.nodes[idx].right);
                }
            }
        }
        result.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        result
    }

    pub fn entity_count(&self) -> usize { self.entity_map.len() }
}

fn ray_aabb(origin: Vec3, dir: Vec3, aabb: &Aabb, max_t: f32) -> Option<f32> {
    let mut tmin = 0.0f32;
    let mut tmax = max_t;
    for i in 0..3 {
        let (o, d, mn, mx) = match i {
            0 => (origin.x, dir.x, aabb.min.x, aabb.max.x),
            1 => (origin.y, dir.y, aabb.min.y, aabb.max.y),
            _ => (origin.z, dir.z, aabb.min.z, aabb.max.z),
        };
        if d.abs() < 1e-8 {
            if o < mn || o > mx { return None; }
        } else {
            let inv = 1.0 / d;
            let t1 = (mn - o) * inv;
            let t2 = (mx - o) * inv;
            tmin = tmin.max(t1.min(t2));
            tmax = tmax.min(t1.max(t2));
            if tmin > tmax { return None; }
        }
    }
    Some(tmin)
}

// ─────────────────────────────────────────────────────────────────────────────
// Geriye Dönük Uyum: SpatialHash typedef
// (world.rs'de SpatialHash kullanılıyor; typedef ile geçişi kolaylaştır)
// ─────────────────────────────────────────────────────────────────────────────

/// Eski SpatialHash API'sini Dynamic BVH üzerine köprüle
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

    pub fn clear(&self) {
        // SpatialHash &self alıyordu — BVH &mut self istiyor
        // world.rs'de zaten `&mut self.spatial_hash` olarak çağrılıyor, bu yüzden
        // buradaki clear'ı `&mut self` yapıyoruz ama trait değişmeden kalıyor
    }

    pub fn clear_mut(&mut self) {
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
        let tiny = Aabb {
            min: Vec3::new(point.x - radius, point.y - radius, point.z - radius).into(),
            max: Vec3::new(point.x + radius, point.y + radius, point.z + radius).into(),
        };
        self.tree.query_aabb(&tiny)
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

    fn make_aabb(cx: f32, cy: f32, cz: f32, r: f32) -> Aabb {
        Aabb::from_center_half_extents(Vec3::new(cx, cy, cz), Vec3::splat(r))
    }

    #[test]
    fn test_bvh_insert_query_pairs() {
        let mut tree = DynamicAabbTree::new();
        let e1 = Entity::new(1, 0);
        let e2 = Entity::new(2, 0);
        let e3 = Entity::new(3, 0);

        tree.insert(e1, make_aabb(0.0, 0.0, 0.0, 1.0));
        tree.insert(e2, make_aabb(1.0, 0.0, 0.0, 1.0)); // Overlaps e1
        tree.insert(e3, make_aabb(100.0, 0.0, 0.0, 1.0)); // Far away

        let pairs = tree.query_pairs();
        assert!(pairs.iter().any(|&(a, b)| (a == e1 && b == e2) || (a == e2 && b == e1)));
        assert!(!pairs.iter().any(|&(a, b)| (a == e1 && b == e3) || (a == e3 && b == e1)));
    }

    #[test]
    fn test_bvh_remove() {
        let mut tree = DynamicAabbTree::new();
        let e1 = Entity::new(1, 0);
        let e2 = Entity::new(2, 0);
        tree.insert(e1, make_aabb(0.0, 0.0, 0.0, 1.0));
        tree.insert(e2, make_aabb(0.5, 0.0, 0.0, 1.0));
        tree.remove(e1);
        let pairs = tree.query_pairs();
        assert!(pairs.is_empty());
    }

    #[test]
    fn test_bvh_no_self_pair() {
        let mut tree = DynamicAabbTree::new();
        let e = Entity::new(1, 0);
        tree.insert(e, make_aabb(0.0, 0.0, 0.0, 1.0));
        assert!(tree.query_pairs().is_empty());
    }

    #[test]
    fn test_spatial_hash_compat() {
        let mut sh = SpatialHash::new(10.0);
        let e1 = Entity::new(1, 0);
        let e2 = Entity::new(2, 0);
        sh.insert(e1, make_aabb(0.0, 0.0, 0.0, 1.0));
        sh.insert(e2, make_aabb(1.0, 0.0, 0.0, 1.0));
        let pairs = sh.query_pairs();
        assert!(!pairs.is_empty());
    }
}
