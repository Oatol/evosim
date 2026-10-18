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

// ─────────────────────── 环境场色表（M4）───────────────────────
//
// 一张 256 项的 RGBA 色表，着色器直接按场值查表。这样换配色/加新场
// 都不用动着色器，只换这张 1 KB 的纹理。

/// 色带：给若干 (位置 0-1, 颜色) 停靠点，线性插值出 256 项
fn ramp(stops: &[(f32, [u8; 3])]) -> Vec<[u8; 4]> {
    let mut out = Vec::with_capacity(256);
    for i in 0..256 {
        let t = i as f32 / 255.0;
        // 找到 t 落在哪一段
        let mut a = stops[0];
        let mut b = stops[stops.len() - 1];
        for w in stops.windows(2) {
            if t >= w[0].0 && t <= w[1].0 {
                a = w[0];
                b = w[1];
                break;
            }
        }
        let span = (b.0 - a.0).max(1e-6);
        let k = ((t - a.0) / span).clamp(0.0, 1.0);
        let c = [
            (a.1[0] as f32 + (b.1[0] as f32 - a.1[0] as f32) * k) as u8,
            (a.1[1] as f32 + (b.1[1] as f32 - a.1[1] as f32) * k) as u8,
            (a.1[2] as f32 + (b.1[2] as f32 - a.1[2] as f32) * k) as u8,
        ];
        out.push([c[0], c[1], c[2], 255]);
    }
    out
}

/// 温度色表：冷 → 紫 → 红 → 橙 → 黄 → 白（和 TPT 的热力图一个路子）
pub fn temp_colormap() -> Vec<[u8; 4]> {
    ramp(&[
        (0.00, [6, 10, 20]),
        (0.12, [26, 20, 62]),
        (0.30, [110, 30, 78]),
        (0.50, [200, 58, 30]),
        (0.70, [255, 140, 20]),
        (0.86, [255, 214, 90]),
        (1.00, [255, 255, 240]),
    ])
}

/// 光照色表：全黑 → 深蓝 → 靛 → 暖橙 → 亮黄 → 近白
///
/// 用"冷到暖"而不是"黑到白"：暗处是蓝黑（像夜色），亮处是暖黄（像阳光），
/// 中间经过一段靛紫——这样一眼就能看出"这块地方晒不晒得到太阳"，
/// 也能看出墙投下的阴影边界。
pub fn light_colormap() -> Vec<[u8; 4]> {
    ramp(&[
        (0.00, [4, 6, 16]),
        (0.15, [14, 18, 52]),
        (0.32, [48, 40, 110]),
        (0.50, [126, 78, 120]),
        (0.68, [214, 126, 62]),
        (0.85, [250, 196, 84]),
        (1.00, [255, 250, 224]),
    ])
}

/// 辐射色表：暗 → 深绿 → 亮绿 → 黄绿 → 近白
pub fn rad_colormap() -> Vec<[u8; 4]> {
    ramp(&[
        (0.00, [6, 10, 20]),
        (0.18, [8, 58, 40]),
        (0.42, [24, 150, 80]),
        (0.66, [120, 226, 96]),
        (0.86, [214, 255, 130]),
        (1.00, [244, 255, 210]),
    ])
}

#[cfg(test)]
mod colormap_tests {
    use super::*;

    #[test]
    fn colormaps_are_monotonic_in_brightness() {
        for (name, lut) in [("temp", temp_colormap()), ("rad", rad_colormap())] {
            assert_eq!(lut.len(), 256);
            let lum = |c: [u8; 4]| c[0] as u32 + c[1] as u32 + c[2] as u32;
            assert!(
                lum(lut[0]) < lum(lut[128]),
                "{} 色表应该从暗到亮（0: {}, 128: {}）",
                name,
                lum(lut[0]),
                lum(lut[128])
            );
            assert!(lum(lut[128]) < lum(lut[255]), "{} 色表末端应更亮", name);
            assert!(lut.iter().all(|c| c[3] == 255), "{} 色表必须不透明", name);
        }
    }

    #[test]
    fn temp_and_rad_differ_at_mid() {
        let t = temp_colormap()[128];
        let r = rad_colormap()[128];
        assert_ne!(t, r, "温度与辐射的中段颜色必须不同，否则分不清滤镜");
    }
}
