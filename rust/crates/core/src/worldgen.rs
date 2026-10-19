//! 世界生成器。与 JS 版 `src/core/WorldGen.js` 对应。
//!
//! 洞穴状墙体（成簇生长）+ 成片食物草甸 + 零散食物。
//! 所有随机走 world.rng，同一 seed 生成同一世界。

use crate::cell_type::ct;
use crate::world::World;

fn paint_blob(world: &mut World, cx: f64, cy: f64, radius: f64, ty: u8, jitter: f64) {
    let r2 = radius * radius;
    let c0 = (cx - radius - 1.0).floor() as i32;
    let c1 = (cx + radius + 1.0).ceil() as i32;
    let r0 = (cy - radius - 1.0).floor() as i32;
    let r1 = (cy + radius + 1.0).ceil() as i32;

    for row in r0..=r1 {
        for col in c0..=c1 {
            let dx = col as f64 - cx;
            let dy = row as f64 - cy;
            let d2 = dx * dx + dy * dy;
            if d2 > r2 {
                continue;
            }
            if jitter > 0.0 && d2 > r2 * (1.0 - jitter) {
                continue;
            }
            if let Some(index) = world.grid.index(col, row) {
                world.set_cell_at(index, ty, -1);
            }
        }
    }
}

/// **富贫斑块**：每格的「肥沃度」0~1，和天空光的透射率共用同一张噪声图。
///
/// 为什么共用：`Light` 的掩码是 `(cols, rows, patchiness, floor, seed)` 的纯函数，
/// 这里用同一个种子 + 同样的 `floor` 算一遍，得到的图**逐格相同**。
/// 于是「光岛」和「富斑块」在空间上是同一批地方——这在生态上也对：
/// 阳光充足的地方本来就是食物链的起点。
///
/// 返回 `None` 表示关闭（`patch_richness = 0`），调用方走完全没改动的旧路径。
fn fertility(world: &World) -> Option<Vec<f32>> {
    let r = world.config.world.patch_richness.clamp(0.0, 1.0);
    if r <= 0.0 {
        return None;
    }
    let floor = (1.0 - r) as f32;
    let n = crate::light::cloud_noise(world.grid.cols, world.grid.rows, world.rng.seed());
    Some(n.iter().map(|&v| floor + (1.0 - floor) * v).collect())
}

/// 取某格肥沃度（关闭时恒 1.0，等于旧行为）。
#[inline]
fn fert_at(f: Option<&Vec<f32>>, cols: usize, col: i32, row: i32) -> f64 {
    match f {
        None => 1.0,
        Some(v) => {
            let c = col.rem_euclid(cols as i32) as usize;
            let r = row.max(0) as usize;
            v.get(r * cols + c).copied().unwrap_or(1.0) as f64
        }
    }
}

pub fn generate_demo_world(world: &mut World, wall_density: f64, food_density: f64) {
    let cols = world.grid.cols;
    let rows = world.grid.rows;
    let area = (cols * rows) as f64;

    world.reset(false);

    // **富贫斑块**（`patch_richness > 0` 才启用，默认关闭）。
    // 一张低频噪声图，同时决定「光有多足」和「食物有多密」——
    // 光那一半由 `Light` 自己算（同一张图），这里只负责食物。
    let fert = fertility(world);

    // ── 1. 洞穴状墙体：成簇生长 ──
    let wall_seeds = (((area / 2600.0) * wall_density).round() as i32).max(6);
    for _ in 0..wall_seeds {
        let mut cx = world.rng.range(0.0, cols as f64);
        let mut cy = world.rng.range(0.0, rows as f64);
        let steps = world.rng.int_range(10, 34);
        let mut radius = world.rng.range(1.6, 3.4);
        for _ in 0..steps {
            paint_blob(world, cx, cy, radius, ct::WALL, 0.45);
            cx += world.rng.gaussian(0.0, 2.1);
            cy += world.rng.gaussian(0.0, 2.1);
            radius = (radius + world.rng.gaussian(0.0, 0.25)).max(1.1);
        }
    }

    // ── 2. 食物草甸 ──
    let meadow_count = (((area / 4200.0) * food_density).round() as i32).max(4);
    for _ in 0..meadow_count {
        // 落点：关闭时一次采样（旧行为）；开启时取 3 个候选里**最肥**的。
        // 这就是"富斑块食物密、贫斑块几乎没有"的第一半——草甸主动往富的地方长。
        let (cx, cy) = if let Some(f) = fert.as_ref() {
            let mut best = (0.0, 0.0, -1.0f64);
            for _ in 0..3 {
                let c = world.rng.range(0.0, cols as f64);
                let r = world.rng.range(0.0, rows as f64);
                let q = fert_at(Some(f), cols, c as i32, r as i32);
                if q > best.2 {
                    best = (c, r, q);
                }
            }
            (best.0, best.1)
        } else {
            (
                world.rng.range(0.0, cols as f64),
                world.rng.range(0.0, rows as f64),
            )
        };
        let q = fert_at(fert.as_ref(), cols, cx as i32, cy as i32);
        // 半径和填充率都随肥沃度走：贫斑块里就算落了草甸，也是又小又稀。
        let radius = world.rng.range(6.0, 15.0) * (0.55 + 0.55 * q);
        let fill = (0.34 * (0.15 + 1.1 * q)).clamp(0.0, 0.95);
        let r2 = radius * radius;
        let row_from = (cy - radius).floor() as i32;
        let row_to = (cy + radius).ceil() as i32;
        let col_from = (cx - radius).floor() as i32;
        let col_to = (cx + radius).ceil() as i32;
        for row in row_from..=row_to {
            for col in col_from..=col_to {
                let dx = col as f64 - cx;
                let dy = row as f64 - cy;
                if dx * dx + dy * dy > r2 {
                    continue;
                }
                let index = match world.grid.index(col, row) {
                    Some(i) => i,
                    None => continue,
                };
                if world.grid.types[index] != ct::EMPTY {
                    continue;
                }
                if world.rng.float() < fill {
                    world.set_cell_at(index, ct::FOOD, -1);
                }
            }
        }
    }

    // ── 3. 零散食物 ──
    let scatter = ((area / 220.0) * food_density).round() as i32;
    for _ in 0..scatter {
        let col = world.rng.int(cols) as i32;
        let row = world.rng.int(rows) as i32;
        // 零散食物也按肥沃度拒绝：贫斑块里几乎不长。
        if let Some(f) = fert.as_ref() {
            let q = fert_at(Some(f), cols, col, row);
            if world.rng.float() > q {
                continue;
            }
        }
        if let Some(index) = world.grid.index(col, row) {
            if world.grid.types[index] == ct::EMPTY {
                world.set_cell_at(index, ct::FOOD, -1);
            }
        }
    }
}

pub fn generate_empty_world(world: &mut World) {
    world.reset(false);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::rng::Rng;

    const COLS: usize = 140;
    const ROWS: usize = 90;

    fn build(richness: f64, seed: u32) -> World {
        let mut cfg = Config::default();
        cfg.world.cols = COLS;
        cfg.world.rows = ROWS;
        cfg.world.patch_richness = richness;
        let mut w = World::new(cfg, Rng::new(seed));
        generate_demo_world(&mut w, 1.0, 1.0);
        w
    }

    /// 按一张「权重图」把世界分成肥/瘦各四分之一，返回 `瘦区食物密度 / 肥区食物密度` 的倒数
    /// （即肥区 ÷ 瘦区，> 1 表示食物偏向肥区）。
    fn food_skew(w: &World, key: &[f32]) -> f64 {
        let mut idx: Vec<usize> = (0..w.grid.len).collect();
        idx.sort_by(|a, b| key[*a].partial_cmp(&key[*b]).unwrap());
        let q = idx.len() / 4;
        let density = |range: &[usize]| {
            let food = range
                .iter()
                .filter(|&&i| w.grid.types[i] == ct::FOOD)
                .count();
            food as f64 / range.len() as f64
        };
        let thin = density(&idx[..q]);
        let fat = density(&idx[idx.len() - q..]);
        if thin <= 1e-9 {
            f64::INFINITY
        } else {
            fat / thin
        }
    }

    /// 关闭富贫斑块时，食物**不该**和那张噪声图相关——旧行为是均匀撒的。
    ///
    /// 这里用 `cloud_noise` 的原始场当权重：如果代码不小心把富贫斑块
    /// 也应用到了关闭状态，食物就会聚到噪声高的地方，比值会明显 > 1。
    #[test]
    fn richness_off_keeps_food_even() {
        let w = build(0.0, 1001);
        let key = crate::light::cloud_noise(COLS, ROWS, w.rng.seed());
        let skew = food_skew(&w, &key);
        assert!(
            (0.6..1.7).contains(&skew),
            "关闭时食物不该偏向噪声高处，实测 肥/瘦 = {:.2}",
            skew
        );
    }

    /// 开启富贫斑块后，**肥斑块里的食物明显更密**——这才是"富斑块"。
    #[test]
    fn richness_piles_food_into_the_rich_patches() {
        let w = build(0.8, 1001);
        let f = fertility(&w).expect("开启时应该有肥沃度图");
        let skew = food_skew(&w, &f);
        assert!(
            skew > 2.5,
            "富斑块的食物密度应该明显高于贫斑块，实测 肥/瘦 = {:.2}",
            skew
        );
    }

    /// 同一组 (种子, 参数) 必须生成同一个世界。
    #[test]
    fn worldgen_is_deterministic_with_richness() {
        let a = build(0.7, 4242);
        let b = build(0.7, 4242);
        assert_eq!(a.grid.types, b.grid.types, "同种子必须逐格相同");
    }

    /// 富贫斑块的图和光场用的是**同一张噪声**，所以「光岛」和「食物岛」重合。
    ///
    /// 这条钉的是两个模块之间的隐性契约：`worldgen::fertility` 和
    /// `Light` 的掩码都由 `cloud_noise(cols, rows, rng.seed())` 推出，
    /// 一旦哪边换了种子或换了 `floor` 的算法，两边就会错位——
    /// 那会变成"光足的地方没吃的、黑的地方全是草甸"，说不通。
    ///
    /// **只在第 0 行比**：天空光沿深度衰减（每行 −3），拿深处的值和
    /// 掩码直接比会对不上，那是另一回事。
    #[test]
    fn fertility_matches_the_light_mask() {
        let w = build(0.7, 9);
        let f = fertility(&w).expect("开启时应该有图");
        for col in 0..COLS {
            let want = (f[col] * 255.0).round().clamp(0.0, 255.0) as i32;
            let got = w.light.sky_at(&w.grid, col as i32, 0) as i32;
            assert!(
                (want - got).abs() <= 2,
                "第 {} 列：肥沃度 {} 和顶行天空光 {} 对不上——两张图错位了",
                col,
                want,
                got
            );
        }
    }
}
