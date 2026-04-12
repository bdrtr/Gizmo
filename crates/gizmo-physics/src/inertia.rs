//! Yaklaşık eylemsizlik: nokta bulutu üzerinden PCA + OBB ile homojen katı kutu tensörü.
//! Tam konveks polihedron hacim integrali yüzey ağı gerektirir; bu modül oyunlarda sık kullanılan
//! merkezlenmiş nokta bulutuna sığan katı kutu yaklaşımını kullanır.

use gizmo_math::{Mat3, Vec3};

const MIN_HALF_EXTENT: f32 = 1e-4;
const EIGEN_ITERS: usize = 32;

/// Simetrik dış çarpım `s * v vᵀ` (sütunlar `v * (s * v_k)`).
#[inline]
fn scaled_sym_outer(v: Vec3, s: f32) -> Mat3 {
    Mat3::from_cols(
        v * (s * v.x),
        v * (s * v.y),
        v * (s * v.z),
    )
}

#[inline]
fn symmetrize(m: Mat3) -> Mat3 {
    (m + m.transpose()) * 0.5
}

/// `m` simetrik varsayımıyla birim baskın özvektör (güç yinelemesi).
fn dominant_unit_eigenvector(m: Mat3) -> Vec3 {
    let mut v = Vec3::X;
    if m.determinant().abs() < 1e-30 {
        let d = Vec3::new(m.x_axis.x, m.y_axis.y, m.z_axis.z);
        if d.x.abs() >= d.y.abs() && d.x.abs() >= d.z.abs() {
            v = Vec3::X;
        } else if d.y.abs() >= d.z.abs() {
            v = Vec3::Y;
        } else {
            v = Vec3::Z;
        }
    }
    for _ in 0..EIGEN_ITERS {
        let nv = (m * v).normalize();
        if (nv - v).length_squared() < 1e-16 {
            break;
        }
        v = nv;
    }
    v.normalize()
}

/// Simetrik 3×3 için üç birim özvektör (sütunlar `R`).
fn eigen_basis_symmetric(c: Mat3) -> Mat3 {
    let c = symmetrize(c);
    let v1 = dominant_unit_eigenvector(c);
    let l1 = v1.dot(c * v1).max(0.0);
    let c1 = c - scaled_sym_outer(v1, l1);

    let mut v2 = dominant_unit_eigenvector(c1);
    v2 = (v2 - v1 * v1.dot(v2)).normalize();
    if v2.length_squared() < 1e-12 {
        v2 = if v1.dot(Vec3::Y).abs() < 0.9 {
            v1.cross(Vec3::Y).normalize()
        } else {
            v1.cross(Vec3::Z).normalize()
        };
    }

    let mut v3 = v1.cross(v2);
    if v3.length_squared() < 1e-12 {
        v3 = if v1.dot(Vec3::X).abs() < 0.9 {
            v1.cross(Vec3::X).normalize()
        } else {
            v1.cross(Vec3::Z).normalize()
        };
    } else {
        v3 = v3.normalize();
    }

    Mat3::from_cols(v1, v2, v3)
}

/// Merkezlenmiş noktalar için eksen `axis` (birim) boyunca yarı uzantı.
fn half_extent_along(centered: &[Vec3], axis: Vec3) -> f32 {
    let mut mn = f32::INFINITY;
    let mut mx = f32::NEG_INFINITY;
    for p in centered {
        let t = p.dot(axis);
        mn = mn.min(t);
        mx = mx.max(t);
    }
    ((mx - mn) * 0.5).max(MIN_HALF_EXTENT)
}

/// Orijinde merkezlenmiş katı dikdörtgen prizma: tam boyut `sx,sy,sz`.
fn solid_box_inertia_at_origin(mass: f32, sx: f32, sy: f32, sz: f32) -> Mat3 {
    let sx2 = sx * sx;
    let sy2 = sy * sy;
    let sz2 = sz * sz;
    let ix = (1.0 / 12.0) * mass * (sy2 + sz2);
    let iy = (1.0 / 12.0) * mass * (sx2 + sz2);
    let iz = (1.0 / 12.0) * mass * (sx2 + sy2);
    Mat3::from_diagonal(Vec3::new(ix, iy, iz))
}

/// Köşe noktaları orijin etrafında simetrik AABB içeren kutu (max |x|,|y|,|z|).
fn fallback_symmetric_box(centered: &[Vec3], mass: f32) -> (Vec3, Mat3) {
    let mut hx = MIN_HALF_EXTENT;
    let mut hy = MIN_HALF_EXTENT;
    let mut hz = MIN_HALF_EXTENT;
    for p in centered {
        hx = hx.max(p.x.abs());
        hy = hy.max(p.y.abs());
        hz = hz.max(p.z.abs());
    }
    let w = (hx * 2.0).max(MIN_HALF_EXTENT * 2.0);
    let h = (hy * 2.0).max(MIN_HALF_EXTENT * 2.0);
    let d = (hz * 2.0).max(MIN_HALF_EXTENT * 2.0);
    let i = solid_box_inertia_at_origin(mass, w, h, d);
    let principal = Vec3::new(
        (1.0 / 12.0) * mass * (h * h + d * d),
        (1.0 / 12.0) * mass * (w * w + d * d),
        (1.0 / 12.0) * mass * (w * w + h * h),
    );
    (principal, i.inverse())
}

/// Homojen yoğunluklu katı OBB yaklaşımı: `vertices` collider lokal çerçevesinde.
/// Dönüş: (OBB ana eksenlerinde katı kutu I diyagonalı, `inverse_inertia_local`).
pub fn inverse_inertia_from_point_cloud(mass: f32, vertices: &[Vec3]) -> (Vec3, Mat3) {
    if mass <= 0.0 || vertices.is_empty() {
        return (Vec3::ZERO, Mat3::ZERO);
    }
    if vertices.len() == 1 {
        return (Vec3::ZERO, Mat3::ZERO);
    }

    let n = vertices.len() as f32;
    let com = vertices.iter().copied().sum::<Vec3>() / n;
    let centered: Vec<Vec3> = vertices.iter().map(|v| *v - com).collect();

    let mut cov = Mat3::ZERO;
    for p in &centered {
        cov += scaled_sym_outer(*p, 1.0);
    }
    cov = symmetrize(cov);

    let r = eigen_basis_symmetric(cov);
    let hx = half_extent_along(&centered, r.x_axis);
    let hy = half_extent_along(&centered, r.y_axis);
    let hz = half_extent_along(&centered, r.z_axis);
    let sx = (hx * 2.0).max(MIN_HALF_EXTENT * 2.0);
    let sy = (hy * 2.0).max(MIN_HALF_EXTENT * 2.0);
    let sz = (hz * 2.0).max(MIN_HALF_EXTENT * 2.0);

    let i_principal = solid_box_inertia_at_origin(mass, sx, sy, sz);
    let i_body = r * i_principal * r.transpose();
    let det = i_body.determinant();
    let inv = if det.is_finite() && det.abs() > 1e-20 {
        i_body.inverse()
    } else {
        return fallback_symmetric_box(&centered, mass);
    };

    if !(inv.x_axis.x.is_finite() && inv.y_axis.y.is_finite() && inv.z_axis.z.is_finite()) {
        return fallback_symmetric_box(&centered, mass);
    }

    let principal_diag = Vec3::new(
        (1.0 / 12.0) * mass * (sy * sy + sz * sz),
        (1.0 / 12.0) * mass * (sx * sx + sz * sz),
        (1.0 / 12.0) * mass * (sx * sx + sy * sy),
    );
    (principal_diag, inv)
}

/// Yükseklik alanı köşe örnekleri (üst yüzey + taban) — katı blok yaklaşımı için nokta bulutu.
pub fn heightfield_sample_vertices(
    heights: &[f32],
    segments_x: u32,
    segments_z: u32,
    width: f32,
    depth: f32,
    max_height: f32,
) -> Vec<Vec3> {
    let sx = segments_x.max(1);
    let sz = segments_z.max(1);
    let sx_f = sx.saturating_sub(1).max(1) as f32;
    let sz_f = sz.saturating_sub(1).max(1) as f32;
    let half_w = width * 0.5;
    let half_d = depth * 0.5;
    let mut out = Vec::with_capacity((sx * sz * 2) as usize);
    for gz in 0..sz {
        let fz = gz as f32 / sz_f;
        let lz = -half_d + depth * fz;
        for gx in 0..sx {
            let fx = gx as f32 / sx_f;
            let lx = -half_w + width * fx;
            let idx = (gz * sx + gx) as usize;
            let h = if idx < heights.len() {
                heights[idx] * max_height
            } else {
                0.0
            };
            out.push(Vec3::new(lx, h, lz));
            out.push(Vec3::new(lx, 0.0, lz));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn point_cloud_inertia_finite() {
        let verts = [
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(-1.0, 0.0, 0.0),
            Vec3::new(0.0, 2.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
        ];
        let (principal, inv) = inverse_inertia_from_point_cloud(10.0, &verts);
        assert!(principal.x > 0.0 && principal.y > 0.0 && principal.z > 0.0);
        let det = inv.determinant();
        assert!(det.is_finite() && det.abs() > 1e-10);
    }
}
