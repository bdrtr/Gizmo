//! The CPU records and the shader structs must agree, byte for byte.
//!
//! `GpuCompute` uploads `#[repr(C)]` Rust structs straight into storage buffers with
//! `bytemuck::cast_slice`, and sizes those buffers as `capacity * size_of::<T>()`. The shader
//! then indexes them with its OWN idea of the element stride. If the two disagree, every
//! element past index 0 is read from the wrong offset — silently, with no validation error
//! and no panic, producing garbage forces rather than a crash.
//!
//! `GpuCompute` is behind the default-off `gpu_physics` feature, and running it needs a GPU
//! adapter that CI does not have (`tests/gpu_step.rs` runs it where there is one, and would show a
//! stride mismatch only as wrong forces).
//!
//! So this test does the one thing that works without a GPU: it parses `soft_body.wgsl` with
//! naga — the same front end wgpu uses — and asks naga's layouter for each struct's size and
//! alignment under the WGSL memory-layout rules. That is an executable check of a property
//! that was previously only assertable by hand.
//!
//! It found a real one. `Tetrahedron` ended in `pad3: vec3<f32>`; `vec3<f32>` has an
//! alignment of 16, so after `rest_volume` at offset 64 the shader placed `pad3` at 80, not
//! 68, and rounded the struct up to 96 bytes — against the 80 the Rust side uploads.

use naga::proc::Layouter;

const SHADER: &str = include_str!("../src/soft_body.wgsl");

/// Size and alignment naga computes for the named struct in the shader.
fn wgsl_struct_layout(name: &str) -> (u32, u32) {
    let module = naga::front::wgsl::parse_str(SHADER)
        .unwrap_or_else(|e| panic!("soft_body.wgsl does not parse: {e:?}"));

    let mut layouter = Layouter::default();
    layouter
        .update(module.to_ctx())
        .expect("naga could not lay out soft_body.wgsl");

    let (handle, _) = module
        .types
        .iter()
        .find(|(_, ty)| ty.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("soft_body.wgsl declares no struct named `{name}`"));

    let layout = layouter[handle];
    (layout.size, layout.alignment.round_up(layout.size))
}

/// Every struct the CPU uploads must have the same size on both sides.
///
/// The expected numbers are the Rust `size_of` values, restated here as literals on purpose:
/// this file is compiled without the `gpu_physics` feature (so the Rust structs do not exist
/// to compare against directly), and a literal makes a change on either side visible in the
/// diff rather than silently agreeing with itself.
#[test]
fn shader_struct_sizes_match_the_uploaded_rust_records() {
    // The nodes are `array<vec4<f32>>` on both sides (`[f32; 4]` in Rust), so they have no struct.
    // `GpuTetrahedron`: [u32;4] + 3 x ([f32;3] + pad) + rest_volume + mu + lambda + pad = 80.
    //
    // The trap: a trailing `vec3<f32>` would align to 16 and push the shader's struct to 96
    // while Rust stayed at 80. Keep the tail of this struct built from `f32`s.
    let (size, _) = wgsl_struct_layout("Tetrahedron");
    assert_eq!(
        size, 80,
        "WGSL `Tetrahedron` is {size} bytes; the Rust `GpuTetrahedron` uploaded into that \
         buffer is 80. A mismatch is silent — the shader would read every element past index \
         0 from the wrong offset. Check for a `vec3<f32>` near the end of the struct: it \
         aligns to 16 and Rust's `[f32; 3]` does not."
    );

    // `GpuParameters`: num_elements + 3 pad = 16 bytes.
    let (size, _) = wgsl_struct_layout("Parameters");
    assert_eq!(
        size, 16,
        "WGSL `Parameters` is {size} bytes; the Rust `GpuParameters` uploaded into that buffer is 16"
    );
}

/// A struct whose alignment exceeds its size rounds UP when arrayed, and it is the ARRAY
/// stride the shader indexes with — not the bare size. Pin that too, or a struct could match
/// on size and still stride differently.
#[test]
fn shader_struct_array_strides_match_their_sizes() {
    for name in ["Tetrahedron", "Parameters"] {
        let (size, stride) = wgsl_struct_layout(name);
        assert_eq!(
            size, stride,
            "WGSL `{name}` has size {size} but an array stride of {stride}; the Rust upload \
             packs elements at `size_of::<T>()` with no gap, so the shader would drift by \
             {} bytes per element",
            stride - size
        );
    }
}
