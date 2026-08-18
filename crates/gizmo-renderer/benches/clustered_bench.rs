//! What clustered light assignment costs on the CPU, per frame.
//!
//! docs/ENGINE.md §3 records this cost — 0.047 ms at 8 lights up to 0.764 at 256 — and hangs a
//! real decision on it: *"a compute-shader build is the follow-up and its **trigger is a
//! profile** showing this"*. The measurement was taken by hand and nothing reproduced it, so the
//! trigger could not actually be pulled: to find out whether assignment had become the frame's
//! problem, you had to redo the experiment from scratch and hope you set it up the same way.
//!
//! This is that experiment, written down. It also joins CI's `cargo bench -- --test` smoke gate,
//! so the harness cannot quietly stop compiling between the times anyone looks at it.
//!
//! CPU-only: `assign_lights` is a pure function of the grid, the camera and the light list, which
//! is the property the same ENGINE.md paragraph gives as the reason it is on the CPU at all
//! ("the whole assignment is a pure function with nine unit tests").
//!
//! The light counts are the ones already quoted, so a new run is directly comparable with the
//! recorded numbers rather than being a fresh baseline nobody can line up against.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use gizmo_math::{Mat4, Vec3};
use gizmo_renderer::clustered::{assign_lights, ClusterGrid, ClusterView, LightSphere};

/// The counts ENGINE.md quotes, so a re-run is a comparison and not a new baseline.
const LIGHT_COUNTS: [usize; 5] = [8, 32, 64, 128, 256];

fn a_view() -> ClusterView {
    let camera_pos = Vec3::new(0.0, 6.0, 24.0);
    let forward = Vec3::new(0.0, 0.0, -1.0);
    let (near, far) = (0.3_f32, 500.0_f32);
    let proj = Mat4::perspective_rh(0.9, 16.0 / 9.0, near, far);
    let view = Mat4::look_at_rh(camera_pos, camera_pos + forward, Vec3::Y);
    ClusterView {
        view_proj: proj * view,
        camera_pos,
        forward,
        near,
        far,
    }
}

/// Lights spread through the view volume rather than piled at one depth: a light's cost is the
/// number of clusters its sphere touches, so a bunched-up set would measure the wrong thing —
/// either all-overlapping (worst case) or all-disjoint (best), and neither is a scene.
fn lights(n: usize) -> Vec<LightSphere> {
    (0..n)
        .map(|i| {
            let t = i as f32 / n.max(1) as f32;
            let ring = t * std::f32::consts::TAU * 3.0;
            LightSphere {
                center: Vec3::new(
                    ring.cos() * (4.0 + t * 30.0),
                    1.0 + (i % 5) as f32 * 1.5,
                    -10.0 - t * 220.0,
                ),
                // A mix of small props and large fills, which is what decides how many clusters
                // each light lands in.
                radius: 3.0 + (i % 4) as f32 * 6.0,
            }
        })
        .collect()
}

fn bench_assignment(c: &mut Criterion) {
    let grid = ClusterGrid::default(); // 16×9×24, the grid the shaders index with
    let view = a_view();

    let mut group = c.benchmark_group("clustered/assign");
    for n in LIGHT_COUNTS {
        let set = lights(n);
        group.bench_with_input(BenchmarkId::from_parameter(n), &set, |b, set| {
            b.iter(|| assign_lights(grid, view, std::hint::black_box(set)));
        });
    }
    group.finish();
}

criterion_group!(benches, bench_assignment);
criterion_main!(benches);
