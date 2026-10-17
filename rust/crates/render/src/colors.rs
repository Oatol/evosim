//! 配色 —— 与 JS 版 `src/render/ColorScheme.js` 对齐。

use evosim_core::cell_type::{def, CELL_TYPE_COUNT};
use evosim_core::world::World;

/// FNV-1a（与 JS 版 `fnv1a` 一致）
pub fn fnv1a(s: &str) -> u32 {
    let mut h: u32 = 2_166_136_261;
    for b in s.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(16_777_619);
    }
    h
}

/// 指纹 → 色相 0–359
pub fn fingerprint_hue(fingerprint: &str) -> u32 {
    if fingerprint.is_empty() {
        0
    } else {
        fnv1a(fingerprint) % 360
    }
}

fn hsl_channel(p: f64, q: f64, mut t: f64) -> f64 {
    if t < 0.0 {
        t += 1.0;
    }
    if t > 1.0 {
        t -= 1.0;
    }
    if t < 1.0 / 6.0 {
        return p + (q - p) * 6.0 * t;
    }
    if t < 0.5 {
        return q;
    }
    if t < 2.0 / 3.0 {
        return p + (q - p) * (2.0 / 3.0 - t) * 6.0;
    }
    p
}

/// 指纹 → RGB（与 JS 的 `speciesRGB` 逐位一致：hsl → rgb，饱和 62%、亮度 58%）
pub fn species_rgb(fingerprint: &str) -> [u8; 3] {
    let h = fingerprint_hue(fingerprint) as f64 / 360.0;
    let s = 0.62;
    let l = 0.58;
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    [
        (hsl_channel(p, q, h + 1.0 / 3.0) * 255.0).round() as u8,
        (hsl_channel(p, q, h) * 255.0).round() as u8,
        (hsl_channel(p, q, h - 1.0 / 3.0) * 255.0).round() as u8,
    ]
}

/// 类型着色调色板（classic 方案），256 项 RGBA8，按类型 id 索引
pub fn build_palette() -> Vec<[u8; 4]> {
    let mut pal = vec![[0u8, 0, 0, 255]; 256];
    for t in 0..CELL_TYPE_COUNT {
        if let Some(d) = def(t as u8) {
            let rgb = d.colors[0];
            pal[t] = [
                ((rgb >> 16) & 0xFF) as u8,
                ((rgb >> 8) & 0xFF) as u8,
                (rgb & 0xFF) as u8,
                255,
            ];
        }
    }
    pal
}

/// 生成"按物种着色"的逐格 RGBA 图（30k 格约 0.1 ms，切换模式时才重建）
pub fn build_species_pixels(world: &World) -> Vec<[u8; 4]> {
    use std::collections::HashMap;
    let grid = &world.grid;
    let mut out = vec![[0u8, 0, 0, 255]; grid.len];
    let mut cache: HashMap<u32, [u8; 3]> = HashMap::new();

    // 先按 owner id → 颜色 建表（每个个体查一次，而不是每格一次）
    let mut owner_color: HashMap<i32, [u8; 3]> = HashMap::with_capacity(world.organisms.len());
    for org in &world.organisms {
        let fp = org.species_fingerprint.as_deref().unwrap_or("");
        let rgb = *cache
            .entry(fnv1a(fp))
            .or_insert_with(|| species_rgb(fp));
        owner_color.insert(org.id as i32, rgb);
    }

    let base = build_palette();
    for i in 0..grid.len {
        let owner = grid.owners[i];
        if owner >= 0 {
            if let Some(rgb) = owner_color.get(&owner) {
                out[i] = [rgb[0], rgb[1], rgb[2], 255];
                continue;
            }
        }
        let t = grid.types[i] as usize;
        out[i] = base[t.min(255)];
    }
    out
}
