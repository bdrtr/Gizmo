/// WASM Optimizasyon Profil Sistemi
/// 
/// Oyun türüne göre otomatik GPU kaynak yönetimi sağlar.
/// Tarayıcı ortamında maksimum performans için gereksiz
/// subsistemler otomatik devre dışı bırakılır.
///
/// # Kullanım
/// ```rust
/// let profile = WebProfile::fighter(); // Dövüş oyunu preset'i
/// let profile = WebProfile::racing();  // Yarış oyunu preset'i
/// let profile = WebProfile::sandbox(); // Açık dünya / sandbox
/// let profile = WebProfile::custom()   // Özel konfigürasyon
///     .with_particles(true, 5000)
///     .with_shadows(true)
///     .with_post_processing(PostProcessLevel::Medium);
/// ```

/// Post-processing kalite seviyesi
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PostProcessLevel {
    /// Sadece tone mapping, gamma correction
    Minimal,
    /// Bloom + tone mapping (DoF yok)
    Low,
    /// Bloom + chromatic aberration + vignette (DoF yok)
    Medium,
    /// Full pipeline (bloom + DoF + film grain + vignette + CA)
    High,
}

/// Shadow kalite seviyesi
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShadowQuality {
    /// Gölge yok
    Off,
    /// Tek cascade, düşük çözünürlük
    Low,
    /// 2 cascade, orta çözünürlük
    Medium,
    /// 4 cascade, yüksek çözünürlük (masaüstü varsayılan)
    High,
}

/// WASM ortamında GPU kaynak konfigürasyonu
#[derive(Clone, Debug)]
pub struct WebProfile {
    /// Profil adı (log ve debug için)
    pub name: &'static str,
    
    // ── GPU Compute Subsystems ────────────────────────────
    /// GPU particle sistemi aktif mi?
    pub gpu_particles_enabled: bool,
    /// Maksimum GPU particle sayısı
    pub gpu_particles_max: u32,
    
    /// GPU fizik simülasyonu aktif mi?
    pub gpu_physics_enabled: bool,
    /// Maksimum GPU fizik nesnesi
    pub gpu_physics_max: u32,
    
    /// GPU sıvı simülasyonu aktif mi?
    pub gpu_fluid_enabled: bool,
    /// Maksimum GPU sıvı parçacığı
    pub gpu_fluid_max: u32,
    
    // ── Rendering Pipeline ───────────────────────────────
    /// Deferred rendering aktif mi? (false = forward only)
    pub deferred_enabled: bool,
    
    /// GPU frustum culling aktif mi?
    pub gpu_cull_enabled: bool,
    
    /// Gölge kalitesi
    pub shadow_quality: ShadowQuality,
    
    /// Post-processing seviyesi
    pub post_process_level: PostProcessLevel,
    
    // ── Screen Space Effects ─────────────────────────────
    /// SSAO (Ambient Occlusion) aktif mi?
    pub ssao_enabled: bool,
    
    /// SSR (Screen Space Reflections) aktif mi?
    pub ssr_enabled: bool,
    
    /// SSGI (Screen Space Global Illumination) aktif mi?
    pub ssgi_enabled: bool,
    
    /// TAA (Temporal Anti-Aliasing) aktif mi?
    pub taa_enabled: bool,
    
    /// Volumetric lighting (God Rays) aktif mi?
    pub volumetric_enabled: bool,
    
    // ── Resource Limits ──────────────────────────────────
    /// Maksimum bind group sayısı (Chrome WebGPU = 4)
    pub max_bind_groups: u32,
    
    /// Maksimum instance sayısı (instanced rendering)
    pub max_instances: usize,
    
    /// HDR texture formatı (Rgba16Float vs Rgba8Unorm)
    pub use_hdr: bool,
}

impl WebProfile {
    // ════════════════════════════════════════════════════════
    //  PRESET'LER — Oyun türüne göre hazır profiller
    // ════════════════════════════════════════════════════════
    
    /// 🥊 Dövüş oyunu — 2 karakter, arena, hit efektleri
    /// Particles: düşük (hit spark), Fluid: kapalı, Deferred: kapalı
    pub fn fighter() -> Self {
        Self {
            name: "Fighter",
            gpu_particles_enabled: true,
            gpu_particles_max: 2_000,     // Hit spark'lar için yeterli
            gpu_physics_enabled: false,
            gpu_physics_max: 0,
            gpu_fluid_enabled: false,
            gpu_fluid_max: 0,
            deferred_enabled: false,
            gpu_cull_enabled: false,
            shadow_quality: ShadowQuality::Off,
            post_process_level: PostProcessLevel::Medium,
            ssao_enabled: false,
            ssr_enabled: false,
            ssgi_enabled: false,
            taa_enabled: false,
            volumetric_enabled: false,
            max_bind_groups: 4,
            max_instances: 256,
            use_hdr: true,
        }
    }
    
    /// 🏎️ Yarış oyunu — çok nesne, hız efektleri, geniş sahne
    /// Particles: orta (toz/kıvılcım), Fluid: kapalı, Shadows: düşük
    pub fn racing() -> Self {
        Self {
            name: "Racing",
            gpu_particles_enabled: true,
            gpu_particles_max: 10_000,    // Toz, kıvılcım, exhaust
            gpu_physics_enabled: false,
            gpu_physics_max: 0,
            gpu_fluid_enabled: false,
            gpu_fluid_max: 0,
            deferred_enabled: false,
            gpu_cull_enabled: false,       // CPU culling yeterli
            shadow_quality: ShadowQuality::Low,
            post_process_level: PostProcessLevel::Medium,
            ssao_enabled: false,
            ssr_enabled: false,
            ssgi_enabled: false,
            taa_enabled: false,
            volumetric_enabled: false,
            max_bind_groups: 4,
            max_instances: 512,
            use_hdr: true,
        }
    }
    
    /// 🌊 Su/sıvı odaklı oyun — SPH fluid, fizik etkileşimi
    /// Particles: orta, Fluid: aktif (düşük limit), Physics: düşük
    pub fn fluid() -> Self {
        Self {
            name: "Fluid",
            gpu_particles_enabled: true,
            gpu_particles_max: 5_000,
            gpu_physics_enabled: true,
            gpu_physics_max: 1_000,
            gpu_fluid_enabled: true,
            gpu_fluid_max: 10_000,        // Mobil/web için 10K yeterli
            deferred_enabled: false,
            gpu_cull_enabled: false,
            shadow_quality: ShadowQuality::Off,
            post_process_level: PostProcessLevel::Low,
            ssao_enabled: false,
            ssr_enabled: false,
            ssgi_enabled: false,
            taa_enabled: false,
            volumetric_enabled: false,
            max_bind_groups: 4,
            max_instances: 128,
            use_hdr: true,
        }
    }
    
    /// 🏗️ Sandbox / açık dünya — dengeli, her şeyden biraz
    pub fn sandbox() -> Self {
        Self {
            name: "Sandbox",
            gpu_particles_enabled: true,
            gpu_particles_max: 5_000,
            gpu_physics_enabled: true,
            gpu_physics_max: 5_000,
            gpu_fluid_enabled: false,
            gpu_fluid_max: 0,
            deferred_enabled: false,
            gpu_cull_enabled: false,
            shadow_quality: ShadowQuality::Low,
            post_process_level: PostProcessLevel::Low,
            ssao_enabled: false,
            ssr_enabled: false,
            ssgi_enabled: false,
            taa_enabled: false,
            volumetric_enabled: false,
            max_bind_groups: 4,
            max_instances: 1024,
            use_hdr: true,
        }
    }
    
    /// 🖥️ Masaüstü — tüm özellikler açık (varsayılan native profil)
    pub fn desktop() -> Self {
        Self {
            name: "Desktop",
            gpu_particles_enabled: true,
            gpu_particles_max: 100_000,
            gpu_physics_enabled: true,
            gpu_physics_max: 50_000,
            gpu_fluid_enabled: true,
            gpu_fluid_max: 100_000,
            deferred_enabled: true,
            gpu_cull_enabled: true,
            shadow_quality: ShadowQuality::High,
            post_process_level: PostProcessLevel::High,
            ssao_enabled: true,
            ssr_enabled: true,
            ssgi_enabled: true,
            taa_enabled: true,
            volumetric_enabled: true,
            max_bind_groups: 8,
            max_instances: 16384,
            use_hdr: true,
        }
    }
    
    /// 📱 Minimum — en düşük ayarlar (eski donanım / mobil)
    pub fn minimal() -> Self {
        Self {
            name: "Minimal",
            gpu_particles_enabled: false,
            gpu_particles_max: 0,
            gpu_physics_enabled: false,
            gpu_physics_max: 0,
            gpu_fluid_enabled: false,
            gpu_fluid_max: 0,
            deferred_enabled: false,
            gpu_cull_enabled: false,
            shadow_quality: ShadowQuality::Off,
            post_process_level: PostProcessLevel::Minimal,
            ssao_enabled: false,
            ssr_enabled: false,
            ssgi_enabled: false,
            taa_enabled: false,
            volumetric_enabled: false,
            max_bind_groups: 4,
            max_instances: 64,
            use_hdr: false,
        }
    }
    
    // ════════════════════════════════════════════════════════
    //  BUILDER API — Özel profil oluşturma
    // ════════════════════════════════════════════════════════
    
    /// Boş profil (her şey kapalı) — builder ile özelleştir
    pub fn custom() -> Self {
        Self::minimal()
    }
    
    pub fn with_particles(mut self, enabled: bool, max: u32) -> Self {
        self.gpu_particles_enabled = enabled;
        self.gpu_particles_max = max;
        self
    }
    
    pub fn with_physics(mut self, enabled: bool, max: u32) -> Self {
        self.gpu_physics_enabled = enabled;
        self.gpu_physics_max = max;
        self
    }
    
    pub fn with_fluid(mut self, enabled: bool, max: u32) -> Self {
        self.gpu_fluid_enabled = enabled;
        self.gpu_fluid_max = max;
        self
    }
    
    pub fn with_shadows(mut self, quality: ShadowQuality) -> Self {
        self.shadow_quality = quality;
        self
    }
    
    pub fn with_post_processing(mut self, level: PostProcessLevel) -> Self {
        self.post_process_level = level;
        self
    }
    
    pub fn with_deferred(mut self, enabled: bool) -> Self {
        self.deferred_enabled = enabled;
        self
    }
    
    pub fn with_ssao(mut self, enabled: bool) -> Self {
        self.ssao_enabled = enabled;
        self
    }

    pub fn with_ssr(mut self, enabled: bool) -> Self {
        self.ssr_enabled = enabled;
        self
    }
    
    pub fn with_max_instances(mut self, max: usize) -> Self {
        self.max_instances = max;
        self
    }
    
    // ════════════════════════════════════════════════════════
    //  UTILITY
    // ════════════════════════════════════════════════════════
    
    /// Mevcut platforma göre varsayılan profil seç
    pub fn auto() -> Self {
        #[cfg(target_arch = "wasm32")]
        { Self::fighter() }
        #[cfg(not(target_arch = "wasm32"))]
        { Self::desktop() }
    }
    
    /// Profil özetini logla
    pub fn log_summary(&self) {
        log::info!("╔══════════════════════════════════════════╗");
        log::info!("║  WebProfile: {:26} ║", self.name);
        log::info!("╠══════════════════════════════════════════╣");
        log::info!("║  Particles:  {:>6}  ({})       ║", 
            if self.gpu_particles_enabled { format!("{}", self.gpu_particles_max) } else { "OFF".to_string() },
            if self.gpu_particles_enabled { "✓" } else { "✗" });
        log::info!("║  Physics:    {:>6}  ({})       ║",
            if self.gpu_physics_enabled { format!("{}", self.gpu_physics_max) } else { "OFF".to_string() },
            if self.gpu_physics_enabled { "✓" } else { "✗" });
        log::info!("║  Fluid:      {:>6}  ({})       ║",
            if self.gpu_fluid_enabled { format!("{}", self.gpu_fluid_max) } else { "OFF".to_string() },
            if self.gpu_fluid_enabled { "✓" } else { "✗" });
        log::info!("║  Shadows:    {:?}{:>16} ║", self.shadow_quality, "");
        log::info!("║  PostFX:     {:?}{:>16} ║", self.post_process_level, "");
        log::info!("║  Deferred:   {:<24}  ║", if self.deferred_enabled { "✓" } else { "✗" });
        log::info!("║  SSAO:       {:<24}  ║", if self.ssao_enabled { "✓" } else { "✗" });
        log::info!("║  Instances:  {:<24}  ║", self.max_instances);
        log::info!("╚══════════════════════════════════════════╝");
    }
}

impl Default for WebProfile {
    fn default() -> Self {
        Self::auto()
    }
}
