//! The incremental fat-margin BVH. See the [module docs](super) for what it is for and how it
//! differs from the physics broadphase tree.

use gizmo_math::{Aabb, Frustum, Intersection, Vec3};

/// The one key value [`RenderAabbTree`] cannot store.
///
/// Keys must be small dense `u32` — the index allocates `Vec`s sized `max(key) + 1`, the same
/// contract `gizmo-core`'s `World` already imposes on entity ids, which is what lets this
/// structure avoid a hash map entirely. `u32::MAX` doubles as the tree's internal "no node"
/// and "no key" sentinel, so it is reserved.
pub const NO_KEY: u32 = u32::MAX;

/// Internal node/slot sentinel. Same value as [`NO_KEY`]; named separately because it means a
/// different thing (an absent node index, not an absent entity key).
const NIL: u32 = u32::MAX;

/// Traversal marker: "an ancestor tested `Inside`, so emit everything below without testing".
///
/// `Frustum::FULL_MASK` is `0b0011_1111`, so the two high bits are unreachable as a real plane
/// mask and `0xFF` cannot collide with one.
const ACCEPT: u8 = 0xFF;

/// One BVH node. Leaves carry a `key` and no children; internal nodes carry two children and
/// [`NO_KEY`].
///
/// 32 B of `Aabb` + four `u32` + one `i32` = 52 B, rounded to 64 by `Vec3A`'s 16-byte
/// alignment: exactly one cache line, which is why `key: u32 + sentinel` is used instead of
/// an `Option<Key>` that would cost 8.
#[derive(Clone, Copy)]
struct Node {
    aabb: Aabb,
    parent: u32,
    left: u32,
    right: u32,
    key: u32,
    /// `-1` marks a slot on the free list.
    height: i32,
}

impl Node {
    #[inline]
    fn is_leaf(&self) -> bool {
        self.left == NIL
    }
}

impl Default for Node {
    fn default() -> Self {
        Self {
            aabb: Aabb::new(Vec3::ZERO, Vec3::ZERO),
            parent: NIL,
            left: NIL,
            right: NIL,
            key: NO_KEY,
            height: -1,
        }
    }
}

// ── Box helpers ─────────────────────────────────────────────────────────────
//
// Local, private, and scalar on purpose. `Aabb::surface_area` guards `is_empty()` and returns
// 0.0; for SAH descent an empty box scoring 0 is the *best* possible sibling, which is exactly
// backwards, so the SAH cost function below uses its own unguarded form and the tree simply
// never stores an empty box (see `insert`).

#[inline]
fn surface_area(a: &Aabb) -> f32 {
    let d = a.max - a.min;
    2.0 * (d.x * d.y + d.y * d.z + d.z * d.x)
}

#[inline]
fn merge(a: &Aabb, b: &Aabb) -> Aabb {
    Aabb {
        min: a.min.min(b.min),
        max: a.max.max(b.max),
    }
}

#[inline]
fn fatten(a: &Aabb, margin: f32) -> Aabb {
    let m = gizmo_math::Vec3A::splat(margin);
    Aabb {
        min: a.min - m,
        max: a.max + m,
    }
}

/// Is `inner` wholly within `outer`? Inclusive on every face, so a box that exactly fills its
/// fat box still counts as contained and still early-outs.
#[inline]
fn contains(outer: &Aabb, inner: &Aabb) -> bool {
    inner.min.x >= outer.min.x
        && inner.min.y >= outer.min.y
        && inner.min.z >= outer.min.z
        && inner.max.x <= outer.max.x
        && inner.max.y <= outer.max.y
        && inner.max.z <= outer.max.z
}

#[inline]
fn overlaps(a: &Aabb, b: &Aabb) -> bool {
    a.min.x <= b.max.x
        && b.min.x <= a.max.x
        && a.min.y <= b.max.y
        && b.min.y <= a.max.y
        && a.min.z <= b.max.z
        && b.min.z <= a.max.z
}

// ── The tree ────────────────────────────────────────────────────────────────

/// An incremental BVH over world-space AABBs, keyed by small dense `u32`.
///
/// One leaf per key; internal nodes hold the merged bounds of their children. Insertion picks
/// its sibling with a branch-and-bound surface-area heuristic and the tree is AVL-rebalanced,
/// so it stays shallow without ever being rebuilt.
///
/// Each leaf stores a **fattened** copy of the box it was given. That margin is the whole
/// point of the structure: [`insert`](Self::insert) on a key whose new box still fits inside
/// the fat box it already has returns immediately and touches nothing, so refreshing the
/// entire scene every frame costs one containment test per unmoved object. The price is that
/// every query answers against the fat boxes and is therefore a conservative **superset** —
/// which, for a skip filter in front of an exact test, is the safe direction and costs one
/// extra exact test per false positive.
///
/// Node slots live in a `Vec` with a free list and are reused after removal. No index into it
/// is exposed and none should be held.
#[derive(Clone)]
pub struct RenderAabbTree {
    nodes: Vec<Node>,
    root: u32,
    free_list: u32,
    /// `key -> leaf node index`, or [`NIL`]. Indexed directly by key; grows to `max(key) + 1`.
    leaf_of: Vec<u32>,
    /// Dense list of live keys, for [`len`](Self::len) / [`keys`](Self::keys) / iteration.
    keys: Vec<u32>,
    /// `key -> its position in `keys``, for O(1) swap-remove. [`NIL`] when absent.
    key_pos: Vec<u32>,
    fat_margin: f32,
}

impl Default for RenderAabbTree {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderAabbTree {
    /// The default fat margin, in world units (metres).
    ///
    /// One metre: large enough that a car at 30 m/s re-bins roughly every other frame instead
    /// of every frame, small enough that it barely widens the candidate set for the
    /// building-sized boxes that make up most of a map.
    pub const DEFAULT_FAT_MARGIN: f32 = 1.0;

    /// An empty tree with the [default fat margin](Self::DEFAULT_FAT_MARGIN).
    pub fn new() -> Self {
        Self::with_fat_margin(Self::DEFAULT_FAT_MARGIN)
    }

    /// An empty tree with an explicit fattening margin, in world units (metres).
    ///
    /// This is the structure's one tuning knob and it trades two costs against each other: a
    /// larger margin means fewer re-bins as objects move, but looser boxes and so more
    /// candidates for the exact test to reject. `0.0` is legal and makes every query exact —
    /// and, because containment is inclusive, an object that has not moved *at all* still
    /// early-outs even at zero margin, so the margin only ever buys anything for movers.
    ///
    /// A negative margin is clamped to `0.0`: a shrunken leaf box could exclude geometry that
    /// is really there, which is the one error class this structure exists to be incapable of.
    pub fn with_fat_margin(margin: f32) -> Self {
        Self {
            nodes: Vec::with_capacity(256),
            root: NIL,
            free_list: NIL,
            leaf_of: Vec::new(),
            keys: Vec::new(),
            key_pos: Vec::new(),
            fat_margin: if margin > 0.0 { margin } else { 0.0 },
        }
    }

    /// The configured fattening margin, in world units.
    #[inline]
    pub fn fat_margin(&self) -> f32 {
        self.fat_margin
    }

    /// Drops every key and frees every node, keeping the allocated capacity and the margin.
    pub fn clear(&mut self) {
        self.nodes.clear();
        self.root = NIL;
        self.free_list = NIL;
        self.leaf_of.fill(NIL);
        self.key_pos.fill(NIL);
        self.keys.clear();
    }

    /// How many keys are in the tree — i.e. the number of leaves, not the number of nodes.
    #[inline]
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// `true` when no key is in the tree.
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Is this key currently indexed?
    #[inline]
    pub fn contains(&self, key: u32) -> bool {
        self.leaf_of.get(key as usize).is_some_and(|&n| n != NIL)
    }

    /// Every live key, in an unspecified order that changes as keys are removed.
    #[inline]
    pub fn keys(&self) -> &[u32] {
        &self.keys
    }

    /// Height of the root: `0` for a single leaf, `0` for an empty tree.
    #[inline]
    pub fn height(&self) -> u32 {
        if self.root == NIL {
            0
        } else {
            self.nodes[self.root as usize].height.max(0) as u32
        }
    }

    // ── Node pool ───────────────────────────────────────────────────────────

    fn alloc_node(&mut self) -> u32 {
        if self.free_list != NIL {
            let idx = self.free_list;
            // The free list is threaded through `parent`.
            self.free_list = self.nodes[idx as usize].parent;
            self.nodes[idx as usize] = Node::default();
            idx
        } else {
            let idx = self.nodes.len() as u32;
            assert!(idx != NIL, "RenderAabbTree: node slot overflow");
            self.nodes.push(Node::default());
            idx
        }
    }

    fn free_node(&mut self, idx: u32) {
        self.nodes[idx as usize] = Node {
            parent: self.free_list,
            height: -1,
            ..Default::default()
        };
        self.free_list = idx;
    }

    // ── Insert / remove ─────────────────────────────────────────────────────

    /// Insert `key` with its tight world-space box, or re-place it if it is already indexed.
    /// There is no separate update call.
    ///
    /// The stored box is `world_aabb` grown by the [fat margin](Self::fat_margin). If the key
    /// is already present and its new tight box still fits the fat box already stored, this
    /// returns `false` and the tree is untouched — that early-out is what makes refreshing
    /// every renderable every frame affordable, and it is the single most load-bearing
    /// property of the structure. Otherwise the old leaf is removed and a new one inserted
    /// (which may rotate the tree) and this returns `true`.
    ///
    /// `world_aabb` must be the box of the transform the geometry is actually **drawn** with.
    /// For a camera-locked material that is not the authored matrix, and such materials should
    /// not be indexed at all — their drawn box moves with the camera every frame.
    ///
    /// An empty/inverted box, or the reserved [`NO_KEY`], is rejected: `false`, nothing stored.
    /// An empty box has no meaningful position, and a leaf holding one would poison every
    /// ancestor's merged bounds. `Aabb::empty()` is what
    /// [`Aabb::from_points`](gizmo_math::Aabb::from_points) returns for a mesh with no
    /// vertices, so this is a *data* condition a shipping game can hit, not a programmer
    /// error — hence a quiet `false` rather than a panic.
    ///
    /// # `false` is ambiguous, and the ambiguity is load-bearing
    ///
    /// This returns `false` for **two unrelated things**: "already inside its fat box, nothing
    /// to do" — the hot path, most keys, most frames — and "refused, nothing stored". A caller
    /// cannot tell them apart from the return value and must not try. Ask
    /// [`contains`](Self::contains) instead, and write the cull guard so that a key the index
    /// does not hold is *drawn*, never skipped (see the [module docs](super)). A refused key
    /// under a candidate-list loop is geometry that never appears again.
    pub fn insert(&mut self, key: u32, world_aabb: Aabb) -> bool {
        if key == NO_KEY {
            debug_assert!(false, "RenderAabbTree: NO_KEY is reserved and cannot be indexed");
            return false;
        }
        if world_aabb.is_empty() {
            return false;
        }

        if let Some(&leaf) = self.leaf_of.get(key as usize) {
            if leaf != NIL {
                if contains(&self.nodes[leaf as usize].aabb, &world_aabb) {
                    return false;
                }
                self.remove(key);
            }
        }

        let fat = fatten(&world_aabb, self.fat_margin);
        let leaf = self.alloc_node();
        self.nodes[leaf as usize].aabb = fat;
        self.nodes[leaf as usize].key = key;
        self.nodes[leaf as usize].height = 0;
        self.insert_leaf(leaf);

        let k = key as usize;
        if self.leaf_of.len() <= k {
            self.leaf_of.resize(k + 1, NIL);
            self.key_pos.resize(k + 1, NIL);
        }
        self.leaf_of[k] = leaf;
        self.key_pos[k] = self.keys.len() as u32;
        self.keys.push(key);
        true
    }

    /// Remove a key. `false` if it was not indexed.
    pub fn remove(&mut self, key: u32) -> bool {
        let k = key as usize;
        let leaf = match self.leaf_of.get(k) {
            Some(&l) if l != NIL => l,
            _ => return false,
        };
        self.remove_leaf(leaf);
        self.free_node(leaf);
        self.leaf_of[k] = NIL;

        // O(1) swap-remove out of the dense key list.
        let pos = self.key_pos[k] as usize;
        self.key_pos[k] = NIL;
        let last = self.keys.len() - 1;
        self.keys.swap_remove(pos);
        if pos != last {
            let moved = self.keys[pos];
            self.key_pos[moved as usize] = pos as u32;
        }
        true
    }

    /// Drop every key `keep` rejects, and report how many went.
    ///
    /// This is the reconciliation half of holding the tree across frames:
    /// [`insert`](Self::insert) refreshes what is still there cheaply, and this evicts what is
    /// gone. Preferred over a despawn hook precisely because it cannot desync — it re-derives
    /// membership from whatever `keep` consults rather than trusting a stream of events.
    pub fn retain(&mut self, keep: impl Fn(u32) -> bool) -> usize {
        let stale: Vec<u32> = self.keys.iter().copied().filter(|&k| !keep(k)).collect();
        for &k in &stale {
            self.remove(k);
        }
        stale.len()
    }

    // ── Leaf plumbing ───────────────────────────────────────────────────────

    fn insert_leaf(&mut self, leaf: u32) {
        if self.root == NIL {
            self.root = leaf;
            self.nodes[leaf as usize].parent = NIL;
            return;
        }

        let leaf_aabb = self.nodes[leaf as usize].aabb;
        let sibling = self.find_best_sibling(&leaf_aabb);

        let old_parent = self.nodes[sibling as usize].parent;
        let new_parent = self.alloc_node();

        self.nodes[new_parent as usize].parent = old_parent;
        self.nodes[new_parent as usize].aabb = merge(&leaf_aabb, &self.nodes[sibling as usize].aabb);
        self.nodes[new_parent as usize].height = self.nodes[sibling as usize].height + 1;
        self.nodes[new_parent as usize].left = sibling;
        self.nodes[new_parent as usize].right = leaf;

        if old_parent != NIL {
            if self.nodes[old_parent as usize].left == sibling {
                self.nodes[old_parent as usize].left = new_parent;
            } else {
                self.nodes[old_parent as usize].right = new_parent;
            }
        } else {
            self.root = new_parent;
        }

        self.nodes[sibling as usize].parent = new_parent;
        self.nodes[leaf as usize].parent = new_parent;

        self.refit_ancestors(new_parent);
    }

    fn remove_leaf(&mut self, leaf: u32) {
        if leaf == self.root {
            self.root = NIL;
            return;
        }

        let parent = self.nodes[leaf as usize].parent;
        let sibling = if self.nodes[parent as usize].left == leaf {
            self.nodes[parent as usize].right
        } else {
            self.nodes[parent as usize].left
        };
        let grand = self.nodes[parent as usize].parent;

        if grand != NIL {
            if self.nodes[grand as usize].left == parent {
                self.nodes[grand as usize].left = sibling;
            } else {
                self.nodes[grand as usize].right = sibling;
            }
            self.nodes[sibling as usize].parent = grand;
            self.free_node(parent);
            self.refit_ancestors(grand);
        } else {
            self.root = sibling;
            self.nodes[sibling as usize].parent = NIL;
            self.free_node(parent);
        }
    }

    /// Recompute height + merged bounds from `index` to the root, AVL-balancing on the way up.
    fn refit_ancestors(&mut self, mut index: u32) {
        while index != NIL {
            let left = self.nodes[index as usize].left;
            let right = self.nodes[index as usize].right;
            self.nodes[index as usize].height =
                1 + self.nodes[left as usize].height.max(self.nodes[right as usize].height);
            self.nodes[index as usize].aabb =
                merge(&self.nodes[left as usize].aabb, &self.nodes[right as usize].aabb);
            index = self.balance(index);
            index = self.nodes[index as usize].parent;
        }
    }

    /// Greedy surface-area-heuristic descent: from the root, keep going down whichever child
    /// makes the tree grow least, and stop as soon as pairing with the current node beats both.
    ///
    /// **This is the fourth deliberate divergence from the physics broadphase tree**, which
    /// does a branch-and-bound search over the whole tree instead. Measured on an 8 192-mesh
    /// scene, that search cost ~0.8 µs per insert and made the *refresh* the dominant term of
    /// the whole cull — 286 µs a frame against a 152 µs linear cull, i.e. the index lost to
    /// the thing it replaced, entirely on insert cost. This descent is `O(height)` — about
    /// sixteen levels, seven box operations each — and the tree it produces measures the same:
    /// same height, same candidate count, same traversal time. Physics inserts far fewer
    /// bodies per frame than a renderer refreshes meshes, so the trade lands differently there
    /// and that tree is right to keep its search.
    fn find_best_sibling(&self, leaf_aabb: &Aabb) -> u32 {
        let mut index = self.root;
        while !self.nodes[index as usize].is_leaf() {
            let node = &self.nodes[index as usize];
            let (c1, c2) = (node.left, node.right);

            let area = surface_area(&node.aabb);
            let combined = surface_area(&merge(&node.aabb, leaf_aabb));
            // Cost of stopping here: a new parent holding this node and the leaf.
            let cost_here = 2.0 * combined;
            // What every ancestor of this node grows by if the leaf goes anywhere below it.
            let inheritance = 2.0 * (combined - area);

            let descend_cost = |child: u32| {
                let ch = &self.nodes[child as usize];
                let merged = surface_area(&merge(leaf_aabb, &ch.aabb));
                if ch.is_leaf() {
                    merged + inheritance
                } else {
                    (merged - surface_area(&ch.aabb)) + inheritance
                }
            };
            let cost1 = descend_cost(c1);
            let cost2 = descend_cost(c2);

            if cost_here < cost1 && cost_here < cost2 {
                break;
            }
            index = if cost1 < cost2 { c1 } else { c2 };
        }
        index
    }

    // ── AVL rotation ────────────────────────────────────────────────────────

    fn balance(&mut self, a: u32) -> u32 {
        if self.nodes[a as usize].is_leaf() || self.nodes[a as usize].height < 2 {
            return a;
        }
        let b = self.nodes[a as usize].left;
        let c = self.nodes[a as usize].right;
        let bf = self.nodes[c as usize].height - self.nodes[b as usize].height;
        if bf > 1 {
            return self.rotate_left(a, b, c);
        }
        if bf < -1 {
            return self.rotate_right(a, b, c);
        }
        a
    }

    /// Pull A's right child C up.
    fn rotate_left(&mut self, a: u32, b: u32, c: u32) -> u32 {
        let f = self.nodes[c as usize].left;
        let g = self.nodes[c as usize].right;

        self.nodes[c as usize].left = a;
        self.nodes[c as usize].parent = self.nodes[a as usize].parent;
        self.nodes[a as usize].parent = c;

        let cp = self.nodes[c as usize].parent;
        if cp != NIL {
            if self.nodes[cp as usize].left == a {
                self.nodes[cp as usize].left = c;
            } else {
                self.nodes[cp as usize].right = c;
            }
        } else {
            self.root = c;
        }

        let (up, down) = if self.nodes[f as usize].height > self.nodes[g as usize].height {
            (f, g)
        } else {
            (g, f)
        };
        self.nodes[c as usize].right = up;
        self.nodes[a as usize].right = down;
        self.nodes[up as usize].parent = c;
        self.nodes[down as usize].parent = a;

        self.nodes[a as usize].aabb = merge(&self.nodes[b as usize].aabb, &self.nodes[down as usize].aabb);
        self.nodes[c as usize].aabb = merge(&self.nodes[a as usize].aabb, &self.nodes[up as usize].aabb);
        self.nodes[a as usize].height =
            1 + self.nodes[b as usize].height.max(self.nodes[down as usize].height);
        self.nodes[c as usize].height =
            1 + self.nodes[a as usize].height.max(self.nodes[up as usize].height);
        c
    }

    /// Pull A's left child B up.
    fn rotate_right(&mut self, a: u32, b: u32, c: u32) -> u32 {
        let d = self.nodes[b as usize].left;
        let e = self.nodes[b as usize].right;

        self.nodes[b as usize].left = a;
        self.nodes[b as usize].parent = self.nodes[a as usize].parent;
        self.nodes[a as usize].parent = b;

        let bp = self.nodes[b as usize].parent;
        if bp != NIL {
            if self.nodes[bp as usize].left == a {
                self.nodes[bp as usize].left = b;
            } else {
                self.nodes[bp as usize].right = b;
            }
        } else {
            self.root = b;
        }

        let (up, down) = if self.nodes[d as usize].height > self.nodes[e as usize].height {
            (d, e)
        } else {
            (e, d)
        };
        self.nodes[b as usize].right = up;
        self.nodes[a as usize].left = down;
        self.nodes[up as usize].parent = b;
        self.nodes[down as usize].parent = a;

        self.nodes[a as usize].aabb = merge(&self.nodes[c as usize].aabb, &self.nodes[down as usize].aabb);
        self.nodes[b as usize].aabb = merge(&self.nodes[a as usize].aabb, &self.nodes[up as usize].aabb);
        self.nodes[a as usize].height =
            1 + self.nodes[c as usize].height.max(self.nodes[down as usize].height);
        self.nodes[b as usize].height =
            1 + self.nodes[a as usize].height.max(self.nodes[up as usize].height);
        b
    }

    // ── Queries ─────────────────────────────────────────────────────────────

    /// Append every key whose stored (fattened) box is not wholly outside `frustum`.
    ///
    /// A conservative **superset** of the visible set: never a false negative, sometimes a
    /// false positive. Callers must still apply their exact test — this is a skip filter, not
    /// a decision.
    ///
    /// This is the caller [`Frustum::test_aabb_masked`] was written for. The mask starts at
    /// [`Frustum::FULL_MASK`] and is reduced on descent, so a subtree that is already fully
    /// inside (say) the near and left planes never tests them again; a node that comes back
    /// `Inside` accepts its whole subtree with zero further plane tests.
    ///
    /// `out` is appended to, not cleared. Keys are unique within one call — each leaf is
    /// reached exactly once — but calling this twice with different frusta can repeat keys;
    /// use [`query_frusta`](Self::query_frusta) when you want the deduplicated union.
    pub fn query_frustum(&self, frustum: &Frustum, out: &mut Vec<u32>) {
        if self.root == NIL {
            return;
        }
        let mut stack: Vec<(u32, u8)> = Vec::with_capacity(2 * self.height() as usize + 4);
        stack.push((self.root, Frustum::FULL_MASK));
        while let Some((idx, mask)) = stack.pop() {
            if mask == ACCEPT {
                self.collect_subtree(idx, out, &mut stack);
                continue;
            }
            let n = &self.nodes[idx as usize];
            match frustum.test_aabb_masked(n.aabb, mask) {
                (Intersection::Outside, _) => {}
                (Intersection::Inside, _) => self.collect_subtree(idx, out, &mut stack),
                (Intersection::Partial, reduced) => {
                    if n.is_leaf() {
                        out.push(n.key);
                    } else {
                        stack.push((n.left, reduced));
                        stack.push((n.right, reduced));
                    }
                }
            }
        }
    }

    /// The same traversal with the plane mask disabled — every node tested against all six
    /// planes, no reduction propagated.
    ///
    /// Exists so tests and benchmarks can hold the mask up against a reference: it must
    /// produce the *same set* as [`query_frustum`](Self::query_frustum) (that is the
    /// correctness claim) while doing strictly more work (that is the performance claim).
    /// Nothing in the engine should call this in anger.
    pub fn query_frustum_full_mask(&self, frustum: &Frustum, out: &mut Vec<u32>) {
        if self.root == NIL {
            return;
        }
        let mut stack: Vec<(u32, u8)> = Vec::with_capacity(2 * self.height() as usize + 4);
        stack.push((self.root, Frustum::FULL_MASK));
        while let Some((idx, mask)) = stack.pop() {
            if mask == ACCEPT {
                self.collect_subtree(idx, out, &mut stack);
                continue;
            }
            let n = &self.nodes[idx as usize];
            match frustum.test_aabb_masked(n.aabb, Frustum::FULL_MASK) {
                (Intersection::Outside, _) => {}
                (Intersection::Inside, _) => self.collect_subtree(idx, out, &mut stack),
                (Intersection::Partial, _) => {
                    if n.is_leaf() {
                        out.push(n.key);
                    } else {
                        stack.push((n.left, Frustum::FULL_MASK));
                        stack.push((n.right, Frustum::FULL_MASK));
                    }
                }
            }
        }
    }

    /// Emit `idx` if it is a leaf, otherwise schedule both children for emission — no plane
    /// tests anywhere below.
    ///
    /// Reached only when an ancestor came back `Inside`, at which point every descendant is
    /// inside too and further tests can only reproduce an answer already known. It shares the
    /// traversal's stack rather than allocating its own, which also bounds the stack at the
    /// tree's height instead of a subtree's leaf count.
    ///
    /// This is a pointer chase with no arithmetic in it. A build-once tree laid out
    /// depth-first could make an accepted subtree one contiguous `extend_from_slice`; a
    /// free-list tree scatters subtrees across the node `Vec` and cannot. That is a real cost
    /// of choosing the incremental structure — paid against six plane tests per leaf saved,
    /// which is still the better side of the trade.
    fn collect_subtree(&self, idx: u32, out: &mut Vec<u32>, stack: &mut Vec<(u32, u8)>) {
        let n = &self.nodes[idx as usize];
        if n.is_leaf() {
            out.push(n.key);
        } else {
            stack.push((n.left, ACCEPT));
            stack.push((n.right, ACCEPT));
        }
    }

    /// The deduplicated union of [`query_frustum`](Self::query_frustum) over several frusta —
    /// a camera plus its shadow cascades, typically.
    ///
    /// `out` is cleared first, and comes back sorted ascending. Sorted is not incidental: the
    /// consumer walks archetype rows in id order and probes this set, so ascending keys are
    /// the cache-friendly order to have produced them in.
    pub fn query_frusta(&self, frusta: &[Frustum], out: &mut Vec<u32>) {
        out.clear();
        for f in frusta {
            self.query_frustum(f, out);
        }
        out.sort_unstable();
        out.dedup();
    }

    /// Append every key whose stored (fattened) box overlaps `aabb`, touching included.
    ///
    /// For tools, debug overlays and tests. Same superset semantics as the frustum queries.
    /// `out` is appended to, not cleared.
    pub fn query_aabb(&self, aabb: &Aabb, out: &mut Vec<u32>) {
        if self.root == NIL {
            return;
        }
        let mut stack: Vec<u32> = Vec::with_capacity(2 * self.height() as usize + 4);
        stack.push(self.root);
        while let Some(idx) = stack.pop() {
            let n = &self.nodes[idx as usize];
            if !overlaps(&n.aabb, aabb) {
                continue;
            }
            if n.is_leaf() {
                out.push(n.key);
            } else {
                stack.push(n.left);
                stack.push(n.right);
            }
        }
    }

    /// The stored (fattened) box for a key, if it is indexed. Debug/test accessor.
    pub fn leaf_aabb(&self, key: u32) -> Option<Aabb> {
        match self.leaf_of.get(key as usize) {
            Some(&l) if l != NIL => Some(self.nodes[l as usize].aabb),
            _ => None,
        }
    }

    // ── Debug ───────────────────────────────────────────────────────────────

    /// Assert every internal invariant, **panicking** on the first violation.
    ///
    /// Structure — parent back-pointers, heights, leaves carrying a key at height 0, internal
    /// nodes carrying [`NO_KEY`] and two children, the key maps agreeing with the leaves —
    /// **and geometry**: every internal node's box must actually enclose both children's.
    ///
    /// That last one is the difference from the physics broadphase's `validate`, which
    /// documents that it skips it. Behind a pair query a bad refit is a missed contact and
    /// the next frame fixes it. Behind a cull query it is a subtree that reports `Outside`
    /// while holding visible geometry — objects that silently stop being drawn, with nothing
    /// to notice it. Debug builds only.
    #[cfg(debug_assertions)]
    pub fn validate(&self) {
        assert_eq!(
            self.keys.len(),
            self.keys.iter().collect::<std::collections::HashSet<_>>().len(),
            "duplicate key in the dense key list"
        );
        for (pos, &k) in self.keys.iter().enumerate() {
            assert_eq!(self.key_pos[k as usize] as usize, pos, "key_pos disagrees for key {k}");
            let leaf = self.leaf_of[k as usize];
            assert!(leaf != NIL, "key {k} is listed live but has no leaf");
            assert_eq!(self.nodes[leaf as usize].key, k, "leaf {leaf} holds the wrong key");
        }
        if self.root != NIL {
            let seen = self.validate_node(self.root, NIL);
            assert_eq!(seen, self.keys.len(), "reachable leaves != live keys");
        } else {
            assert!(self.keys.is_empty(), "empty tree with live keys");
        }
    }

    /// Returns the number of leaves under `idx`.
    #[cfg(debug_assertions)]
    fn validate_node(&self, idx: u32, expected_parent: u32) -> usize {
        let n = &self.nodes[idx as usize];
        assert_eq!(n.parent, expected_parent, "node {idx}: wrong parent");
        assert!(n.height >= 0, "node {idx}: live node on the free list");

        if n.is_leaf() {
            assert_eq!(n.height, 0, "leaf {idx}: height must be 0");
            assert_ne!(n.key, NO_KEY, "leaf {idx}: no key");
            assert_eq!(self.leaf_of[n.key as usize], idx, "leaf {idx}: leaf_of disagrees");
            assert_eq!(n.right, NIL, "leaf {idx}: half a child");
            return 1;
        }

        let (l, r) = (n.left, n.right);
        assert!(l != NIL && r != NIL, "internal node {idx}: missing a child");
        assert_eq!(n.key, NO_KEY, "internal node {idx}: carries a key");
        assert_eq!(
            n.height,
            1 + self.nodes[l as usize].height.max(self.nodes[r as usize].height),
            "node {idx}: wrong height"
        );
        // The geometric invariant the whole cull rests on.
        for child in [l, r] {
            assert!(
                contains(&n.aabb, &self.nodes[child as usize].aabb),
                "node {idx} does not enclose child {child}: a subtree can now report Outside \
                 while holding visible geometry"
            );
        }
        self.validate_node(l, idx) + self.validate_node(r, idx)
    }

    /// Corrupt a leaf's stored box, for the test that proves [`validate`](Self::validate)
    /// actually checks the geometric invariant rather than merely claiming to.
    #[cfg(test)]
    fn corrupt_node_aabb(&mut self, key: u32, aabb: Aabb) {
        let leaf = self.leaf_of[key as usize];
        self.nodes[leaf as usize].aabb = aabb;
    }
}

#[cfg(test)]
mod tests;
