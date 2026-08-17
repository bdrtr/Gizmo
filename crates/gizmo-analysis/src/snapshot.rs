//! The structural snapshot of a single frame.
//!
//! Everything observable about the engine in one frame: ECS statistics, the detailed archetype
//! table, timestamped spans (from the FrameProfiler) and the free-form metric groups the
//! collectors add. The `groups` field is what lets any subsystem add its own detail without
//! changing the snapshot type.

use gizmo_core::world::{ArchetypeSummary, WorldStats};
use std::collections::BTreeMap;

/// The analysis-side copy of one FrameProfiler scope (including nanoseconds, for the Chrome
/// trace).
#[derive(Debug, Clone)]
pub struct SpanSample {
    /// The scope's name, as `profile_scope!` was given it.
    pub name: &'static str,
    /// How long the scope was open, in milliseconds — the same number the profiler shows.
    pub ms: f64,
    /// Nesting depth, 0 for a scope opened at the top of the frame. What makes a flame chart a
    /// chart rather than a list.
    pub depth: u32,
    /// Start, in nanoseconds since the frame's own epoch — the Chrome trace's `ts`.
    pub start_ns: u64,
    /// End, in the same units. `end_ns - start_ns` is `ms` without the rounding.
    pub end_ns: u64,
}

/// The complete snapshot of one frame.
#[derive(Debug, Clone, Default)]
pub struct FrameSnapshot {
    /// The frame number, counting from zero.
    pub frame: u64,
    /// This frame's total duration in ms — from the FrameProfiler where possible, otherwise
    /// the wall clock.
    pub frame_ms: f64,
    /// Time elapsed since the Analyzer's epoch (ns) — for the time axis.
    pub timestamp_ns: u64,
    /// Top-level ECS statistics.
    pub ecs: WorldStats,
    /// The detailed archetype table (can be empty depending on the config — it is heavy).
    pub archetypes: Vec<ArchetypeSummary>,
    /// The profiling spans that closed during this frame; they may be nested.
    pub spans: Vec<SpanSample>,
    /// The free-form metric groups the collectors add: group → [(metric, value)].
    /// For example "physics" → [("bodies", 1281.0), ("solver_ms", 4.1), …].
    pub groups: BTreeMap<String, Vec<(String, f64)>>,
}

impl FrameSnapshot {
    /// Adds a value to a metric group, creating the group if it is not there. This is what
    /// collectors use.
    pub fn push_metric(&mut self, group: &str, name: &str, value: f64) {
        self.groups
            .entry(group.to_string())
            .or_default()
            .push((name.to_string(), value));
    }

    /// Reads a group+metric value, if it exists.
    pub fn metric(&self, group: &str, name: &str) -> Option<f64> {
        self.groups
            .get(group)
            .and_then(|g| g.iter().find(|(n, _)| n == name).map(|(_, v)| *v))
    }

    /// The most expensive span (ms) — a quick bottleneck indicator.
    pub fn hottest_span(&self) -> Option<&SpanSample> {
        self.spans
            .iter()
            .max_by(|a, b| a.ms.partial_cmp(&b.ms).unwrap_or(std::cmp::Ordering::Equal))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(name: &'static str, ms: f64) -> SpanSample {
        SpanSample { name, ms, depth: 0, start_ns: 0, end_ns: 0 }
    }

    #[test]
    fn default_snapshot_is_empty() {
        let s = FrameSnapshot::default();
        assert_eq!(s.frame, 0);
        assert_eq!(s.frame_ms, 0.0);
        assert!(s.groups.is_empty());
        assert!(s.spans.is_empty());
        assert!(s.hottest_span().is_none());
        assert_eq!(s.ecs, WorldStats::default());
    }

    #[test]
    fn push_metric_creates_group_then_appends_in_order() {
        let mut s = FrameSnapshot::default();
        s.push_metric("physics", "bodies", 5.0);
        s.push_metric("physics", "contacts", 2.0);
        let g = &s.groups["physics"];
        // Insertion order is preserved within a group (it's a Vec, not a map).
        assert_eq!(g, &vec![("bodies".to_string(), 5.0), ("contacts".to_string(), 2.0)]);
    }

    #[test]
    fn metric_lookup_hits_misses_and_first_write_wins() {
        let mut s = FrameSnapshot::default();
        s.push_metric("physics", "bodies", 5.0);
        assert_eq!(s.metric("physics", "bodies"), Some(5.0));
        assert_eq!(s.metric("physics", "nope"), None, "missing name → None");
        assert_eq!(s.metric("render", "bodies"), None, "missing group → None");

        // Duplicate name in the same group: `metric` returns the FIRST occurrence.
        s.push_metric("physics", "bodies", 99.0);
        assert_eq!(s.metric("physics", "bodies"), Some(5.0));
    }

    #[test]
    fn groups_iterate_in_sorted_order() {
        // `groups` is a BTreeMap → deterministic, sorted key order regardless of
        // insertion sequence (important for stable JSON/CSV export).
        let mut s = FrameSnapshot::default();
        s.push_metric("zebra", "a", 1.0);
        s.push_metric("alpha", "b", 2.0);
        s.push_metric("mid", "c", 3.0);
        let keys: Vec<&str> = s.groups.keys().map(|k| k.as_str()).collect();
        assert_eq!(keys, vec!["alpha", "mid", "zebra"]);
    }

    #[test]
    fn hottest_span_picks_max_ms() {
        let s = FrameSnapshot {
            spans: vec![span("a", 1.0), span("b", 5.0), span("c", 3.0)],
            ..Default::default()
        };
        assert_eq!(s.hottest_span().unwrap().name, "b");
    }

    #[test]
    fn hottest_span_is_nan_tolerant() {
        // A NaN duration must not panic the comparator; a finite span is still returned.
        let s = FrameSnapshot {
            spans: vec![span("bad", f64::NAN), span("good", 2.0)],
            ..Default::default()
        };
        let h = s.hottest_span().expect("must return a span, not panic");
        // Comparator maps NaN to Equal; the finite maximum "good" is reachable.
        assert!(h.name == "good" || h.name == "bad");
    }
}
