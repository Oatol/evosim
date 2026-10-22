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
    // ── 4. 丰富地形（批次 44）──
    //
    // 顺序是有讲究的：河流先挖（要穿过草甸），结冰跟着河岸，
    // 然后是斑块类（沼泽 / 辐射 / 腐殖质），最后是岩浆——
    // 岩浆是"熔掉一切"的东西，放最后才不会把别的点缀掉。
    let rich = world.config.world.rich_terrain;
    if rich {
        paint_rich_terrain(world);
    }

    // ── 5. 洞穴（批次 25 / 44）──
    //
    // 放在最后：洞穴要覆盖掉墙体与草甸，才能保证"顶是完整的、底是腐殖质"。
    //
    // `cave_count = 0` 的含义按世界类型分叉：
    // - 丰富地形：**自动**（按面积算 3~8 条）——默认世界里就该有洞；
    // - 经典地形：一条都不挖（逐位等于批次 43 之前）。
    let caves = cave_tunnels(world);
    if caves > 0 {
        carve_caves(world, caves);
    }
}

/// 这个世界该挖几条洞穴（批次 44）。
///
/// `cave_count = 0` 的含义按世界类型分叉：丰富地形 = 按面积自动（3~8 条），
/// 经典地形 = 一条都不挖。抽成公开函数是因为**「重挖」按钮也要用同一个数**——
/// 否则默认世界（`cave_count = 0`）按重挖会一格都不挖，按钮等于假的。
pub fn cave_tunnels(world: &World) -> i32 {
    let n = world.config.world.cave_count;
    if n > 0 {
        return n;
    }
    if !world.config.world.rich_terrain {
        return 0;
    }
    ((world.grid.cols * world.grid.rows) / 20_000).clamp(3, 8) as i32
}

// ────────────────────────── 丰富地形（批次 44）──────────────────────────

/// 只在「空 / 草甸」上铺地形，**不动墙、不叠已有地形**。
///
/// 为什么不直接用 `paint_blob`：那个无条件覆盖，几块沼泽就能把山脉抹平，
/// 世界会越挖越平。地形应该"填在空处"，不是"重画一遍"。
fn paint_terrain_blob(world: &mut World, cx: f64, cy: f64, radius: f64, ty: u8, jitter: f64) {
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
            let Some(i) = world.grid.index(col, row) else {
                continue;
            };
            let cur = world.grid.types[i];
            if cur != ct::EMPTY && cur != ct::FOOD {
                continue;
            }
            world.set_cell_at(i, ty, -1);
        }
    }
}

/// 蜿蜒河道：从上到下贯穿，**每隔几行留一处浅滩**。
///
/// 浅滩是关键：一条完整不断的水带等于一堵墙（水要花能量才能过），
/// 有浅滩才叫"走廊"——**绕路远一点，还是蹚水贵一点**，这个取舍才是水存在的意义。
///
/// 河撞到墙就断（不覆盖墙），那正好是"峡谷尽头"和"此路不通"的来源。
fn carve_rivers(world: &mut World, count: i32) {
    let cols = world.grid.cols as i32;
    let rows = world.grid.rows as i32;
    if rows < 12 {
        return;
    }
    for _ in 0..count {
        let mut x = world.rng.range(0.0, cols as f64);
        let width = world.rng.int_range(1, 2) as i32;
        let mut fords: Vec<i32> = Vec::new();
        for _ in 0..world.rng.int_range(2, 4) {
            fords.push(world.rng.int_range(2, rows as i64 - 3) as i32);
        }
        for y in 0..rows {
            // 浅滩：这一行不放水，留一条能走的路
            if fords.iter().any(|&f| f == y) {
                continue;
            }
            x += world.rng.gaussian(0.0, 0.6);
            let cx = x.round() as i32;
            for d in 0..width {
                let Some(i) = world.grid.index(cx + d, y) else {
                    continue;
                };
                let cur = world.grid.types[i];
                if cur == ct::EMPTY || cur == ct::FOOD {
                    world.set_cell_at(i, ct::WATER, -1);
                }
            }
        }
    }
}

/// 河岸结冰：沿岸随机挑几格冻上。
///
/// 冰是唯一一种**改"落点"不改收支**的地形（批次 43），所以它在河岸上的效果是
/// "踩上去会滑进/滑出水面"——一个纯几何的意外，正好是移动策略最需要的那种变数。
fn freeze_banks(world: &mut World) {
    let len = world.grid.len;
    let cols = world.grid.cols as i32;
    for i in 0..len {
        if world.grid.types[i] != ct::WATER {
            continue;
        }
        let col = (i as i32) % cols;
        let row = (i as i32) / cols;
        // ① 水面结冰：整格从水变成冰 → 一段「能滑过去的浅滩」。
        //    这比"岸边结冰"有用得多：滑行本身就有位移，冻住的河段会把人送过去。
        if world.rng.bernoulli(0.12) {
            world.set_cell_at(i, ct::ICE, -1);
            continue;
        }
        // ② 岸边挂霜：偶尔把一格空地冻上，让冰出现在水边而不是只有水面上。
        if world.rng.bernoulli(0.10) {
            let (dc, dr) = [(1, 0), (-1, 0), (0, 1), (0, -1)][world.rng.int(4)];
            if let Some(j) = world.grid.index(col + dc, row + dr) {
                if world.grid.types[j] == ct::EMPTY {
                    world.set_cell_at(j, ct::ICE, -1);
                }
            }
        }
    }
}

/// 一整套地形（批次 44）。每个世界都长这样，但细节随种子不同。
fn paint_rich_terrain(world: &mut World) {
    let cols = world.grid.cols as i32;
    let rows = world.grid.rows as i32;
    // 太小的世界放不下河流 + 洞穴，直接不铺（那些尺寸只在单测里出现）
    if cols < 24 || rows < 24 {
        return;
    }
    let rf = |w: &mut World, lo: f64, hi: f64| w.rng.range(lo, hi);

    // ── 1. 河流 ──
    let rivers = world.rng.int_range(2, 3) as i32;
    carve_rivers(world, rivers);

    // ── 2. 河岸结冰 ──
    freeze_banks(world);

    // ── 3. 沼泽：低洼（下半部分），比水更贵 → 真正的禁区 ──
    for _ in 0..world.rng.int_range(3, 5) {
        let cx = rf(world, 0.0, cols as f64);
        let cy = rf(world, rows as f64 * 0.45, rows as f64 * 0.95);
        let r = rf(world, 3.0, 7.0);
        paint_terrain_blob(world, cx, cy, r, ct::SWAMP, 0.35);
    }

    // ── 4. 辐射带：风险/收益对称的斑块（变异率 ×3） ──
    for _ in 0..world.rng.int_range(3, 5) {
        let cx = rf(world, 0.0, cols as f64);
        let cy = rf(world, rows as f64 * 0.15, rows as f64 * 0.85);
        let r = rf(world, 3.0, 5.0);
        paint_terrain_blob(world, cx, cy, r, ct::RAD, 0.30);
    }

    // ── 5. 地表腐殖质：森林地表，暗处也有饭吃 ──
    //
    // 批次 10.5 的老结论："有暗区不够，暗区还得有饭吃"。
    // 地表这层腐殖质就是给"被墙遮住的格子"配的饭。
    for _ in 0..world.rng.int_range(3, 5) {
        let cx = rf(world, 0.0, cols as f64);
        let cy = rf(world, rows as f64 * 0.15, rows as f64 * 0.60);
        let r = rf(world, 2.0, 4.0);
        paint_terrain_blob(world, cx, cy, r, ct::HUMUS, 0.30);
    }

    // ── 6. 底部岩浆带 ──
    //
    // 世界的"地板"：一条烧红的地层，加上几个探到中段的岩浆池。
    // 它是这个世界唯一的**绝对禁区**（不像沼泽只是贵），
    // 所以位置放在最低处——生物得先有理由往下走，才会遇到它。
    let lava_top = (rows as f64 * 0.93) as i32;
    for row in lava_top..rows {
        for col in 0..cols {
            if world.rng.bernoulli(0.55) {
                if let Some(i) = world.grid.index(col, row) {
                    world.set_cell_at(i, ct::LAVA, -1);
                }
            }
        }
    }
    for _ in 0..world.rng.int_range(2, 3) {
        let cx = rf(world, 0.0, cols as f64);
        let cy = rf(world, rows as f64 * 0.72, rows as f64 * 0.93);
        let r = rf(world, 2.0, 4.0);
        paint_terrain_blob(world, cx, cy, r, ct::LAVA, 0.30);
    }
}

/// 挖几条带顶的隧道，洞底铺腐殖质（批次 25）。
///
/// 为什么是"顶 + 腐殖质"这个组合：云影已经证明「有暗区」不够——暗区还得有饭吃
/// 才可能住人（批次 10.5 的结论）。`WALL` 挡光、`HUMUS` 每 tick 产食，
/// 两样都是现成的，缺的只是把它们摆在一起。
///
/// 每条隧道起点挖一条竖井通到地表：不然生物进不去，洞穴就只是装饰。
/// 竖井会变成"光柱"——洞口那一小段是亮的，越往里越暗，正好是个梯度。
/// **在已经跑起来的世界里重挖洞穴**（批次 43）。
///
/// 和 `carve_caves` 的唯一区别：**跳过有生物占用的格子**。
/// 直接 `set_cell_at(WALL)` 会把站在那儿的身体格碾成墙——身体还在、格子已经不是它的了，
/// 那只生物会带着一个"虚空的器官"继续活着（`anatomy` 与 `grid` 不一致）。
/// 所以重挖只在空地上动土：挖出来的洞会比新建世界时少几个口子，但世界不会被挖坏。
///
/// 返回实际改动的格子数（0 = 世界太挤，一格都挖不动）。
pub fn recarve_caves(world: &mut World, count: i32) -> usize {
    let before = world.grid.types.clone();
    carve_caves_inner(world, count, true);
    world.refresh_light_now();
    before
        .iter()
        .zip(world.grid.types.iter())
        .filter(|(a, b)| a != b)
        .count()
}

fn carve_caves(world: &mut World, count: i32) {
    carve_caves_inner(world, count, false);
}

fn carve_caves_inner(world: &mut World, count: i32, skip_occupied: bool) {
    let cols = world.grid.cols as i32;
    let rows = world.grid.rows as i32;
    if cols < 16 || rows < 16 {
        return;
    }
    // 只挖下半部分：太靠上会和地表连成一片，失去"隔离"的意义
    let top = (rows as f64 * 0.50) as i32;
    let bottom = ((rows as f64 * 0.90) as i32).max(top + 4);
    for _ in 0..count {
        let mut cy = world.rng.int_range(top as i64, bottom as i64) as i32;
        let start_y = cy;
        let cx0 = world.rng.int_range(0, cols as i64 - 1) as i32;
        let len = world.rng.int_range((cols / 4).max(10) as i64, (cols / 2).max(12) as i64) as i32;
        let h = world.rng.int_range(2, 3) as i32;

        for dx in 0..len {
            // **蜿蜒**（批次 44）：主体一直往前推，偶尔上下漂一两格。
            // 直线隧道看着像地铁，弯的才像洞；漂移不改长度，所以挖出来的量级不变。
            if dx > 0 && world.rng.bernoulli(0.30) {
                cy = (cy + world.rng.int_range(-1, 1) as i32).clamp(top, bottom);
            }
            let x = (cx0 + dx).rem_euclid(cols);
            // 隧道本体挖空
            for dy in 0..h {
                if let Some(i) = world.grid.index(x, cy + dy) {
                    if skip_occupied && world.grid.owners[i] >= 0 {
                        continue;
                    }
                    world.set_cell_at(i, ct::EMPTY, -1);
                }
            }
            // 洞顶两层墙（挡光）——没有它，洞穴就只是一条沟
            for dy in 1..=2 {
                if let Some(i) = world.grid.index(x, cy - dy) {
                    if skip_occupied && world.grid.owners[i] >= 0 {
                        continue;
                    }
                    world.set_cell_at(i, ct::WALL, -1);
                }
            }
            // 洞底腐殖质（每 tick 产食）
            if let Some(i) = world.grid.index(x, cy + h) {
                if !(skip_occupied && world.grid.owners[i] >= 0) {
                    world.set_cell_at(i, ct::HUMUS, -1);
                }
            }
        }

        // 竖井：起点一个、中段一个。
        //
        // 两个入口才像"洞"：只有一个口的话，洞里没光、外面有光，
        // 生物进去等于走进死胡同（批次 25 时只有一个口，也是暗区人口偏少的原因之一）。
        let mid_x = (cx0 + len / 2).rem_euclid(cols);
        for (sx, sy) in [(cx0, start_y), (mid_x, cy)] {
            for y in 0..sy {
                if let Some(i) = world.grid.index(sx, y) {
                    if skip_occupied && world.grid.owners[i] >= 0 {
                        continue;
                    }
                    world.set_cell_at(i, ct::EMPTY, -1);
                }
            }
        }
    }
}

/// 空世界：只清场，什么都不画（画笔 / 工坊投放用）。
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

    /// 经典地形（`rich_terrain = false`）：这一组测试量的是**旧的生成管线**
    /// （成簇墙体 / 草甸 / 富贫斑块 / 洞穴），要把批次 44 的丰富地形关掉，
    /// 否则河流和岩浆会混进统计里。丰富地形由 `rich_*` 那组单独覆盖。
    fn build(richness: f64, seed: u32) -> World {
        let mut cfg = Config::default();
        cfg.world.cols = COLS;
        cfg.world.rows = ROWS;
        cfg.world.rich_terrain = false;
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

    // ── 批次 25：洞穴 ──

    /// `cave_count = 0`（默认）时世界里一个腐殖质都没有——默认世界逐位不变。
    #[test]
    fn zero_caves_is_a_no_op() {
        let w = build(0.0, 11);
        assert_eq!(w.config.world.cave_count, 0);
        let humus = w.grid.floor.iter().filter(|&&t| t == ct::HUMUS).count();
        assert_eq!(humus, 0, "默认世界不该有腐殖质，实测 {humus}");
    }

    /// 洞穴必须有**顶**（墙）和**底**（腐殖质），否则它就只是一条沟。
    #[test]
    fn caves_have_a_roof_and_a_humus_floor() {
        let mut cfg = Config::default();
        cfg.world.cols = COLS;
        cfg.world.rows = ROWS;
        cfg.world.rich_terrain = false; // 只量洞穴，别混进地表腐殖质
        cfg.world.cave_count = 3;
        let mut w = World::new(cfg, Rng::new(7));
        generate_demo_world(&mut w, 1.0, 1.0);

        let mut roofed = 0usize;
        for i in 0..w.grid.len {
            if w.grid.floor[i] != ct::HUMUS {
                continue;
            }
            let col = (i % COLS) as i32;
            let row = (i / COLS) as i32;
            // 洞底往上 3~5 行里必须有墙（隧道高度 2~3 + 顶 2 层）
            let has_roof = (-5..=-3).any(|dy| {
                w.grid
                    .index(col, row + dy)
                    .map(|j| w.grid.floor[j] == ct::WALL)
                    .unwrap_or(false)
            });
            if has_roof {
                roofed += 1;
            }
        }
        let total = w.grid.floor.iter().filter(|&&t| t == ct::HUMUS).count();
        assert!(total > 20, "三条隧道应该铺出不少腐殖质，实测 {total}");
        assert!(
            roofed * 100 / total.max(1) > 80,
            "腐殖质上方应该基本都有顶，实测 {roofed}/{total}"
        );
    }

    /// 洞穴在地下一半以下：不能挖到地表附近，否则"隔离"就不成立。
    #[test]
    fn caves_stay_underground() {
        let mut cfg = Config::default();
        cfg.world.cols = COLS;
        cfg.world.rows = ROWS;
        cfg.world.rich_terrain = false;
        cfg.world.cave_count = 4;
        let mut w = World::new(cfg, Rng::new(3));
        generate_demo_world(&mut w, 1.0, 1.0);
        let half = ROWS / 2;
        for i in 0..w.grid.len {
            if w.grid.floor[i] == ct::HUMUS {
                let row = i / COLS;
                assert!(
                    row >= half,
                    "第 {} 行出现了腐殖质，洞穴挖得太靠上了",
                    row
                );
            }
        }
    }

    // ── 批次 44：丰富地形 ──

    fn build_rich(seed: u32) -> World {
        let mut cfg = Config::default();
        cfg.world.cols = COLS;
        cfg.world.rows = ROWS;
        let mut w = World::new(cfg, Rng::new(seed));
        generate_demo_world(&mut w, 1.0, 1.0);
        w
    }

    fn count(w: &World, ty: u8) -> usize {
        w.grid.floor.iter().filter(|&&t| t == ty).count()
    }

    /// 默认世界必须**六种地形全都有**——这是这一批的全部意义。
    #[test]
    fn rich_world_has_every_terrain() {
        let w = build_rich(21);
        for (ty, name) in [
            (ct::WALL, "墙"),
            (ct::WATER, "水"),
            (ct::ICE, "冰"),
            (ct::SWAMP, "沼泽"),
            (ct::RAD, "辐射带"),
            (ct::HUMUS, "腐殖质"),
            (ct::LAVA, "岩浆"),
        ] {
            let n = count(&w, ty);
            assert!(n > 0, "默认世界里没有{}（{name}），实测 0 格", name);
        }
    }

    /// 关掉丰富地形时，除墙和食物之外一格地形都不该有（逐位等于批次 43 之前）。
    #[test]
    fn classic_world_has_only_walls_and_food() {
        let w = build(0.0, 22);
        for ty in [ct::WATER, ct::ICE, ct::SWAMP, ct::RAD, ct::HUMUS, ct::LAVA] {
            assert_eq!(
                count(&w, ty),
                0,
                "关掉丰富地形之后还冒出了地形 {}",
                crate::cell_type::label(ty)
            );
        }
        assert!(count(&w, ct::WALL) > 0, "经典世界也该有墙");
    }

    /// `cave_count = 0` 的含义按世界类型分叉——「重挖」按钮靠这条才不会变成假的。
    #[test]
    fn cave_tunnels_reads_zero_as_auto_only_when_rich() {
        let rich = build_rich(66);
        assert_eq!(rich.config.world.cave_count, 0);
        let auto = cave_tunnels(&rich);
        assert!(
            (3..=8).contains(&auto),
            "丰富地形下 0 应该自动算成 3~8 条，实测 {auto}"
        );

        let classic = build(0.0, 66);
        assert_eq!(
            cave_tunnels(&classic),
            0,
            "经典地形下 0 就该是一条都不挖"
        );

        let mut w = build_rich(66);
        w.config.world.cave_count = 2;
        assert_eq!(cave_tunnels(&w), 2, "手填的正数优先");
    }

    /// 同一 seed 必须生成同一个世界——地形是随机的，但**不能不可复现**。
    #[test]
    fn rich_world_is_deterministic() {
        let a = build_rich(33);
        let b = build_rich(33);
        assert_eq!(a.grid.floor, b.grid.floor, "同 seed 的地形必须逐格相同");
    }

    /// 岩浆只在最低处：它是绝对禁区，不该出现在生物日常活动的中上段。
    #[test]
    fn lava_stays_at_the_bottom() {
        let w = build_rich(44);
        let rows = w.grid.rows;
        let mut top_lava = rows;
        for i in 0..w.grid.len {
            if w.grid.floor[i] == ct::LAVA {
                top_lava = top_lava.min(i / w.grid.cols);
            }
        }
        assert!(top_lava < rows, "应该真的有岩浆");
        assert!(
            top_lava as f64 > rows as f64 * 0.6,
            "岩浆跑到了第 {} 行（共 {} 行），太靠上了",
            top_lava,
            rows
        );
    }

    /// 水不能把世界切成两半：浅滩必须存在，否则"蹚水 vs 绕路"这个取舍不存在。
    #[test]
    fn rivers_have_fords() {
        let w = build_rich(55);
        // 逐行数水：只要有水，就说明河存在；再检查**至少有一行几乎没有水**
        // （浅滩行），否则整条河是连续的墙。
        let mut wet_rows = 0;
        let mut dry_rows_in_river_cols = 0;
        for row in 0..w.grid.rows {
            let n = (0..w.grid.cols)
                .filter(|&c| {
                    w.grid
                        .index(c as i32, row as i32)
                        .map(|i| w.grid.floor[i] == ct::WATER)
                        .unwrap_or(false)
                })
                .count();
            if n > 0 {
                wet_rows += 1;
            } else {
                dry_rows_in_river_cols += 1;
            }
        }
        assert!(wet_rows > 10, "应该有一条从上到下的河，实测只有 {wet_rows} 行有水");
        assert!(
            dry_rows_in_river_cols > 5,
            "整张图每一行都有水——河没有浅滩，等于一堵墙"
        );
    }
}
