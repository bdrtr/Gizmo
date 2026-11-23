//! A GPU soft-body step is the CPU's step: run side by side on the same bodies, the two keep
//! together.
//!
//! `GpuCompute` computes the element forces on the GPU and ends the step in the CPU's own
//! integrator, so the only difference left is the rounding of the force pass. Until 2026-11-23 it
//! had an integrator of its own in the shader, and each test here is one place where that one
//! went its own way:
//! - two bodies of different materials in one step (it gave both the first one's);
//! - a body below y = 0.1 m (the shader had a floor there that the world did not).
//!
//! Both tests are red on that code: run against it, the stiff block springs back at the soft
//! block's pace, and the falling block stops in mid-air at 0.1 m.
//!
//! The tests need a GPU and skip, saying so, where there is none (CI). Each one holds this
//! binary's GPU lock and the machine-wide one, as every GPU test in the workspace does.

#![cfg(feature = "gpu_physics")]

use gizmo_math::Vec3;
use gizmo_physics_core::{BodyHandle, Collider, Transform};
use gizmo_physics_soft::gpu_compute::GpuCompute;
use gizmo_physics_soft::SoftBodyMesh;

/// Serialises this binary's GPU tests and takes the machine-wide GPU test lock — see
/// `gizmo-renderer/src/test_gpu.rs` for why both are needed. The file name is the one every GPU
/// test binary in the workspace uses.
fn gpu_lock() -> std::sync::MutexGuard<'static, ()> {
    static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let guard = GPU.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    static HELD: std::sync::OnceLock<Option<std::fs::File>> = std::sync::OnceLock::new();
    HELD.get_or_init(|| {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(std::env::temp_dir().join("gizmo-gpu-tests.lock"))
            .ok()?;
        file.lock().ok()?;
        Some(file)
    });
    guard
}

fn gpu() -> Option<GpuCompute> {
    match pollster::block_on(GpuCompute::new()) {
        Ok(gpu) => Some(gpu),
        Err(e) => {
            eprintln!("skipped: no GPU to step on ({e})");
            None
        }
    }
}

type Body = (BodyHandle, SoftBodyMesh, Transform);

/// A 1 m block of 2×2×2 cells and 8 kg with its lowest corner at `min`, stretched by 30 % along x
/// after its rest shape was taken, so it springs back.
fn stretched_block(min: Vec3, youngs_modulus: f32) -> SoftBodyMesh {
    let mut sb = SoftBodyMesh::block(min, Vec3::ONE, [2, 2, 2], 8.0, youngs_modulus, 0.3).expect("a valid block");
    for node in &mut sb.nodes {
        node.position.x = min.x + (node.position.x - min.x) * 1.3;
    }
    sb
}

/// Steps the same bodies on the CPU and on the GPU, `frames` steps of `dt` cut into `substeps`,
/// and returns the two results.
fn both(gpu: &mut GpuCompute, bodies: Vec<SoftBodyMesh>, colliders: &[(BodyHandle, Transform, Collider)], gravity: Vec3, frames: usize) -> (Vec<SoftBodyMesh>, Vec<SoftBodyMesh>) {
    let dt = 1.0 / 60.0;
    let substeps = bodies.iter().map(|b| b.stable_substeps(dt)).max().unwrap_or(1);
    let h = dt / substeps as f32;
    let mut cpu = bodies.clone();
    let mut on_gpu: Vec<Body> = bodies.into_iter().map(|b| (BodyHandle::from_id(0), b, Transform::default())).collect();
    for _ in 0..frames * substeps {
        for b in &mut cpu {
            b.step(h, gravity, colliders);
        }
        gpu.step_soft_bodies(&mut on_gpu, colliders, h, gravity).expect("a GPU step");
    }
    (cpu, on_gpu.into_iter().map(|(_, b, _)| b).collect())
}

/// The largest distance between a node on one side and the same node on the other.
fn apart(a: &SoftBodyMesh, b: &SoftBodyMesh) -> f32 {
    a.nodes.iter().zip(&b.nodes).map(|(a, b)| a.position.distance(b.position)).fold(0.0, f32::max)
}

/// How far apart the two paths may end while nothing touches anything. Measured on this machine:
/// 0.7 µm (the soft block), 5.2 µm (the stiff one) and 0.5 µm (half a second of free fall).
const TOLERANCE: f32 = 1e-4;

/// The same once the bodies have touched something. A contact is a yes or a no, so a node that
/// the two paths' rounding puts on either side of it is pushed out by one path a sub-step before
/// the other, and from there they part by millimetres: measured, under 20 µm through the landing
/// and the first bounces, then 2.9 mm at the first such flip and 5.2 mm a second later. Still
/// three metres short of the old shader's floor.
const AFTER_CONTACT: f32 = 2e-2;

/// A soft and a stiff block, stepped together in one call: each springs back at its own
/// material's pace, as on the CPU.
#[test]
fn two_materials_in_one_step_each_keep_their_own() {
    let _gpu = gpu_lock();
    let Some(mut gpu) = gpu() else { return };
    let soft = stretched_block(Vec3::new(0.0, 2.0, 0.0), 1.0e3);
    let stiff = stretched_block(Vec3::new(4.0, 2.0, 0.0), 2.0e4);
    let (cpu, on_gpu) = both(&mut gpu, vec![soft.clone(), stiff.clone()], &[], Vec3::ZERO, 20);
    for (i, name) in ["soft", "stiff"].into_iter().enumerate() {
        let gap = apart(&cpu[i], &on_gpu[i]);
        eprintln!("{name}: {gap} m apart");
        assert!(gap < TOLERANCE, "the {name} block is {gap} m from the CPU's");
    }
    // The control: the stiff block given the soft one's material — what the old shader did — ends
    // far from where it should, so the comparison above can tell the two apart.
    let mut wrong = stiff.clone();
    wrong.mu = soft.mu;
    wrong.lambda = soft.lambda;
    let (as_soft, _) = both(&mut gpu, vec![wrong], &[], Vec3::ZERO, 20);
    let control = apart(&as_soft[0], &cpu[1]);
    assert!(control > 100.0 * TOLERANCE, "a stiff block with a soft material ends only {control} m from the right one");
}

/// A block falling from below y = 0.1 m onto a floor at y = −3: it falls past where the shader had
/// a floor as on the CPU, and lands on the real one.
#[test]
fn a_block_falls_past_y_0_1_onto_the_floor_there_is() {
    let _gpu = gpu_lock();
    let Some(mut gpu) = gpu() else { return };
    let floor = (
        BodyHandle::from_id(1),
        Transform::new(Vec3::new(0.0, -3.5, 0.0)),
        Collider::box_collider(Vec3::new(10.0, 0.5, 10.0)),
    );
    let block = SoftBodyMesh::block(Vec3::new(0.0, -1.0, 0.0), Vec3::ONE, [2, 2, 2], 8.0, 2.0e3, 0.3).expect("a valid block");
    let gravity = Vec3::new(0.0, -9.81, 0.0);
    // Half a second: falling, a metre below where the old floor was, not yet down.
    let (cpu, on_gpu) = both(&mut gpu, vec![block.clone()], std::slice::from_ref(&floor), gravity, 30);
    let gap = apart(&cpu[0], &on_gpu[0]);
    let lowest = on_gpu[0].nodes.iter().map(|n| n.position.y).fold(f32::INFINITY, f32::min);
    eprintln!("falling block: {gap} m apart, lowest node at {lowest}");
    assert!(lowest < -2.0, "half a second down, the block is still falling; its lowest node is at {lowest}");
    assert!(gap < TOLERANCE, "in the air the GPU's block is {gap} m from the CPU's");
    // A second and a half: down, and bouncing on the floor.
    let (cpu, on_gpu) = both(&mut gpu, vec![block], &[floor], gravity, 90);
    let gap = apart(&cpu[0], &on_gpu[0]);
    let lowest = on_gpu[0].nodes.iter().map(|n| n.position.y).fold(f32::INFINITY, f32::min);
    eprintln!("landed block: {gap} m apart, lowest node at {lowest}");
    assert!((lowest - (-3.0)).abs() < 0.1, "the block is on the floor at y = −3; its lowest node is at {lowest}");
    assert!(gap < AFTER_CONTACT, "on the floor the GPU's block is {gap} m from the CPU's");
}
