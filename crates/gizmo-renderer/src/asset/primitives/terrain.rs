use crate::components::Mesh;
use crate::renderer::Vertex;
use gizmo_math::Vec3;

impl crate::asset::AssetManager {
    /// Builds a terrain mesh from a heightmap image — see
    /// [`Terrain`](crate::components::Terrain), which holds the recipe this reads.
    pub fn create_terrain(
        device: &wgpu::Device,
        heightmap_path: &str,
        width: f32,
        depth: f32,
        max_height: f32,
    ) -> Result<(Mesh, Vec<f32>, u32, u32), crate::asset::error::AssetError> {
        let canonical = std::path::Path::new(heightmap_path)
            .canonicalize()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| heightmap_path.to_string());

        let img = image::open(&canonical)
            .map_err(|source| crate::asset::error::AssetError::ImageDecode {
                path: std::path::PathBuf::from(&canonical),
                source,
            })?
            .into_luma8(); // Grayscale format

        let (img_width, img_height) = img.dimensions();
        if img_width < 2 || img_height < 2 {
            return Err(crate::asset::error::AssetError::HeightmapTooSmall {
                path: std::path::PathBuf::from(&canonical),
                width: img_width,
                height: img_height,
            });
        }
        let heights: Vec<f32> = img.pixels().map(|p| p[0] as f32 / 255.0).collect();
        let final_vertices = Self::terrain_triangles(&heights, img_width, img_height, width, depth, max_height);

        let mesh = Mesh::new_indexed(
            device,
            &final_vertices,
            Vec3::ZERO,
            format!("terrain:{}", heightmap_path),
        );
        Ok((mesh, heights, img_width, img_height))
    }

    /// The terrain surface as a flat-shaded triangle list, from `heights` (row-major, `0..=1`,
    /// `cols` along X and `rows` along Z) stretched over `width × depth`, centred on the origin,
    /// `0..=max_height` tall — what [`create_terrain`](Self::create_terrain) uploads.
    ///
    /// Each cell is split along the **same diagonal as `Collider::heightfield`'s cells**,
    /// `(row, col)`–`(row+1, col+1)`, so the surface drawn and the surface collided with are the
    /// same two triangles. They were the opposite diagonals until 2026-11-04: on any cell that is
    /// not flat a body rested above or inside the ground it was drawn on.
    pub fn terrain_triangles(heights: &[f32], cols: u32, rows: u32, width: f32, depth: f32, max_height: f32) -> Vec<Vertex> {
        if cols < 2 || rows < 2 || heights.len() != (cols as usize) * (rows as usize) {
            return Vec::new();
        }
        let at = |col: u32, row: u32| {
            let h = heights[(row * cols + col) as usize];
            Vec3::new(
                -width / 2.0 + (col as f32 / (cols as f32 - 1.0)) * width,
                h * max_height,
                -depth / 2.0 + (row as f32 / (rows as f32 - 1.0)) * depth,
            )
        };
        // UV Mapping: Repeat 10 times across terrain so grass doesn't look stretched
        let uv = |col: u32, row: u32| [(col as f32 / (cols as f32 - 1.0)) * 10.0, (row as f32 / (rows as f32 - 1.0)) * 10.0];
        let mut out = Vec::with_capacity(((cols - 1) * (rows - 1) * 6) as usize);
        for row in 0..rows - 1 {
            for col in 0..cols - 1 {
                // Counter-clockwise seen from +Y, the collider's own order.
                let c00 = (col, row);
                let c01 = (col + 1, row);
                let c10 = (col, row + 1);
                let c11 = (col + 1, row + 1);
                for tri in [[c00, c10, c11], [c00, c11, c01]] {
                    let p = tri.map(|(c, r)| at(c, r));
                    let n = (p[1] - p[0]).cross(p[2] - p[0]);
                    let normal = if n.length_squared() > 1e-12 { n.normalize() } else { Vec3::Y };
                    for (i, &(c, r)) in tri.iter().enumerate() {
                        out.push(Vertex {
                            position: p[i].to_array(),
                            normal: normal.to_array(),
                            tex_coords: uv(c, r),
                            ..Default::default()
                        });
                    }
                }
            }
        }
        out
    }
}
