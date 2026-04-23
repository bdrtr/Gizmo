// Math constants
const PI: f32 = 3.14159265359;

// Poly6 Kernel for Density (3D Formulation)
fn W_poly6(r_sq: f32, h: f32) -> f32 {
    let h_sq = h * h;
    if (r_sq >= 0.0 && r_sq <= h_sq) {
        let diff = h_sq - r_sq;
        // 315 / (64 * pi * h^9) = 1.56668147106 / h^9
        let coeff = 1.56668147106 / (h * h_sq * h_sq * h_sq * h_sq);
        return coeff * diff * diff * diff;
    }
    return 0.0;
}

// Spiky Kernel Gradient for Pressure (3D Formulation)
fn gradW_spiky(r: vec3<f32>, r_len: f32, h: f32) -> vec3<f32> {
    if (r_len > 0.0 && r_len <= h) {
        let diff = h - r_len;
        // -45 / (pi * h^6) = -14.3239448783 / h^6
        let coeff = -14.3239448783 / (h * h * h * h * h * h);
        return (r / r_len) * coeff * diff * diff;
    }
    return vec3<f32>(0.0, 0.0, 0.0);
}

// Viscosity Kernel Laplacian (3D Formulation)
fn laplacianW_viscosity(r_len: f32, h: f32) -> f32 {
    if (r_len > 0.0 && r_len <= h) {
        // 45 / (pi * h^6) = 14.3239448783 / h^6
        let coeff = 14.3239448783 / (h * h * h * h * h * h);
        return coeff * (h - r_len);
    }
    return 0.0;
}
