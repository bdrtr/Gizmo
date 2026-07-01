// Benchmark payload structs legitimately carry fields that are only written
// (never read) to model realistic component sizes; suppress dead_code crate-wide.
#![allow(dead_code)]

use criterion::{criterion_group, criterion_main};

mod common;
mod storage_churn;
mod spawn;
mod iteration_frag;
mod iteration_wide;
mod iteration_table;
mod iteration_sparse;
mod iteration_par;
mod observer;
mod system_param;
mod run_condition;
mod schedule_scaling;
mod commands;
mod world_lifecycle;
mod world_query;
mod change_detection;
mod empty_archetypes;
mod clone;
mod iter_frag_empty;
mod resources;

use storage_churn::*;
use spawn::*;
use iteration_frag::*;
use iteration_wide::*;
use iteration_table::*;
use iteration_sparse::*;
use iteration_par::*;
use observer::*;
use system_param::*;
use run_condition::*;
use schedule_scaling::*;
use commands::*;
use world_lifecycle::*;
use world_query::*;
use change_detection::*;
use empty_archetypes::*;
use clone::*;
use iter_frag_empty::*;
use resources::*;

pub use common::Mat4;

criterion_group!(
    benches,
    bench_insert_remove_sparseset,
    bench_insert_remove_batch,
    bench_heavyweight_bundle,
    bench_spawn_batch,
    bench_heavy_compute,
    bench_fragmented_iteration,
    bench_wide_iteration,
    bench_fragmented_wide_iteration,
    bench_simple_iter,
    bench_contiguous_iter,
    bench_contiguous_iter_avx2,
    bench_for_each_iter,
    bench_cache_locality_loss,
    bench_sparse_iter,
    bench_wide_simple_iter,
    bench_wide_sparse_iter,
    bench_bypass_change_detection,
    bench_sparse_simple_iter,
    bench_system_iter,
    bench_wide_sparse_simple_iter,
    bench_par_cache_locality_loss,
    bench_observer_lifecycle_insert,
    bench_event_propagation,
    bench_combinator_system,
    dyn_param,
    run_condition_yes,
    run_condition_no,
    run_condition_yes_with_query,
    run_condition_yes_with_resource,
    empty_systems,
    busy_systems,
    contrived,
    schedule_bench,
    build_schedule,
    empty_schedule_run,
    empty_commands,
    spawn_commands,
    nonempty_spawn_commands,
    insert_commands,
    fake_commands,
    zero_sized_commands,
    medium_sized_commands,
    large_sized_commands,
    world_despawn,
    world_despawn_recursive,
    entity_allocator_benches,
    entity_set_build_and_lookup,
    world_spawn,
    world_spawn_batch,
    world_entity,
    world_get,
    world_query_get,
    world_query_iter,
    world_query_for_each,
    query_get,
    query_get_components_mut_2,
    query_get_components_mut_5,

    query_get_components_mut_10,
    all_added_detection,
    all_changed_detection,
    few_changed_detection,
    none_changed_detection,

    multiple_archetype_none_changed_detection,

    empty_archetypes,
    single_clone,
    hierarchy_tall,
    hierarchy_wide,

    hierarchy_many,

    iter_frag_empty,
    get_resource,
    get_resource_mut,
    insert_remove_resource
);
criterion_main!(benches);
