use super::*;
use rustc_hash::FxHashMap;

#[derive(Clone)]
struct Node {
    aabb: Aabb,
    parent: usize,
    left: usize,
    right: usize,
    entity: Option<BodyHandle>, // Sadece yaprak node'larda dolu
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
            aabb: Aabb {
                min: Vec3::ZERO.into(),
                max: Vec3::ZERO.into(),
            },
            parent: NULL,
            left: NULL,
            right: NULL,
            entity: None,
            height: -1,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Dynamic AABB Tree
// ─────────────────────────────────────────────────────────────────────────────

/// An incremental BVH over moving bodies: one leaf per entity, internal nodes
/// holding the merged bounds of their children.
///
/// Bodies are keyed by [`BodyHandle::id`] — two handles with the same id are the
/// same body here — and each leaf stores a **fattened** copy of the AABB it was
/// given. That margin is the whole point of the structure: a body that moves but
/// stays inside its fat box costs nothing to "update", and only one that escapes it
/// pays for a reinsertion. The price is that every query answers against the fat
/// boxes, so results are a conservative superset of the true overlaps.
///
/// Insertion picks its sibling with a branch-and-bound surface-area heuristic and
/// the tree is rebalanced with AVL rotations, so it stays shallow without a rebuild.
/// Node slots live in a `Vec` with a free list and are **reused after removal**; no
/// index into it is exposed, and none should be held.
pub struct DynamicAabbTree {
    nodes: Vec<Node>,
    root: usize,
    free_list: usize,
    pub(crate) entity_map: FxHashMap<u32, usize>,
    fat_margin: f32,
}

impl Default for DynamicAabbTree {
    fn default() -> Self {
        Self::new()
    }
}

impl DynamicAabbTree {
    /// An empty tree with a fattening margin of 0.1 world units (metres) and room
    /// preallocated for 256 nodes.
    ///
    /// Same as `Default`. Use [`with_fat_margin`](Self::with_fat_margin) to pick a
    /// different margin.
    pub fn new() -> Self {
        Self {
            nodes: Vec::with_capacity(256),
            root: NULL,
            free_list: NULL,
            entity_map: FxHashMap::default(),
            fat_margin: 0.1,
        }
    }

    /// Sets how far, in world units (metres), each leaf's stored box is grown on
    /// every face beyond the AABB it was given.
    ///
    /// This is the structure's one tuning knob, and it trades two costs against each
    /// other: a larger margin means fewer reinsertions as bodies move, but looser
    /// boxes and so more false-positive pairs for the narrowphase to reject. It only
    /// affects leaves inserted afterwards — leaves already in the tree keep the box
    /// they were built with until they are reinserted. A margin of 0 is legal and
    /// turns every movement into a reinsertion.
    pub fn with_fat_margin(mut self, margin: f32) -> Self {
        self.fat_margin = margin;
        self
    }

    /// Removes every entity and frees every node, leaving an empty tree.
    ///
    /// Keeps the allocated node capacity, so refilling costs no reallocation, and
    /// keeps the configured fat margin.
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.root = NULL;
        self.free_list = NULL;
        self.entity_map.clear();
    }

    /// How many distinct entity ids are in the tree — i.e. the number of leaves.
    ///
    /// Not the number of nodes: internal nodes roughly double that, and freed slots
    /// stay allocated. Re-inserting an id already present does not increase it.
    pub fn entity_count(&self) -> usize {
        self.entity_map.len()
    }

    // ── Node havuzu ──────────────────────────────────────────────────────────

    fn alloc_node(&mut self) -> usize {
        if self.free_list != NULL {
            let idx = self.free_list;
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

    /// Adds `entity` with its tight world-space `aabb`, or re-places it if that id is
    /// already in the tree. There is no separate update call.
    ///
    /// The stored box is `aabb` grown by the fat margin on every face. If the id is
    /// already present and its new tight box still fits inside the fat box already
    /// stored, this returns immediately and the tree is untouched — that early-out is
    /// what makes a per-frame "update every body" loop affordable. Otherwise the old
    /// leaf is removed and a new one inserted, which may rotate the tree.
    ///
    /// Nothing validates `aabb`: a non-finite or inverted box is fattened and stored
    /// as-is, and the merged bounds of its ancestors are then only as meaningful as
    /// it was.
    pub fn insert(&mut self, entity: BodyHandle, aabb: Aabb) {
        // FIX-1: tight AABB hâlâ fat AABB içindeyse rebuild'den kaçın
        // Skalar karşılaştırma — Vec3A cmpge/cmple trait sorununu önler
        if let Some(&node_idx) = self.entity_map.get(&entity.id()) {
            let fat = self.nodes[node_idx].aabb;
            if aabb_contains(&fat, &aabb) {
                return;
            }
            self.remove(entity);
        }

        let fat_aabb = fatten(&aabb, self.fat_margin);

        let leaf = self.alloc_node();
        self.nodes[leaf].aabb = fat_aabb;
        self.nodes[leaf].entity = Some(entity);
        self.nodes[leaf].height = 0;

        self.insert_leaf(leaf);
        self.entity_map.insert(entity.id(), leaf);
    }

    /// Removes the entity's leaf and returns its node slot to the free list.
    ///
    /// A no-op if that id is not in the tree. The freed slots are reused by later
    /// insertions, so nothing outside may retain node indices across this call.
    pub fn remove(&mut self, entity: BodyHandle) {
        if let Some(leaf) = self.entity_map.remove(&entity.id()) {
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

        let leaf_aabb = self.nodes[leaf].aabb;
        let sibling = self.find_best_sibling(&leaf_aabb);

        let old_parent = self.nodes[sibling].parent;
        let new_parent = self.alloc_node();

        self.nodes[new_parent].parent = old_parent;
        self.nodes[new_parent].aabb = merge_aabb(&leaf_aabb, &self.nodes[sibling].aabb);
        self.nodes[new_parent].height = self.nodes[sibling].height + 1;
        self.nodes[new_parent].left = sibling;
        self.nodes[new_parent].right = leaf;

        if old_parent != NULL {
            if self.nodes[old_parent].left == sibling {
                self.nodes[old_parent].left = new_parent;
            } else {
                self.nodes[old_parent].right = new_parent;
            }
        } else {
            self.root = new_parent;
        }

        self.nodes[sibling].parent = new_parent;
        self.nodes[leaf].parent = new_parent;

        self.refit_ancestors(self.nodes[leaf].parent);
    }

    fn remove_leaf(&mut self, leaf: usize) {
        if leaf == self.root {
            self.root = NULL;
            return;
        }

        let parent = self.nodes[leaf].parent;
        let sibling = if self.nodes[parent].left == leaf {
            self.nodes[parent].right
        } else {
            self.nodes[parent].left
        };

        let grand = self.nodes[parent].parent;

        if grand != NULL {
            if self.nodes[grand].left == parent {
                self.nodes[grand].left = sibling;
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
            let left = self.nodes[index].left;
            let right = self.nodes[index].right;

            self.nodes[index].height = 1 + self.nodes[left].height.max(self.nodes[right].height);
            self.nodes[index].aabb = merge_aabb(&self.nodes[left].aabb, &self.nodes[right].aabb);

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
        let mut best = self.root;

        // Stack: (node_idx, inherited_cost)
        let mut stack = Vec::with_capacity(32);
        stack.push((self.root, 0.0f32));

        while let Some((idx, inherited)) = stack.pop() {
            if idx == NULL {
                continue;
            }

            let node_sa = surface_area(&self.nodes[idx].aabb);
            let merged_sa = surface_area(&merge_aabb(leaf_aabb, &self.nodes[idx].aabb));
            let direct = merged_sa;
            let total_cost = direct + inherited;

            if total_cost < best_cost {
                best_cost = total_cost;
                best = idx;
            }

            if !self.nodes[idx].is_leaf() {
                // FIX-7: child'ın inherited cost'u = parent'ın (merged_sa - node_sa) + inherited
                // Bu, yaprağı bu node'un altına eklersek ancestor AABB'lerinin ne kadar büyüyeceğini gösterir
                let child_inherited = (merged_sa - node_sa) + inherited;

                // Lower bound: leaf_sa + child_inherited (node SA = 0 olsa bile en az bu kadar maliyet)
                let lower_bound = surface_area(leaf_aabb) + child_inherited;
                if lower_bound < best_cost {
                    stack.push((self.nodes[idx].left, child_inherited));
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
        self.nodes[c].left = a;
        self.nodes[c].parent = self.nodes[a].parent;
        self.nodes[a].parent = c;

        // C'nin eski parent'ını güncelle
        let cp = self.nodes[c].parent;
        if cp != NULL {
            if self.nodes[cp].left == a {
                self.nodes[cp].left = c;
            } else {
                self.nodes[cp].right = c;
            }
        } else {
            self.root = c;
        }

        // F ve G'den hangisi daha yüksek?
        if self.nodes[f].height > self.nodes[g].height {
            // G → A'nın sağına, F → C'nin sağına
            self.nodes[c].right = f;
            self.nodes[a].right = g;
            // FIX-3: g'nin parent pointer'ını güncelle
            self.nodes[g].parent = a;
            self.nodes[f].parent = c; // f zaten c'nin çocuğu kalıyor ama parent'ı güncelle

            self.nodes[a].aabb = merge_aabb(&self.nodes[b].aabb, &self.nodes[g].aabb);
            self.nodes[c].aabb = merge_aabb(&self.nodes[a].aabb, &self.nodes[f].aabb);
            self.nodes[a].height = 1 + self.nodes[b].height.max(self.nodes[g].height);
            self.nodes[c].height = 1 + self.nodes[a].height.max(self.nodes[f].height);
        } else {
            // F → A'nın sağına, G → C'nin sağına
            self.nodes[c].right = g;
            self.nodes[a].right = f;
            // FIX-3: f'nin parent pointer'ını güncelle
            self.nodes[f].parent = a;
            self.nodes[g].parent = c;

            self.nodes[a].aabb = merge_aabb(&self.nodes[b].aabb, &self.nodes[f].aabb);
            self.nodes[c].aabb = merge_aabb(&self.nodes[a].aabb, &self.nodes[g].aabb);
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
        self.nodes[b].left = a;
        self.nodes[b].parent = self.nodes[a].parent;
        self.nodes[a].parent = b;

        let bp = self.nodes[b].parent;
        if bp != NULL {
            if self.nodes[bp].left == a {
                self.nodes[bp].left = b;
            } else {
                self.nodes[bp].right = b;
            }
        } else {
            self.root = b;
        }

        if self.nodes[d].height > self.nodes[e].height {
            // E → A'nın soluna, D → B'nin sağına
            self.nodes[b].right = d;
            self.nodes[a].left = e;
            // FIX-3: parent pointer güncelle
            self.nodes[e].parent = a;
            self.nodes[d].parent = b;

            self.nodes[a].aabb = merge_aabb(&self.nodes[c].aabb, &self.nodes[e].aabb);
            self.nodes[b].aabb = merge_aabb(&self.nodes[a].aabb, &self.nodes[d].aabb);
            self.nodes[a].height = 1 + self.nodes[c].height.max(self.nodes[e].height);
            self.nodes[b].height = 1 + self.nodes[a].height.max(self.nodes[d].height);
        } else {
            // D → A'nın soluna, E → B'nin sağına
            self.nodes[b].right = e;
            self.nodes[a].left = d;
            // FIX-3: parent pointer güncelle
            self.nodes[d].parent = a;
            self.nodes[e].parent = b;

            self.nodes[a].aabb = merge_aabb(&self.nodes[c].aabb, &self.nodes[d].aabb);
            self.nodes[b].aabb = merge_aabb(&self.nodes[a].aabb, &self.nodes[e].aabb);
            self.nodes[a].height = 1 + self.nodes[c].height.max(self.nodes[d].height);
            self.nodes[b].height = 1 + self.nodes[a].height.max(self.nodes[e].height);
        }

        b
    }

    // ── Sorgular ─────────────────────────────────────────────────────────────

    /// Tüm olası çarpışma çiftlerini döndür.
    /// FIX-2: Dual-tree descent ile garantili duplicate-free, self-pair yok.
    /// Algoritma: her internal node için sol ve sağ alt ağaçları birbirine karşı test et.
    ///
    /// Every pair of entities whose **fattened** boxes overlap, each pair reported
    /// exactly once, never a body with itself. Each tuple is ordered by id
    /// (`.0.id() < .1.id()`), so it can be used directly as a symmetric key.
    ///
    /// Uniqueness is structural rather than filtered: a colliding leaf pair has one
    /// lowest common ancestor, and only that node emits it, so nothing here
    /// de-duplicates afterwards. The order of the returned vector follows the tree's
    /// shape and is an implementation detail.
    pub fn query_pairs(&self) -> Vec<(BodyHandle, BodyHandle)> {
        let mut pairs = Vec::new();
        if self.root == NULL || self.nodes[self.root].is_leaf() {
            return pairs;
        }

        // Standard BVH self-query: for every internal node, test its LEFT subtree
        // against its RIGHT subtree (`collect_internal_pairs` → `descent_pair`).
        // Every colliding leaf pair has a unique lowest-common-ancestor internal
        // node, so this enumerates each pair EXACTLY ONCE — no dedup required.
        //
        // PERF: the previous implementation additionally ran a root-seeded dual-tree
        // descent first (phase 1) that produced the *same* pairs this phase does,
        // then paid an O(P²) `pairs.contains` linear scan in `descent_pair` to drop
        // the duplicates. Both are gone: pure phase-2 is complete and duplicate-free,
        // so query_pairs is now O(P) in the number of reported pairs instead of O(P²).
        // On a sustained-active 2000-box scene this took narrowphase (which owns the
        // query_pairs call) from ~170 ms to a fraction of that. See docs.
        pairs.reserve(self.entity_map.len());
        self.collect_internal_pairs(&mut pairs);

        pairs
    }

    /// Her internal node'un sol ve sağ çocuklarını birbirine karşı test et
    /// (aynı subtree içindeki çiftler için)
    fn collect_internal_pairs(&self, pairs: &mut Vec<(BodyHandle, BodyHandle)>) {
        if self.root == NULL {
            return;
        }
        let mut stack = vec![self.root];
        while let Some(idx) = stack.pop() {
            if self.nodes[idx].is_leaf() {
                continue;
            }
            let l = self.nodes[idx].left;
            let r = self.nodes[idx].right;
            // Sol ve sağ alt ağaçları birbirine karşı test et
            self.descent_pair(l, r, pairs);
            stack.push(l);
            stack.push(r);
        }
    }

    fn descent_pair(&self, a: usize, b: usize, pairs: &mut Vec<(BodyHandle, BodyHandle)>) {
        if a == NULL || b == NULL {
            return;
        }
        if !aabb_overlaps(&self.nodes[a].aabb, &self.nodes[b].aabb) {
            return;
        }

        let a_leaf = self.nodes[a].is_leaf();
        let b_leaf = self.nodes[b].is_leaf();

        match (a_leaf, b_leaf) {
            (true, true) => {
                if let (Some(ea), Some(eb)) = (self.nodes[a].entity, self.nodes[b].entity) {
                    let pair = if ea.id() < eb.id() {
                        (ea, eb)
                    } else {
                        (eb, ea)
                    };
                    // Each pair reaches this arm exactly once (unique LCA), so no
                    // `pairs.contains` dedup is needed — that was the old O(P²) cost.
                    pairs.push(pair);
                }
            }
            (true, false) => {
                self.descent_pair(a, self.nodes[b].left, pairs);
                self.descent_pair(a, self.nodes[b].right, pairs);
            }
            (false, true) => {
                self.descent_pair(self.nodes[a].left, b, pairs);
                self.descent_pair(self.nodes[a].right, b, pairs);
            }
            (false, false) => {
                if surface_area(&self.nodes[a].aabb) >= surface_area(&self.nodes[b].aabb) {
                    self.descent_pair(self.nodes[a].left, b, pairs);
                    self.descent_pair(self.nodes[a].right, b, pairs);
                } else {
                    self.descent_pair(a, self.nodes[b].left, pairs);
                    self.descent_pair(a, self.nodes[b].right, pairs);
                }
            }
        }
    }

    /// Verilen AABB ile örtüşen tüm entity'leri döndür
    ///
    /// Entities whose **fattened** boxes overlap `aabb`, touching included. Each
    /// entity appears at most once; the order is the traversal's and carries no
    /// meaning.
    pub fn query_aabb(&self, aabb: &Aabb) -> Vec<BodyHandle> {
        let mut result = Vec::new();
        if self.root == NULL {
            return result;
        }

        let mut stack = vec![self.root];
        while let Some(idx) = stack.pop() {
            if !aabb_overlaps(&self.nodes[idx].aabb, aabb) {
                continue;
            }
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
    ///
    /// Entities whose **fattened** boxes the ray reaches no later than `max_t`, each
    /// with the parameter at which the ray enters that box, sorted ascending by it.
    /// A ray starting inside a box gets `0.0` for it — the value is clamped, never
    /// negative.
    ///
    /// These are box-entry parameters, not surface hits: the real hit on a body lies
    /// at or beyond the reported `t`. The sort makes near candidates cheap to try
    /// first, but it does not let a caller stop at the first exact hit — only at one
    /// that is nearer than the next candidate's entry `t`.
    ///
    /// `t` and `max_t` are in units of `dir`'s length; pass a unit vector to work in
    /// metres. Direction components below 1e-8 become an infinite slab inverse rather
    /// than a parallel special case, so a ray whose origin sits exactly on such a
    /// slab boundary is a degenerate case with no promised answer.
    pub fn query_ray(&self, origin: Vec3, dir: Vec3, max_t: f32) -> Vec<(BodyHandle, f32)> {
        let mut result = Vec::new();
        if self.root == NULL {
            return result;
        }

        let inv_dir = Vec3::new(
            if dir.x.abs() > 1e-8 {
                1.0 / dir.x
            } else {
                f32::INFINITY
            },
            if dir.y.abs() > 1e-8 {
                1.0 / dir.y
            } else {
                f32::INFINITY
            },
            if dir.z.abs() > 1e-8 {
                1.0 / dir.z
            } else {
                f32::INFINITY
            },
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
    ///
    /// Walks the whole tree asserting its internal invariants: parent back-pointers,
    /// non-negative heights, every leaf carrying an entity at height 0, and every
    /// internal node's height being one above its taller child.
    ///
    /// **Panics** on the first violation — that is the entire point, it is a debug
    /// tool, not a predicate. It exists only under `debug_assertions`, so code that
    /// calls it needs the same `cfg`, and it recurses over the tree's depth.
    /// Note that it checks structure, not geometry: it does not verify that a
    /// parent's box actually encloses its children's.
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

