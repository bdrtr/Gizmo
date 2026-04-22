use gizmo::prelude::*;
fn main() {
    println!("FluidParams size: {}", std::mem::size_of::<gizmo::renderer::gpu_fluid_system::FluidParams>());
    println!("mouse_pos offset: {}", std::mem::offset_of!(gizmo::renderer::gpu_fluid_system::FluidParams, mouse_pos));
}
