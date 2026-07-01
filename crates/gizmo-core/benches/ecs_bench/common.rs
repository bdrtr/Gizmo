use gizmo_core::{
    component::{Component, Bundle, StorageType},
    world::World,
    entity::Entity,
};

#[derive(Clone, Copy)]
pub struct Mat4(pub [f32; 16]);

impl Mat4 {
    pub const ONE: Self = Self([1.0; 16]);
    pub const ZERO: Self = Self([0.0; 16]);
}

#[derive(Clone, Copy)]
pub struct Vec3(pub [f32; 3]);

impl Vec3 {
    pub const ONE: Self = Self([1.0; 3]);
    pub const ZERO: Self = Self([0.0; 3]);
}

#[derive(Clone, Copy)]
pub struct Transform(pub Mat4);
impl Component for Transform {}

#[derive(Clone, Copy)]
pub struct Position(pub Vec3);
impl Component for Position {}

#[derive(Clone, Copy)]
pub struct Rotation(pub Vec3);
impl Component for Rotation {}

#[derive(Clone, Copy)]
pub struct Velocity(pub Vec3);
impl Component for Velocity {}

// 1. SparseSet Benchmark (High churn)
#[derive(Clone, Copy)]
pub struct A(pub f32);
impl Component for A {}

#[derive(Clone, Copy)]
pub struct B(pub f32);
impl Component for B {
    fn storage_type() -> StorageType { StorageType::SparseSet }
}

#[derive(Clone, Copy)]
pub struct SparsePos(pub Vec3);
impl Component for SparsePos {
    fn storage_type() -> StorageType { StorageType::SparseSet }
}

#[derive(Clone, Copy)]
pub struct SparseVel(pub Vec3);
impl Component for SparseVel {
    fn storage_type() -> StorageType { StorageType::SparseSet }
}

#[derive(Clone, Copy)]
pub struct Pos<const N: usize>(pub Vec3);
impl<const N: usize> Component for Pos<N> {}

#[derive(Clone, Copy)]
pub struct Vel<const N: usize>(pub Vec3);
impl<const N: usize> Component for Vel<N> {}

#[derive(Clone, Copy)]
pub struct SparsePosWide<const N: usize>(pub Vec3);
impl<const N: usize> Component for SparsePosWide<N> {
    fn storage_type() -> StorageType { StorageType::SparseSet }
}

#[derive(Clone, Copy)]
pub struct SparseVelWide<const N: usize>(pub Vec3);
impl<const N: usize> Component for SparseVelWide<N> {
    fn storage_type() -> StorageType { StorageType::SparseSet }
}

#[derive(Clone, Copy)]
pub struct TestA(pub f32);
impl Component for TestA {}

#[derive(Clone, Copy)]
pub struct TestB(pub f32);
impl Component for TestB {}

#[derive(Clone, Copy)]
pub struct TestC(pub f32);
impl Component for TestC {}

#[derive(Clone, Copy)]
pub struct TestD(pub f32);
impl Component for TestD {}

#[derive(Clone, Copy)]
pub struct TestE(pub f32);
impl Component for TestE {}

#[derive(Clone, Copy, Default)]
pub struct Table(pub f32);
impl Component for Table {
    fn storage_type() -> StorageType { StorageType::Table }
}

#[derive(Clone, Copy, Default)]
pub struct Sparse(pub f32);
impl Component for Sparse {
    fn storage_type() -> StorageType { StorageType::SparseSet }
}

#[derive(Clone, Copy, Default)]
pub struct WideTable<const X: usize>(pub f32);
impl<const X: usize> Component for WideTable<X> {
    fn storage_type() -> StorageType { StorageType::Table }
}

#[derive(Clone, Copy, Default)]
pub struct WideSparse<const X: usize>(pub f32);
impl<const X: usize> Component for WideSparse<X> {
    fn storage_type() -> StorageType { StorageType::SparseSet }
}

#[derive(Clone, Copy, Default)]
pub struct ArchetypeData<const X: u16>(pub f32);
impl<const X: u16> Component for ArchetypeData<X> {
    fn storage_type() -> gizmo_core::component::StorageType { gizmo_core::component::StorageType::Table }
}

pub const RANGE: core::ops::Range<u32> = 5..6;

pub fn setup<T: Component + Default + Clone>(entity_count: u32) -> (World, Vec<Entity>) {
    let mut world = World::new();
    let entities: Vec<Entity> = world
        .spawn_batch(std::iter::repeat_n((T::default(),), entity_count as usize))
        .collect();
    core::hint::black_box((world, entities))
}

pub fn setup_wide<T: Bundle + Default + Clone>(
    entity_count: u32,
) -> (World, Vec<Entity>) {
    let mut world = World::new();
    let entities: Vec<Entity> = world
        .spawn_batch(std::iter::repeat_n(T::default(), entity_count as usize))
        .collect();
    core::hint::black_box((world, entities))
}
