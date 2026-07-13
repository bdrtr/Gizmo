//! glTF mesh building — vertex assembly, per-vertex tangent fallback, flat-normal generation
//! and skin-weight normalization, plus the recursive `parse_gltf_node`. Extracted verbatim
//! from `loaders.rs` (pure move). `parse_gltf_node` is driven by `load_gltf_from_import`.

use super::*;

/// Compute per-triangle flat normals and assign them to each vertex in the
/// triangle.  Vertices must already be in expanded (non-indexed) form and the
/// primitive mode must be `Triangles` (guaranteed by the caller).
/// glTF joint weights must form a partition of unity (sum = 1.0): the skinning
/// shader computes `Σ wᵢ·Mᵢ` WITHOUT renormalizing (shader.wgsl). Exporters and
/// quantized `KHR_mesh_quantization` weights frequently emit sums slightly ≠ 1
/// (e.g. 0.998), which scales the skin matrix and distorts the mesh. Normalize
/// whenever there is any weight; leave an all-zero weight (a non-skinned vertex)
/// untouched so the shader's `sum > 0` guard correctly keeps it unskinned.
fn normalize_skin_weights(w: [f32; 4]) -> [f32; 4] {
    let sum = w[0] + w[1] + w[2] + w[3];
    if sum > 1e-5 {
        [w[0] / sum, w[1] / sum, w[2] / sum, w[3] / sum]
    } else {
        w
    }
}

fn compute_flat_normals(vertices: &mut [Vertex]) {
    for tri in vertices.chunks_exact_mut(3) {
        let v0 = Vec3::from(tri[0].position);
        let v1 = Vec3::from(tri[1].position);
        let v2 = Vec3::from(tri[2].position);

        let edge1 = v1 - v0;
        let edge2 = v2 - v0;
        let cross = edge1.cross(edge2);

        let normal = if cross.length_squared() > 1e-10 {
            cross.normalize()
        } else {
            Vec3::Y // degenerate triangle → point up
        };

        let n = [normal.x, normal.y, normal.z];
        tri[0].normal = n;
        tri[1].normal = n;
        tri[2].normal = n;
    }
}

impl crate::asset::AssetManager {
    pub(super) fn parse_gltf_node(
        &mut self,
        device: &wgpu::Device,
        node: &gltf::Node,
        buffers: &[gltf::buffer::Data],
        materials: &[Material],
        file_name: &str,
    ) -> GltfNodeData {
        let (translation, rotation, scale) = node.transform().decomposed();

        let mut primitives = Vec::new();

        if let Some(mesh) = node.mesh() {
            for (prim_i, primitive) in mesh.primitives().enumerate() {
                // Only handle triangles — skip lines, points, strips, etc.
                if primitive.mode() != gltf::mesh::Mode::Triangles {
                    tracing::error!(
                        "[GLTF WARN] Skipping non-triangle primitive (mode={:?}) on node '{}'",
                        primitive.mode(),
                        node.name().unwrap_or("<unnamed>"),
                    );
                    continue;
                }

                let reader = primitive.reader(|buf| Some(&buffers[buf.index()]));

                let positions: Vec<[f32; 3]> = reader
                    .read_positions()
                    .map(|it| it.collect())
                    .unwrap_or_default();

                if positions.is_empty() {
                    continue; // nothing to upload
                }

                let supplied_normals: Option<Vec<[f32; 3]>> =
                    reader.read_normals().map(|it| it.collect());

                let supplied_tangents: Option<Vec<[f32; 4]>> =
                    reader.read_tangents().map(|it| it.collect());

                let tex_coords: Vec<[f32; 2]> = reader
                    .read_tex_coords(0)
                    .map(|it| it.into_f32().collect())
                    .unwrap_or_else(|| vec![[0.0, 0.0]; positions.len()]);

                let joints: Option<Vec<[u16; 4]>> =
                    reader.read_joints(0).map(|it| it.into_u16().collect());
                let weights: Option<Vec<[f32; 4]>> =
                    reader.read_weights(0).map(|it| it.into_f32().collect());

                // Expand indexed geometry into a flat vertex list.
                let mut all_vertices: Vec<Vertex> = Vec::new();
                let mut aabb = gizmo_math::Aabb::empty();

                let make_vertex = |idx: usize| -> Vertex {
                    let pos = positions[idx];

                    // Safe access — attribute arrays may be shorter than positions.
                    let normal = supplied_normals
                        .as_ref()
                        .and_then(|n| n.get(idx).copied())
                        .unwrap_or([0.0, 1.0, 0.0]);
                    let uv = tex_coords.get(idx).copied().unwrap_or([0.0, 0.0]);
                    let j = joints
                        .as_ref()
                        .and_then(|js| js.get(idx))
                        .map(|&[a, b, c, d]| [a as u32, b as u32, c as u32, d as u32])
                        .unwrap_or([0; 4]);
                    let w = normalize_skin_weights(
                        weights
                            .as_ref()
                            .and_then(|ws| ws.get(idx))
                            .copied()
                            .unwrap_or([0.0; 4]),
                    );

                    let tangent = if let Some(ref tangents) = supplied_tangents {
                        tangents.get(idx).copied().unwrap_or([1.0, 0.0, 0.0, 1.0])
                    } else {
                        // Calculate a dynamic tangent orthogonal to normal
                        let n = gizmo_math::Vec3::from(normal);
                        let t = if n.x.abs() > 0.9 {
                            gizmo_math::Vec3::new(0.0, 1.0, 0.0).cross(n).normalize()
                        } else {
                            gizmo_math::Vec3::new(1.0, 0.0, 0.0).cross(n).normalize()
                        };
                        [t.x, t.y, t.z, 1.0]
                    };

                    Vertex {
                        position: pos,
                        normal,
                        tex_coords: uv,
                        color: [1.0, 1.0, 1.0],
                        joint_indices: j,
                        joint_weights: w,
                        tangent,
                    }
                };

                if let Some(indices) = reader.read_indices() {
                    // Triangle-list assembly: process indices in groups of 3 and
                    // drop the WHOLE triangle if any index is out of bounds.
                    // (Skipping a single OOB index would shift every later vertex
                    // and corrupt the grouping of all following triangles.)
                    let idx: Vec<u32> = indices.into_u32().collect();
                    for tri in idx.chunks_exact(3) {
                        if tri.iter().any(|&t| (t as usize) >= positions.len()) {
                            continue;
                        }
                        for &t in tri {
                            let i = t as usize;
                            let pos = positions[i];
                            aabb.extend(Vec3::new(pos[0], pos[1], pos[2]));
                            all_vertices.push(make_vertex(i));
                        }
                    }
                } else {
                    for (i, pos) in positions.iter().enumerate() {
                        aabb.extend(Vec3::new(pos[0], pos[1], pos[2]));
                        all_vertices.push(make_vertex(i));
                    }
                }

                // Compute flat normals when the file did not supply any.
                // We only do this for triangle lists (guaranteed above).
                if supplied_normals.is_none() {
                    compute_flat_normals(&mut all_vertices);
                }

                let vbuf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(&format!("GLTF VBuf: {file_name}_prim{prim_i}")),
                    contents: bytemuck::cast_slice(&all_vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });

                // Use a deterministic cache key that doesn't depend on Debug formatting.
                let mesh_source = format!(
                    "gltf_mesh_{file_name}_{}_p{prim_i}",
                    node.name().unwrap_or("<unnamed>")
                );
                let mesh_comp = Mesh::new(
                    device,
                    Arc::new(vbuf),
                    &all_vertices,
                    Vec3::ZERO,
                    mesh_source.clone(),
                );
                self.mesh_cache.insert(mesh_source, mesh_comp.clone());

                let mat_opt = primitive
                    .material()
                    .index()
                    .and_then(|idx| materials.get(idx).cloned());

                primitives.push((mesh_comp, mat_opt));
            }
        }

        let children = node
            .children()
            .map(|child| self.parse_gltf_node(device, &child, buffers, materials, file_name))
            .collect();

        GltfNodeData {
            index: node.index(),
            name: node.name().map(str::to_owned),
            skin_index: node.skin().map(|s| s.index()),
            translation,
            rotation,
            scale,
            primitives,
            children,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wsum(w: [f32; 4]) -> f32 {
        w[0] + w[1] + w[2] + w[3]
    }

    #[test]
    fn skin_weights_normalize_to_unity() {
        // Sum < 1 → ölçeklenip 1'e çıkar.
        let w = normalize_skin_weights([0.25, 0.25, 0.0, 0.0]);
        assert!((wsum(w) - 1.0).abs() < 1e-6, "sum={}", wsum(w));
        assert!((w[0] - 0.5).abs() < 1e-6 && (w[1] - 0.5).abs() < 1e-6);

        // Sum > 1 → küçültülüp 1'e iner, oranlar korunur.
        let w = normalize_skin_weights([0.3, 0.3, 0.3, 0.3]);
        assert!((wsum(w) - 1.0).abs() < 1e-6);
        for c in w {
            assert!((c - 0.25).abs() < 1e-6);
        }

        // Zaten 1 → değişmez.
        let w = normalize_skin_weights([0.5, 0.5, 0.0, 0.0]);
        assert!((wsum(w) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn skin_weights_all_zero_preserved() {
        // Skinless vertex: [0,0,0,0] DOKUNULMAZ ki shader'ın `sum > 0` guard'ı
        // onu doğru şekilde unskinned tutsun (yoksa skinless mesh bozulurdu).
        assert_eq!(normalize_skin_weights([0.0; 4]), [0.0; 4]);
    }

    #[test]
    fn skin_weights_arbitrary_sum_to_one() {
        for w in [[0.1, 0.2, 0.3, 0.05], [0.9, 0.05, 0.02, 0.0], [2.0, 1.0, 0.5, 0.5]] {
            let n = normalize_skin_weights(w);
            assert!((wsum(n) - 1.0).abs() < 1e-5, "input {w:?} → {n:?} sum {}", wsum(n));
        }
    }
}
