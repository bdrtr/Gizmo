//! Shader yükleme yardımcıları (WGSL modül oluşturma, WASM bind-group yeniden eşleme).

pub fn load_shader(
    device: &wgpu::Device,
    file_path: &str,
    fallback_src: &str,
    label: &str,
) -> wgpu::ShaderModule {
    let source = std::fs::read_to_string(file_path).unwrap_or_else(|_| fallback_src.to_string());
    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    })
}

/// WASM-only: Shader'daki bind group indekslerini yeniden eşle (shadow kaldırılıyor).
/// Native:  group(0)=global, group(1)=texture, group(2)=shadow, group(3)=skeleton, group(4)=instance
/// WASM:    group(0)=global, group(1)=texture, group(2)=skeleton, group(3)=instance  (shadow yok)
#[cfg(target_arch = "wasm32")]
pub fn load_shader_web(
    device: &wgpu::Device,
    fallback_src: &str,
    label: &str,
) -> wgpu::ShaderModule {
    let mut source = fallback_src.to_string();

    // 1) Shadow binding tanımlarını kaldır
    //    @group(2) @binding(0)\nvar t_shadow: ... ve @group(2) @binding(1)\nvar s_shadow: ...
    let mut cleaned_lines: Vec<String> = Vec::new();
    let lines: Vec<&str> = source.lines().collect();
    let mut skip_next = false;
    for (i, line) in lines.iter().enumerate() {
        if skip_next {
            skip_next = false;
            continue;
        }
        let trimmed = line.trim();
        // Shadow binding annotation + next line (var t_shadow / var s_shadow)
        if trimmed.starts_with("@group(2)") && trimmed.contains("@binding") {
            // Check if next line is shadow-related
            if i + 1 < lines.len() {
                let next = lines[i + 1].trim();
                if next.starts_with("var t_shadow") || next.starts_with("var s_shadow") {
                    skip_next = true;
                    continue;
                }
            }
        }
        cleaned_lines.push(line.to_string());
    }
    source = cleaned_lines.join("\n");

    // 2) textureSampleCompare bloğunu shadow_visibility = 1.0 ile değiştir
    // Shader'daki "var shadow_visibility = 1.0;" sonrası gelen if (scene.sun_direction.w > 0.5) bloğunu kaldıracağız.
    // Dövüş oyununda gölgeyi komple kapattığımız için bu blok WASM'da tamamen gereksiz yere GPU'yu yoruyor.
    if source.contains("textureSampleCompare") {
        let shadow_block_start = "    if (scene.sun_direction.w > 0.5) {";
        if let Some(start_pos) = source.find(shadow_block_start) {
            let after_start = &source[start_pos..];
            let mut depth = 0i32;
            let mut end_offset = 0;
            for (j, ch) in after_start.char_indices() {
                if ch == '{' {
                    depth += 1;
                }
                if ch == '}' {
                    depth -= 1;
                    if depth == 0 {
                        end_offset = j + 1;
                        break;
                    }
                }
            }
            if end_offset > 0 {
                // Bloğu tamamen sil (zaten yukarıda var shadow_visibility = 1.0 tanımlı)
                source.replace_range(start_pos..(start_pos + end_offset), "");
            }
        }
    }

    // 3) Bind group indekslerini yeniden eşle: 3→2, 4→3 (shadow kaldırıldı)
    source = source.replace("@group(4)", "@group(##INST##)");
    source = source.replace("@group(3)", "@group(2)");
    source = source.replace("@group(##INST##)", "@group(3)");

    device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some(label),
        source: wgpu::ShaderSource::Wgsl(source.into()),
    })
}
