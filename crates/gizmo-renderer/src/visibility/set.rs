//! A frame-stamped membership set over dense `u32` keys.

/// "Is this key in this frame's candidate set?" in one array read, cleared in O(1).
///
/// A `Vec<bool>` would have to be zeroed every frame — 8 k writes to throw away, before the
/// cull has done anything. Instead each slot holds the frame number it was last inserted on
/// and [`begin_frame`](Self::begin_frame) bumps a counter, so clearing touches no memory at
/// all.
///
/// Keys must be small and dense: the backing `Vec` grows to `max(key) + 1`, the same contract
/// [`RenderAabbTree`](super::RenderAabbTree) imposes.
///
/// Deliberately minimal — one flat set, no per-frustum breakdown. Distinguishing
/// "camera-visible" from "shadow-caster-visible" is what
/// [`classify_visibility_world`](crate::classify_visibility_world) is for, and it still runs on
/// every candidate; a second, subtly different copy of that decision living here is exactly the
/// drift this whole design is built to avoid.
#[derive(Debug, Clone, Default)]
pub struct VisibleSet {
    /// `key -> the frame counter value it was last inserted on`. `0` means "never".
    stamp: Vec<u32>,
    /// Current frame counter. Starts at 0, so a fresh set matches nothing until the first
    /// [`begin_frame`](Self::begin_frame).
    frame: u32,
    /// The keys inserted this frame, in insertion order.
    touched: Vec<u32>,
}

impl VisibleSet {
    /// An empty set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Start a new frame: everything inserted before this call stops matching.
    ///
    /// O(1) in the common case. The exception is counter wraparound: `frame` is a `u32`, and
    /// at ~60 fps it takes about two years and three months of uptime to exhaust — but "about
    /// two years" is not "never", and the failure it would produce is a stale stamp read as
    /// fresh, i.e. a mesh that should have been culled being drawn, or the reverse. So when
    /// the counter is about to wrap, the stamp array is zeroed once and the counter restarts.
    /// That is one memset roughly every 4 billion frames.
    pub fn begin_frame(&mut self) {
        self.touched.clear();
        match self.frame.checked_add(1) {
            Some(next) => self.frame = next,
            None => {
                for s in &mut self.stamp {
                    *s = 0;
                }
                self.frame = 1;
            }
        }
    }

    /// Insert every key in `keys` into the current frame's set.
    pub fn insert_all(&mut self, keys: &[u32]) {
        if let Some(&max) = keys.iter().max() {
            let need = max as usize + 1;
            if self.stamp.len() < need {
                self.stamp.resize(need, 0);
            }
        }
        for &k in keys {
            let slot = &mut self.stamp[k as usize];
            if *slot != self.frame {
                *slot = self.frame;
                self.touched.push(k);
            }
        }
    }

    /// Was this key inserted in the current frame?
    #[inline]
    pub fn contains(&self, key: u32) -> bool {
        self.stamp
            .get(key as usize)
            .is_some_and(|&s| s == self.frame)
    }

    /// The keys inserted this frame, in insertion order, without duplicates.
    #[inline]
    pub fn keys(&self) -> &[u32] {
        &self.touched
    }

    /// How many distinct keys are in this frame's set.
    #[inline]
    pub fn len(&self) -> usize {
        self.touched.len()
    }

    /// Is this frame's set empty?
    #[inline]
    pub fn is_empty(&self) -> bool {
        self.touched.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// T13 — a new frame forgets the old one without writing to the stamp array.
    #[test]
    fn begin_frame_clears_without_touching_memory() {
        let mut s = VisibleSet::new();
        s.begin_frame();
        s.insert_all(&[3, 7, 11]);
        assert!(s.contains(7));
        assert_eq!(s.len(), 3);

        // The proof that no memory was touched: the backing array does not change identity or
        // size across the clear, and the old stamps are still physically there — they simply
        // no longer equal the current frame.
        let ptr_before = s.stamp.as_ptr();
        let len_before = s.stamp.len();
        s.begin_frame();
        assert_eq!(s.stamp.as_ptr(), ptr_before, "begin_frame reallocated");
        assert_eq!(s.stamp.len(), len_before, "begin_frame resized");
        assert!(!s.contains(7));
        assert!(s.is_empty());
        assert_eq!(s.keys(), &[] as &[u32]);
    }

    /// Duplicate keys in one frame are collapsed — `keys()` is a set, not a bag. The union of
    /// five frusta genuinely produces repeats, and a caller sizing a buffer off `len()` would
    /// otherwise over-allocate silently.
    #[test]
    fn inserting_a_key_twice_in_one_frame_lists_it_once() {
        let mut s = VisibleSet::new();
        s.begin_frame();
        s.insert_all(&[4, 4, 9]);
        s.insert_all(&[9, 4]);
        assert_eq!(s.len(), 2);
        assert_eq!(s.keys(), &[4, 9]);
    }

    /// T14 — the classic stamped-set bug. Drive the counter to `u32::MAX` and past it: a stamp
    /// written long ago must not be read as belonging to the current frame.
    ///
    /// A false positive here draws a mesh that should have been culled; a false negative hides
    /// one that should be on screen. Both are silent.
    #[test]
    fn stamp_wraparound_is_safe() {
        let mut s = VisibleSet::new();
        s.begin_frame();
        s.insert_all(&[1, 2, 3]);

        // Jump to the last frame the counter can represent, and stamp a key there.
        s.frame = u32::MAX;
        s.touched.clear();
        s.insert_all(&[2]);
        assert!(s.contains(2));
        assert!(!s.contains(1), "key 1 was stamped on an older frame");

        // The wrap.
        s.begin_frame();
        assert_eq!(s.frame, 1, "counter must restart, not wrap onto a live stamp");
        assert!(
            !s.contains(2),
            "the u32::MAX stamp must not survive the wrap and read as frame 1"
        );
        assert!(!s.contains(1));
        assert!(!s.contains(3));

        // And the set still works afterwards.
        s.insert_all(&[3]);
        assert!(s.contains(3));
        assert!(!s.contains(2));
    }

    /// An out-of-range key is simply absent, not a panic — the guard in the batching loop asks
    /// about every entity it sees, including ones that were never indexed.
    #[test]
    fn a_key_beyond_the_backing_array_is_absent_not_a_panic() {
        let mut s = VisibleSet::new();
        s.begin_frame();
        s.insert_all(&[2]);
        assert!(!s.contains(9_999));
        assert!(!s.contains(u32::MAX));
    }
}
