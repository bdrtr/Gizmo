// ═══════════════════════════════════════════════════════════════════════
//  AAA SPH Kernel Functions — Gizmo Engine Fluid Physics
//  Navier-Stokes tabanlı sıkıştırılamaz akış çekirdekleri
// ═══════════════════════════════════════════════════════════════════════

const PI: f32 = 3.14159265359;

// ─── Poly6 Kernel (Density Estimation) ───
// Müller et al. 2003, 3D formulation
// W(r,h) = 315 / (64 * π * h^9) * (h² - r²)³
fn W_poly6(r_sq: f32, h: f32) -> f32 {
    let h_sq = h * h;
    if (r_sq >= 0.0 && r_sq <= h_sq) {
        let diff = h_sq - r_sq;
        // 315 / (64 * π * h^9)
        let coeff = 1.56668147106 / (h * h_sq * h_sq * h_sq * h_sq);
        return coeff * diff * diff * diff;
    }
    return 0.0;
}

// ─── Spiky Kernel Gradient (Pressure) ───
// Desbrun & Gascuel 1996, ensures repulsion at short range
// ∇W(r,h) = -45 / (π * h^6) * (h - |r|)² * r̂
fn gradW_spiky(r: vec3<f32>, r_len: f32, h: f32) -> vec3<f32> {
    if (r_len > 0.0 && r_len <= h) {
        let diff = h - r_len;
        // -45 / (π * h^6)
        let coeff = -14.3239448783 / (h * h * h * h * h * h);
        return (r / r_len) * coeff * diff * diff;
    }
    return vec3<f32>(0.0, 0.0, 0.0);
}

// ─── Viscosity Kernel Laplacian ───
// Müller et al. 2003, positive-definite Laplacian for stable viscosity
// ∇²W(r,h) = 45 / (π * h^6) * (h - |r|)
fn laplacianW_viscosity(r_len: f32, h: f32) -> f32 {
    if (r_len > 0.0 && r_len <= h) {
        let coeff = 14.3239448783 / (h * h * h * h * h * h);
        return coeff * (h - r_len);
    }
    return 0.0;
}

// ─── Cubic Spline Kernel (Higher accuracy, used for Vorticity) ───
// Monaghan & Lattanzio 1985
// Better behaved near center compared to Poly6
fn W_cubic(r_len: f32, h: f32) -> f32 {
    let q = r_len / h;
    // σ = 8 / (π * h³) in 3D
    let sigma = 2.546479089470 / (h * h * h);
    if (q <= 0.5) {
        return sigma * (6.0 * q * q * q - 6.0 * q * q + 1.0);
    } else if (q <= 1.0) {
        let t = 1.0 - q;
        return sigma * 2.0 * t * t * t;
    }
    return 0.0;
}

// ─── Cohesion Kernel (Akinci et al. 2013 — Surface Tension) ───
// Two-part kernel for modeling molecular cohesion forces
// This kernel has a negative region that creates surface tension
fn W_cohesion(r_len: f32, h: f32) -> f32 {
    // 32 / (π * h^9)
    let coeff = 10.185916357881 / (h * h * h * h * h * h * h * h * h);
    if (r_len <= h && r_len > 0.0) {
        let half_h = h * 0.5;
        if (r_len <= half_h) {
            // Inner region: attractive + repulsive
            let t1 = h - r_len;
            let t2 = t1 * t1 * t1 * r_len * r_len * r_len;
            let t3 = h * h * h * h * h * h / 64.0;
            return coeff * (2.0 * t2 - t3);
        } else {
            // Outer region: purely attractive
            let t1 = h - r_len;
            return coeff * t1 * t1 * t1 * r_len * r_len * r_len;
        }
    }
    return 0.0;
}
