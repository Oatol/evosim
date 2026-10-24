//! 体型扫描：繁殖成本指数 p 与体型上限对演化结果的影响。
//!
//! 用法：
//! ```text
//! cargo run --release -p evosim-core --example body_scan -- <ticks> [seeds...]
//! ```
//!
//! 为什么要有这个：`breed_cost_exponent` 的默认值必须是 1.0（黄金快照锁着），
//! 所以"到底该给多少"只能靠扫描得出证据，不能凭感觉改默认值。
//!
//! **必须多种子取平均**：这个模拟处在临界状态，同一个 p 换个种子，种群能从 256
//! 跳到 1157（4.5 倍）。单种子单点的结论不可信——第一次扫就是这么被坑的。
//!
//! p 的**方向**容易搞反：`food_needed = n^p`，p > 1 让大体型更贵（体型变小），
//! p < 1 让大体型更便宜（体型变大）。想看到"大体型"就得往 1 以下扫。

use evosim_core::anatomy::Flags;
use evosim_core::cell_type::ct;
use evosim_core::config::Config;
use evosim_core::factory::{seed_origin_of_life, spawn_population};
use evosim_core::organism::Income;
use evosim_core::rng::Rng;
use evosim_core::world::World;
use evosim_core::worldgen::generate_demo_world;

/// 一次运行的结果
struct Row {
    pop: f64,
    body_avg: f64,
    body_max: usize,
    species: f64,
    /// **纯异养**：有嘴、但**没有**生产者细胞 —— 它必须靠吃别人活着。
    /// 这是"捕食生态位到底站没站住"最硬的读数；光看"嘴%"会把
    /// "顺手长了一张嘴的生产者"也算进去。
    hetero: f64,
    /// 带嘴的占比（含混养）
    mouth: f64,
    /// 带杀手的占比
    killer: f64,
    /// 繁殖尝试 / 成功 / 各种受阻（诊断：体型为什么涨不上去）
    tries: u64,
    ok: u64,
    blk_space: u64,
    blk_path: u64,
    blk_cap: u64,
    blk_small: u64,
    /// 有光格占用率（%）：`被生物占据的格子 / 有光格子`。
    /// 判断"世界到底挤不挤"的唯一硬指标——温度生态位需要"别处太挤"才有驱动力。
    occ: f64,
    /// 按温度分带：每带的（个体数, 平均体型）。5 带，每带 52 点温度。
    ///
    /// **必须分带看**：生物只占 5% 的格子，热区又只是一小块，
    /// 全局平均体型会把"大个体挤进热区"这件事完全稀释掉。
    bands: [(usize, f64); 5],
    /// 按**所在格光照质量**分带（0–25% / 25–50% / 50–75% / 75–100% 满照）。
    ///
    /// 这是"富贫斑块到底有没有分化出两种体型"的直接读数。
    /// 全局平均会把"富区大体型 + 贫区小体型"平成一个中间值，必须分带看。
    light_bands: [(usize, f64); 4],
    /// **按累计能量收入**判定的构成（光合 / 吃食物 / 咬活体 / 母体供养 / 没收入）。
    ///
    /// 和上面的 `hetero` 是两套判据，别混：
    /// - `hetero` 看**结构**（有嘴、没有生产者）；
    /// - 这里看**行为**（能量到底从哪来）。「生产者 + 嘴」在结构上是自养，
    ///   但如果九成收入来自咬活体，这里会把它算成捕食者。
    income: [f64; 5],
    /// 收入里「咬活体」占一半以上的个体占比（%）
    pred_share: f64,
    /// 捕食为主（咬 > 50%）里，**没有生产者**的占比（% of 全体）
    pred_pure: f64,
    /// 捕食为主（咬 > 50%）里，**带生产者**的占比（% of 全体）
    pred_mix: f64,
    /// **成体**里的结构异养（有嘴、无生产者），% of 成体。
    ///
    /// 和 `hetero` 的区别只有一条：**芽不算**。芽的 `anatomy` 只有 1 格、
    /// 蓝图在 `growth_plan` 里，如果那一格恰好是嘴，就会被旧判据算成
    /// "结构异养"——把「还没长完的芽」误读成「纯捕食者」。
    hetero_adult: f64,
    /// 芽（`growth_plan` 还在）占全体的比例（%）
    buds: f64,
    /// **成体**平均体型（格）。`body_avg` 被 1 格的芽稀释，这个才是"长成的身体多大"
    body_adult: f64,
    /// 有运动细胞的个体占比（%）。大脑只在这部分个体上起作用。
    movers: f64,
    /// 全图平均地力（0~100%）。关掉地力时恒为 100。
    soil: f64,

    // ── 批次 38：移动者 vs 非移动者的**一阶量** ──
    //
    // 种群是二阶量，3 种子噪声 ±30%（批次 37 实测）。这一组是「每个个体自己的账」，
    // 400 个个体 × 6000 tick 会把个体级噪声平均掉——用它才量得出策略好坏。
    mover_n: f64,
    mover_age: f64,
    mover_income: f64,
    mover_life: f64,
    mover_soil: f64,
    still_age: f64,
    still_income: f64,
    still_life: f64,
    /// 每个种子的个体数 / 移动者收入（看噪声用；只有 `run_seeds` 填）
    per_seed_pop: Vec<f64>,
    per_seed_minc: Vec<f64>,
    /// **聚类物种数**（构成距离 ≤ `kin_distance`，个体数 ≥ 2）。
    /// `species` 那一列是精确指纹——变异一格就换一个，数的是结构变体累计。
    species_clustered: f64,
    /// 体型分布：1 格 / 2 格 / … 的个体占比（只统计前 10 档，够看形状）
    hist: [f64; 10],

    // ── 批次 47：体型收益 ──
    /// **成体**的平均「体表/体积比」`8n / 暴露面数`（1 格 = 1.00）。
    /// 这是新机制唯一的自变量：它涨了，收益才可能涨。
    svr: f64,
    /// 成体的平均产出倍率（= `svr^exposure_prod_exp`）。关掉时恒为 1.00。
    prod_mult: f64,
    /// 成体里「紧凑度」分布：`svr ≥ 2`（明显比单格厚实）的占比（%）
    compact: f64,
    /// **有附肢的成体占比（%）**：展开后蓝图里存在 depth ≥ 1 的格。
    /// 阶段 2（段嵌套）的验收指标——「躯干 + 附肢」到底有没有长出来。
    limbs: f64,
    /// 成体蓝图里出现过的最大嵌套深度
    depth_max: u8,
}

fn run(
    p: f64,
    cap: usize,
    ticks: u64,
    seed: u32,
    free: bool,
    tries: usize,
    min_cells: usize,
    cost_mul: f64,
    upkeep_p: f64,
    heat: usize,
    temp_dmg: f64,
    photo: f64,
    ambient: f64,
    comfort: f64,
    growth: bool,
    gcost: f64,
    stall: u32,
    cloud: f64,
    rich: f64,
    mingrow: u32,
    bud: bool,
    budper: f64,
    budfeed: f64,
    oldbite: bool,
    bite: f64,
    shade: f64,
    oldmouth: bool,
    day: u32,
    night: f64,
    sizeadv: f64,
    kin: f64,
    spdist: f64,
    kinspec: bool,
    brain: u8,
    starve: f64,
    produce: bool,
    soil: u8,
    soilr: u8,
    soilf: f64,
    rec: bool,
    rot: bool,
    flat: bool,
    cave: i32,
    cooldown: u32,
    egg: bool,
    hatch: u32,
    spec: f64,
    expprod: f64,
    expup: f64,
) -> Row {
    let mut cfg = Config::default();
    cfg.life.breed_cost_exponent = p;
    cfg.life.max_body_cells = cap;
    // `free = true`：繁殖失败**不扣能量**（「找不到地方就继续攒着」）。
    // 旧行为是失败也白付一次 `food_needed`，而实测 86.5% 的尝试都会失败。
    cfg.life.repro_fail_costs_energy = !free;
    // `tries`：每次繁殖尝试最多试几个候选位置（1 = 旧行为）
    cfg.life.repro_search_tries = tries;
    // `min_cells`：最小可繁殖体型（1 = 旧行为）
    cfg.life.min_breed_cells = min_cells;
    // `costmul`：所有细胞的 cost 同乘一个数（用来调「成本占收入的比例」）
    if (cost_mul - 1.0).abs() > 1e-12 {
        for c in cfg.cells.iter_mut() {
            c.cost *= cost_mul;
        }
    }
    // `upkeep`：维持成本随体型的指数（1 = 线性；0.75 = Kleiber）
    cfg.life.upkeep_exponent = upkeep_p;
    // `heatdmg`：环境温度伤害（0 = 关闭）
    cfg.life.temp_damage = temp_dmg;
    // `photo`：光合速率（默认 0.12）。调高 = 世界更拥挤
    cfg.life.photosynthesis_rate = photo;
    // `ambient` / `comfort`：环境温度与舒适温度（摄氏度）
    cfg.world.ambient_temp = ambient;
    cfg.life.comfort_temp = comfort;
    // `growth` / `gcost`：成长过程与每格生长成本
    cfg.life.growth_enabled = growth;
    cfg.life.growth_cost = gcost;
    // `stall`：连续卡多少 tick 就提前成熟（0 = 必须长满）
    cfg.life.growth_stall_mature = stall;
    // `mingrow`：最短成长期（0 = 关闭，1 格目标出生即成年）
    cfg.life.growth_min_ticks = mingrow;
    // `bud`：出芽（芽紧贴母体 + 母体供养 + 芽位随体型涨）
    cfg.life.budding_enabled = bud;
    cfg.life.bud_cells_per_slot = budper;
    cfg.life.bud_feed = budfeed;
    // `oldbite`：退回旧的「按接触面积叠加」伤害
    cfg.combat.killer_bite_capped = !oldbite;
    // `bite=R`：嘴每 tick 能从活体身上抽多少能量（默认 0.06，是满照光合 0.12 的一半）
    cfg.life.bite_rate = bite;
    // `shade=K`：遮挡强度（默认 0.25，0 = 关掉）
    cfg.life.shade_strength = shade;
    // `oldmouth`：退回「一张嘴每 tick 咬满 4 个邻居」（批次 9.5 的行为）
    cfg.life.mouth_one_bite = !oldmouth;
    // `day=N`：昼夜周期（0 = 关闭 = 永远正午）；`night=k`：夜间剩余日照比例
    cfg.world.day_length = day;
    cfg.world.night_floor = night;
    // `sizeadv=K`：捕食端体型差指数（0 = 关闭）
    cfg.life.size_advantage = sizeadv;
    // `kin=P`：同种相残惩罚（0 = 关闭，1 = 同种咬不动）
    cfg.life.kin_bite_penalty = kin;
    // `spd=X`：物种聚类阈值（Jaccard 构成距离）；`kinspec`：同种相残按聚类判
    cfg.life.species_distance = spdist;
    cfg.life.kin_by_species = kinspec;
    // `brain=N`：大脑消融（0 正常 / 1 恒定方向 / 2 每 tick 随机）
    cfg.life.brain_ablation = brain;
    // `starve=M`：纯捕食者的抗饿倍率（1.0 = 关闭）
    cfg.life.predator_starve_mult = starve;
    // `produce`：有运动细胞也能光合
    cfg.life.movers_can_produce = produce;
    // `soil=D` / `soilr=R` / `soilf=F`：地力耗竭
    cfg.life.soil_deplete = soil;
    cfg.life.soil_recover = soilr;
    cfg.life.soil_floor = soilf;
    // `rec`：循环连接
    cfg.brain.recurrent = rec;
    // `rot`：没有运动细胞也能转身（免费换地）
    cfg.life.rotate_without_mover = rot;
    // `cave=N`：世界生成时挖几条带顶隧道（0 = 关闭）
    cfg.world.cave_count = cave;
    // `cd=N`：繁殖失败后的冷却 tick（0 = 关闭）
    cfg.life.repro_cooldown = cooldown;
    // `egg`：卵生；`hatch=N`：孵化时间
    cfg.life.egging = egg;
    cfg.life.egg_hatch_ticks = hatch;
    // `spec=K`：捕食专精奖励（1.0 = 关闭）
    cfg.life.predator_specialization = spec;
    // `expprod=K` / `expup=K`：体型收益（批次 47），0 = 关闭
    cfg.life.exposure_prod_exp = expprod;
    cfg.life.exposure_upkeep_exp = expup;
    // `cloud`：云影（光岛 + 暗廊），用来造空间异质性
    cfg.world.sky_patchiness = cloud;
    // `rich`：富贫斑块（光岛 + 微光暗廊 + 食物跟着走）
    cfg.world.patch_richness = rich;
    // `flat`：关掉丰富地形（批次 44），退回「墙 + 草甸」的老世界
    cfg.world.rich_terrain = !flat;

    let mut w = World::new(cfg, Rng::new(seed));
    generate_demo_world(&mut w, 1.0, 1.0);

    // `heat=N`：在世界上半部分铺 N 块岩浆当热源。
    // 温度场靠 `update_fields` 扩散，所以下面每 tick 推一次。
    if heat > 0 {
        for k in 0..heat {
            let cx = (w.grid.cols as i32) * (k as i32 + 1) / (heat as i32 + 1);
            let cy = 20 + (k as i32 % 3) * 12;
            for dr in -4..=4 {
                for dc in -4..=4 {
                    if dc * dc + dr * dr <= 16 {
                        w.set_cell(cx + dc, cy + dr, ct::LAVA, -1);
                    }
                }
            }
        }
    }

    seed_origin_of_life(&mut w);
    spawn_population(&mut w, 8, 3);
    for _ in 0..ticks {
        w.step();
        // 主程序按 60 Hz 推温度场；这里跟 tick 走，保证热区真的热起来
        w.update_fields();
    }

    // 强制刷新一次物种统计：`species.update()` 只在 `stats.interval` 的整数倍跑，
    // 跑完最后一 tick 时 `alive_count` 可能是几十 tick 前的快照——和刚算出来的
    // 「物种(聚类)」不是同一时刻的数，两者不能直接比。
    {
        let fps: Vec<std::sync::Arc<str>> = w
            .organisms
            .iter()
            .filter_map(|o| o.species_fingerprint.clone())
            .collect();
        w.species.update(fps.iter(), w.tick);
    }

    if std::env::var("EVOSIM_DBG").is_ok() {
        let mut fps: Vec<&str> = w
            .organisms
            .iter()
            .filter_map(|o| o.species_fingerprint.as_deref())
            .collect();
        fps.sort_unstable();
        fps.dedup();
        println!(
            "[dbg] 个体 {} · 有指纹 {} · 不同指纹 {} · map {} · alive {} · 聚类(≥1) {} · 聚类(≥2) {}",
            w.organisms.len(),
            w.organisms
                .iter()
                .filter(|o| o.species_fingerprint.is_some())
                .count(),
            fps.len(),
            w.species.map.len(),
            w.species.alive_count,
            w.species_clusters(1),
            w.species_clusters(2)
        );
    }

    let st = w.organism_stats();
    let n = w.organisms.len().max(1);

    let mut hist = [0f64; 10];
    for o in &w.organisms {
        let c = o.cell_count();
        if (1..=10).contains(&c) {
            hist[c - 1] += 100.0 / n as f64;
        }
    }

    let pct = |f: u16| {
        w.organisms.iter().filter(|o| o.anatomy.has(f)).count() as f64 * 100.0 / n as f64
    };
    // 按收入来源分类。`Nothing` 是"还没挣到钱"（刚出生 / 快饿死），
    // 单独一列——把它并进任何一类都会污染结论。
    let mut inc = [0f64; 5];
    let mut pred = 0f64;
    let mut pure = 0f64;
    let mut mix = 0f64;
    for o in &w.organisms {
        let k = match o.income_source() {
            Income::Photo => 0,
            Income::Scavenge => 1,
            Income::Predate => 2,
            Income::Gift => 3,
            Income::Nothing => 4,
        };
        inc[k] += 1.0;
        let total = o.income_total();
        if total > 1e-9 && o.gain_bite / total > 0.5 {
            pred += 1.0;
            if o.anatomy.has(Flags::PRODUCER) {
                mix += 1.0;
            } else {
                pure += 1.0;
            }
        }
    }
    for v in inc.iter_mut() {
        *v = *v * 100.0 / n as f64;
    }
    let pred_share = pred * 100.0 / n as f64;
    let adult_n = w
        .organisms
        .iter()
        .filter(|o| o.growth_plan.is_none())
        .count();
    let hetero_adult = if adult_n == 0 {
        0.0
    } else {
        w.organisms
            .iter()
            .filter(|o| o.growth_plan.is_none())
            .filter(|o| o.anatomy.has(Flags::MOUTH) && !o.anatomy.has(Flags::PRODUCER))
            .count() as f64
            * 100.0
            / adult_n as f64
    };
    let buds = (w.organisms.len() - adult_n) as f64 * 100.0 / n as f64;
    let body_adult = if adult_n == 0 {
        0.0
    } else {
        w.organisms
            .iter()
            .filter(|o| o.growth_plan.is_none())
            .map(|o| o.cell_count() as f64)
            .sum::<f64>()
            / adult_n as f64
    };
    let pred_pure = pure * 100.0 / n as f64;
    let pred_mix = mix * 100.0 / n as f64;

    // ── 批次 47：成体的体表/体积比 ──
    // **只统计成体**：芽的 anatomy 只有已经长出来的那几格，`svr` 会偏低，
    // 混进来会把"体型收益到底有没有被吃到"这个问题糊掉。
    let mut svr_sum = 0.0;
    let mut mult_sum = 0.0;
    let mut compact_n = 0usize;
    for o in w.organisms.iter().filter(|o| o.growth_plan.is_none()) {
        let r = o.surface_volume_ratio();
        svr_sum += r;
        mult_sum += o.size_prod_mult(&w.config);
        if r >= 2.0 {
            compact_n += 1;
        }
    }
    let svr = if adult_n == 0 { 0.0 } else { svr_sum / adult_n as f64 };
    let prod_mult = if adult_n == 0 { 0.0 } else { mult_sum / adult_n as f64 };
    let compact = if adult_n == 0 { 0.0 } else { compact_n as f64 * 100.0 / adult_n as f64 };

    // ── 阶段 2：附肢（嵌套深度 ≥ 1）──
    //
    // 成体没有 `growth_plan`，所以要重新展开一次基因组才看得到深度。
    // 只在诊断里做——模拟主循环里不需要这个量。
    let mut limb_n = 0usize;
    let mut depth_max = 0u8;
    for o in w.organisms.iter().filter(|o| o.growth_plan.is_none()) {
        let plan = o.effective_genome().expand(w.config.life.max_body_cells);
        let d = plan.depth.iter().copied().max().unwrap_or(0);
        depth_max = depth_max.max(d);
        if d >= 1 {
            limb_n += 1;
        }
    }
    let limbs = if adult_n == 0 { 0.0 } else { limb_n as f64 * 100.0 / adult_n as f64 };

    Row {
        pop: w.organisms.len() as f64,
        body_avg: st.avg_cells,
        body_max: st.max_cells,
        species: w.species.alive_count as f64,
        hetero: w
            .organisms
            .iter()
            .filter(|o| {
                o.anatomy.has(Flags::MOUTH) && !o.anatomy.has(Flags::PRODUCER)
            })
            .count() as f64
            * 100.0
            / n as f64,
        mouth: pct(Flags::MOUTH),
        killer: pct(Flags::KILLER),
        income: inc,
        pred_share,
        pred_pure,
        pred_mix,
        hetero_adult,
        buds,
        body_adult,
        species_clustered: st.species_clustered as f64,
        mover_n: st.mover_n as f64,
        mover_age: st.mover_age,
        mover_income: st.mover_income,
        mover_life: st.mover_life,
        mover_soil: st.mover_soil,
        still_age: st.still_age,
        still_income: st.still_income,
        still_life: st.still_life,
        per_seed_pop: Vec::new(),
        per_seed_minc: Vec::new(),
        movers: st.movers as f64 * 100.0 / n as f64,
        soil: if w.soil.is_empty() {
            0.0
        } else {
            w.soil.iter().map(|&v| v as f64).sum::<f64>() * 100.0
                / (255.0 * w.soil.len() as f64)
        },
        occ: {
            let lit = w.light.level.iter().filter(|&&v| v > 0).count().max(1);
            let occ = w.grid.owners.iter().filter(|&&o| o >= 0).count();
            occ as f64 * 100.0 / lit as f64
        },
        light_bands: {
            let mut cnt = [0usize; 4];
            let mut sum = [0usize; 4];
            for o in &w.organisms {
                let l = w.light.sky_at(&w.grid, o.col, o.row) as usize;
                let b = (l * 4 / 256).min(3);
                cnt[b] += 1;
                sum[b] += o.cell_count();
            }
            let mut out = [(0usize, 0.0f64); 4];
            for b in 0..4 {
                out[b] = (
                    cnt[b],
                    if cnt[b] == 0 {
                        0.0
                    } else {
                        sum[b] as f64 / cnt[b] as f64
                    },
                );
            }
            out
        },
        bands: {
            let mut cnt = [0usize; 5];
            let mut sum = [0usize; 5];
            for o in &w.organisms {
                let t = w.fields.temp_at(&w.grid, o.col, o.row) as usize;
                let b = (t / 52).min(4);
                cnt[b] += 1;
                sum[b] += o.cell_count();
            }
            let mut out = [(0usize, 0.0f64); 5];
            for b in 0..5 {
                out[b] = (
                    cnt[b],
                    if cnt[b] == 0 {
                        0.0
                    } else {
                        sum[b] as f64 / cnt[b] as f64
                    },
                );
            }
            out
        },
        tries: w.repro_tries,
        ok: w.repro_ok,
        blk_space: w.repro_blocked_space,
        blk_path: w.repro_blocked_path,
        blk_cap: w.repro_blocked_cap,
        blk_small: w.repro_blocked_too_small,
        hist,
        svr,
        prod_mult,
        compact,
        limbs,
        depth_max,
    }
}

/// 同参数跑多个种子，取平均（`body_max` 取最大，因为要看"最极端的那只"）
fn run_seeds(
    p: f64,
    cap: usize,
    ticks: u64,
    seeds: &[u32],
    free: bool,
    tries: usize,
    min_cells: usize,
    cost_mul: f64,
    upkeep_p: f64,
    heat: usize,
    temp_dmg: f64,
    photo: f64,
    ambient: f64,
    comfort: f64,
    growth: bool,
    gcost: f64,
    stall: u32,
    cloud: f64,
    rich: f64,
    mingrow: u32,
    bud: bool,
    budper: f64,
    budfeed: f64,
    oldbite: bool,
    bite: f64,
    shade: f64,
    oldmouth: bool,
    day: u32,
    night: f64,
    sizeadv: f64,
    kin: f64,
    spdist: f64,
    kinspec: bool,
    brain: u8,
    starve: f64,
    produce: bool,
    soil: u8,
    soilr: u8,
    soilf: f64,
    rec: bool,
    rot: bool,
    flat: bool,
    cave: i32,
    cooldown: u32,
    egg: bool,
    hatch: u32,
    spec: f64,
    expprod: f64,
    expup: f64,
) -> Row {
    let mut acc = Row {
        pop: 0.0,
        body_avg: 0.0,
        body_max: 0,
        species: 0.0,
        hetero: 0.0,
        mouth: 0.0,
        killer: 0.0,
        income: [0.0; 5],
        pred_share: 0.0,
        hetero_adult: 0.0,
        buds: 0.0,
        body_adult: 0.0,
        species_clustered: 0.0,
        movers: 0.0,
        soil: 0.0,
        mover_n: 0.0,
        mover_age: 0.0,
        mover_income: 0.0,
        mover_life: 0.0,
        mover_soil: 0.0,
        still_age: 0.0,
        still_income: 0.0,
        still_life: 0.0,
        per_seed_pop: Vec::new(),
        per_seed_minc: Vec::new(),
        pred_pure: 0.0,
        pred_mix: 0.0,
        occ: 0.0,
        bands: [(0, 0.0); 5],
        light_bands: [(0, 0.0); 4],
        tries: 0,
        ok: 0,
        blk_space: 0,
        blk_path: 0,
        blk_cap: 0,
        blk_small: 0,
        hist: [0f64; 10],
        svr: 0.0,
        prod_mult: 0.0,
        compact: 0.0,
        limbs: 0.0,
        depth_max: 0,
    };
    for &s in seeds {
        let r = run(
            p, cap, ticks, s, free, tries, min_cells, cost_mul, upkeep_p, heat, temp_dmg, photo,
            ambient, comfort, growth, gcost, stall, cloud, rich, mingrow, bud, budper, budfeed,
            oldbite, bite, shade, oldmouth, day, night, sizeadv, kin, spdist, kinspec, brain, starve, produce, soil, soilr, soilf, rec, rot, flat, cave, cooldown, egg, hatch, spec,
            expprod, expup,
        );
        acc.pop += r.pop;
        acc.body_avg += r.body_avg;
        acc.body_max = acc.body_max.max(r.body_max);
        acc.species += r.species;
        acc.hetero += r.hetero;
        acc.mouth += r.mouth;
        acc.killer += r.killer;
        for k in 0..5 {
            acc.income[k] += r.income[k];
        }
        acc.pred_share += r.pred_share;
        acc.hetero_adult += r.hetero_adult;
        acc.buds += r.buds;
        acc.body_adult += r.body_adult;
        acc.species_clustered += r.species_clustered;
        acc.movers += r.movers;
        acc.soil += r.soil;
        acc.mover_n += r.mover_n;
        acc.mover_age += r.mover_age;
        acc.mover_income += r.mover_income;
        acc.mover_life += r.mover_life;
        acc.mover_soil += r.mover_soil;
        acc.still_age += r.still_age;
        acc.still_income += r.still_income;
        acc.still_life += r.still_life;
        acc.per_seed_pop.push(r.pop);
        acc.per_seed_minc.push(r.mover_income);
        acc.pred_pure += r.pred_pure;
        acc.pred_mix += r.pred_mix;
        acc.occ += r.occ;
        for b in 0..5 {
            acc.bands[b].0 += r.bands[b].0;
            acc.bands[b].1 += r.bands[b].1;
        }
        for b in 0..4 {
            acc.light_bands[b].0 += r.light_bands[b].0;
            acc.light_bands[b].1 += r.light_bands[b].1;
        }
        acc.tries += r.tries;
        acc.ok += r.ok;
        acc.blk_space += r.blk_space;
        acc.blk_path += r.blk_path;
        acc.blk_cap += r.blk_cap;
        acc.blk_small += r.blk_small;
        for k in 0..10 {
            acc.hist[k] += r.hist[k];
        }
        acc.svr += r.svr;
        acc.prod_mult += r.prod_mult;
        acc.compact += r.compact;
        acc.limbs += r.limbs;
        acc.depth_max = acc.depth_max.max(r.depth_max);
    }
    let n = seeds.len() as f64;
    acc.pop /= n;
    acc.body_avg /= n;
    acc.species /= n;
    acc.hetero /= n;
    acc.mouth /= n;
    acc.killer /= n;
    for k in 0..5 {
        acc.income[k] /= n;
    }
    acc.pred_share /= n;
    acc.hetero_adult /= n;
    acc.buds /= n;
    acc.body_adult /= n;
    acc.species_clustered /= n;
    acc.movers /= n;
    acc.soil /= n;
    acc.mover_n /= n;
    acc.mover_age /= n;
    acc.mover_income /= n;
    acc.mover_life /= n;
    acc.mover_soil /= n;
    acc.still_age /= n;
    acc.still_income /= n;
    acc.still_life /= n;
    acc.pred_pure /= n;
    acc.pred_mix /= n;
    acc.occ /= n;
    for b in 0..5 {
        acc.bands[b].1 /= n;
    }
    for b in 0..4 {
        acc.light_bands[b].1 /= n;
    }
    for k in 0..10 {
        acc.hist[k] /= n;
    }
    acc.svr /= n;
    acc.prod_mult /= n;
    acc.compact /= n;
    acc.limbs /= n;
    acc
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // `free`：繁殖失败不扣能量。**默认就是 true**（跟 `Config` 默认一致），
    // 要旧行为请显式传 `strict`。
    let free = !args.iter().any(|a| a == "strict");
    // `tries=N`：每次繁殖尝试的候选位置数（1 = 旧行为）
    let tries: usize = args
        .iter()
        .find_map(|a| a.strip_prefix("tries=").and_then(|v| v.parse().ok()))
        .unwrap_or(1);
    // `mincells=N`：最小可繁殖体型（1 = 旧行为）
    let min_cells: usize = args
        .iter()
        .find_map(|a| a.strip_prefix("mincells=").and_then(|v| v.parse().ok()))
        .unwrap_or(1);
    // `costmul=M`：所有细胞 cost 同乘 M（调「成本占收入的比例」）
    let cost_mul: f64 = args
        .iter()
        .find_map(|a| a.strip_prefix("costmul=").and_then(|v| v.parse().ok()))
        .unwrap_or(1.0);
    // `upkeep=P`：维持成本随体型的指数（1 = 线性，0.75 = Kleiber）
    let upkeep_p: f64 = args
        .iter()
        .find_map(|a| a.strip_prefix("upkeep=").and_then(|v| v.parse().ok()))
        .unwrap_or(1.0);
    // `heat=N`：铺 N 块岩浆当热源
    let heat: usize = args
        .iter()
        .find_map(|a| a.strip_prefix("heat=").and_then(|v| v.parse().ok()))
        .unwrap_or(0);
    // `heatdmg=D`：环境温度每 tick 伤害（0 = 关闭）
    let temp_dmg: f64 = args
        .iter()
        .find_map(|a| a.strip_prefix("heatdmg=").and_then(|v| v.parse().ok()))
        .unwrap_or(0.0);
    // `photo=R`：光合速率（默认 0.12）。调高 = 世界更拥挤，用来测"热区避难"
    let photo: f64 = args
        .iter()
        .find_map(|a| a.strip_prefix("photo=").and_then(|v| v.parse().ok()))
        .unwrap_or(Config::default().life.photosynthesis_rate);
    // `ambient=C`：环境温度（℃）；`comfort=C`：舒适温度（℃）
    let ambient: f64 = args
        .iter()
        .find_map(|a| a.strip_prefix("ambient=").and_then(|v| v.parse().ok()))
        .unwrap_or(Config::default().world.ambient_temp);
    let comfort: f64 = args
        .iter()
        .find_map(|a| a.strip_prefix("comfort=").and_then(|v| v.parse().ok()))
        .unwrap_or(Config::default().life.comfort_temp);
    // `growth`：启用成长过程（芽从 1 格长到目标体型）；`gcost=C`：每格生长成本
    // 默认**跟 Config 走**（批次 17 起成长是默认开的）。
    // `growth` / `bud` 仍然接受（写了就是开），`nogrowth` / `nobud` 才是关。
    let growth = if args.iter().any(|a| a == "nogrowth") {
        false
    } else if args.iter().any(|a| a == "growth") {
        true
    } else {
        Config::default().life.growth_enabled
    };
    let gcost: f64 = args
        .iter()
        .find_map(|a| a.strip_prefix("gcost=").and_then(|v| v.parse().ok()))
        .unwrap_or(Config::default().life.growth_cost);
    // `stall=N`：连续卡 N tick 就提前成熟（0 = 必须长满）
    let stall: u32 = args
        .iter()
        .find_map(|a| a.strip_prefix("stall=").and_then(|v| v.parse().ok()))
        .unwrap_or(Config::default().life.growth_stall_mature);
    // `cloud=P`：云影强度（0 = 无，1 = 光岛 + 暗廊）
    let cloud: f64 = args
        .iter()
        .find_map(|a| a.strip_prefix("cloud=").and_then(|v| v.parse().ok()))
        .unwrap_or(Config::default().world.sky_patchiness);
    // `rich=R`：富贫斑块强度（0 = 关闭）
    let rich: f64 = args
        .iter()
        .find_map(|a| a.strip_prefix("rich=").and_then(|v| v.parse().ok()))
        .unwrap_or(Config::default().world.patch_richness);
    // `mingrow=N`：最短成长期（0 = 关闭）
    let mingrow: u32 = args
        .iter()
        .find_map(|a| a.strip_prefix("mingrow=").and_then(|v| v.parse().ok()))
        .unwrap_or(Config::default().life.growth_min_ticks);
    // `bud`：出芽
    let bud = if args.iter().any(|a| a == "nobud") {
        false
    } else if args.iter().any(|a| a == "bud") {
        true
    } else {
        Config::default().life.budding_enabled
    };
    let oldbite = args.iter().any(|a| a == "oldbite");
    // `oldmouth`：一张嘴每 tick 咬满 4 个邻居（批次 9.5 的行为）
    let oldmouth = args.iter().any(|a| a == "oldmouth");
    // `day=N`：昼夜周期（0 = 关闭 = 永远正午）
    let day: u32 = args
        .iter()
        .find_map(|a| a.strip_prefix("day=").and_then(|v| v.parse().ok()))
        .unwrap_or(Config::default().world.day_length);
    // `night=k`：夜间剩余日照比例（默认 0.05）
    let night: f64 = args
        .iter()
        .find_map(|a| a.strip_prefix("night=").and_then(|v| v.parse().ok()))
        .unwrap_or(Config::default().world.night_floor);
    // `sizeadv=K`：捕食端体型差（0 = 关闭）
    let sizeadv: f64 = args
        .iter()
        .find_map(|a| a.strip_prefix("sizeadv=").and_then(|v| v.parse().ok()))
        .unwrap_or(Config::default().life.size_advantage);
    // `kin=P`：同种相残惩罚（0 = 关闭）
    let kin: f64 = args
        .iter()
        .find_map(|a| a.strip_prefix("kin=").and_then(|v| v.parse().ok()))
        .unwrap_or(Config::default().life.kin_bite_penalty);
    // `spd=X`：物种聚类阈值（Jaccard 构成距离）
    let spdist: f64 = args
        .iter()
        .find_map(|a| a.strip_prefix("spd=").and_then(|v| v.parse().ok()))
        .unwrap_or(Config::default().life.species_distance);
    // `kinspec`：同种相残按聚类判（默认还是指纹）
    let kinspec = args.iter().any(|a| a == "kinspec");
    // `brain=N`：大脑消融（0 正常 / 1 恒定方向 / 2 每 tick 随机）
    let brain: u8 = args
        .iter()
        .find_map(|a| a.strip_prefix("brain=").and_then(|v| v.parse().ok()))
        .unwrap_or(Config::default().life.brain_ablation);
    // `starve=M`：纯捕食者抗饿倍率（1.0 = 关闭）
    let starve: f64 = args
        .iter()
        .find_map(|a| a.strip_prefix("starve=").and_then(|v| v.parse().ok()))
        .unwrap_or(Config::default().life.predator_starve_mult);
    // `produce`：有运动细胞也能光合（默认不能）
    let produce = args.iter().any(|a| a == "produce");
    // `rec`：循环连接（大脑带记忆）
    let rec = args.iter().any(|a| a == "rec");
    // `rot`：没有运动细胞也能转身
    let rot = args.iter().any(|a| a == "rot");
    // `flat`：关掉丰富地形（批次 44），用来做"有地形 / 没地形"的 A/B
    let flat = args.iter().any(|a| a == "flat");
    // `soil=D` / `soilr=R` / `soilf=F`：地力耗竭（D=0 关闭）
    let soil: u8 = args
        .iter()
        .find_map(|a| a.strip_prefix("soil=").and_then(|v| v.parse().ok()))
        .unwrap_or(Config::default().life.soil_deplete);
    let soilr: u8 = args
        .iter()
        .find_map(|a| a.strip_prefix("soilr=").and_then(|v| v.parse().ok()))
        .unwrap_or(Config::default().life.soil_recover);
    let soilf: f64 = args
        .iter()
        .find_map(|a| a.strip_prefix("soilf=").and_then(|v| v.parse().ok()))
        .unwrap_or(Config::default().life.soil_floor);
    // `cave=N`：洞穴数量（0 = 关闭）
    let cave: i32 = args
        .iter()
        .find_map(|a| a.strip_prefix("cave=").and_then(|v| v.parse().ok()))
        .unwrap_or(Config::default().world.cave_count);
    // `cd=N`：繁殖失败冷却（0 = 关闭）
    let cooldown: u32 = args
        .iter()
        .find_map(|a| a.strip_prefix("cd=").and_then(|v| v.parse().ok()))
        .unwrap_or(Config::default().life.repro_cooldown);
    // `egg`：卵生（默认跟 Config 走）
    let egg = if args.iter().any(|a| a == "noegg") {
        false
    } else if args.iter().any(|a| a == "egg") {
        true
    } else {
        Config::default().life.egging
    };
    // `hatch=N`：卵的孵化时间
    let hatch: u32 = args
        .iter()
        .find_map(|a| a.strip_prefix("hatch=").and_then(|v| v.parse().ok()))
        .unwrap_or(Config::default().life.egg_hatch_ticks);
    // `spec=K`：捕食专精奖励（1.0 = 关闭）
    let spec: f64 = args
        .iter()
        .find_map(|a| a.strip_prefix("spec=").and_then(|v| v.parse().ok()))
        .unwrap_or(Config::default().life.predator_specialization);
    // `expprod=K` / `expup=K`：体型收益（批次 47），0 = 关闭
    let expprod: f64 = args
        .iter()
        .find_map(|a| a.strip_prefix("expprod=").and_then(|v| v.parse().ok()))
        .unwrap_or(Config::default().life.exposure_prod_exp);
    let expup: f64 = args
        .iter()
        .find_map(|a| a.strip_prefix("expup=").and_then(|v| v.parse().ok()))
        .unwrap_or(Config::default().life.exposure_upkeep_exp);
    // `exponly`：只跑「体型收益」那一节（其余全跳过）。批次 47 的专用模式。
    let exponly = args.iter().any(|a| a == "exponly");
    // `shade=K`：遮挡强度（默认跟 Config 走）
    let shade: f64 = args
        .iter()
        .find_map(|a| a.strip_prefix("shade=").and_then(|v| v.parse().ok()))
        .unwrap_or(Config::default().life.shade_strength);
    // `bite=R`：咬活体的速率（默认跟 Config 走）
    let bite: f64 = args
        .iter()
        .find_map(|a| a.strip_prefix("bite=").and_then(|v| v.parse().ok()))
        .unwrap_or(Config::default().life.bite_rate);
    let budper: f64 = args
        .iter()
        .find_map(|a| a.strip_prefix("budper=").and_then(|v| v.parse().ok()))
        .unwrap_or(Config::default().life.bud_cells_per_slot);
    let budfeed: f64 = args
        .iter()
        .find_map(|a| a.strip_prefix("budfeed=").and_then(|v| v.parse().ok()))
        .unwrap_or(Config::default().life.bud_feed);
    // 位置参数：第一个数字是 ticks，其余是种子。
    // **必须把 ticks 从种子里剔掉**——第一版用 `filter_map(parse)` 全收，
    // 于是 `body_scan 10000 1001 1002 1003` 会把 10000 也当成种子跑（多跑一个）。
    let mut nums: Vec<u32> = args[1..].iter().filter_map(|s| s.parse().ok()).collect();
    let ticks: u64 = if nums.is_empty() {
        15000
    } else {
        nums.remove(0) as u64
    };
    let seeds: Vec<u32> = if nums.is_empty() {
        vec![1001, 777, 4242]
    } else {
        nums
    };
    assert!(!seeds.is_empty(), "至少要一个种子");

    println!(
        "── 体型扫描 · {ticks} tick · 种子 {:?} · 繁殖失败{}扣钱 · 搜索 {} · 最小体型 {} · cost×{:.1} · 维持 {:.2} · 岩浆 {} 块 · 热伤害 {:.2} · 光合 {:.2} · 咬 {:.2}{} · 昼夜 {} / 夜 {:.2} · 成长 {} ──",
        seeds,
        if free { "**不**" } else { "" },
        tries,
        min_cells,
        cost_mul,
        upkeep_p,
        heat,
        temp_dmg,
        photo,
        bite,
        if oldmouth { "(旧:四邻域各一口)" } else { "(一口一目标)" },
        if day == 0 { "关".to_string() } else { format!("{day} tick") },
        night,
        if growth {
            let mut t = format!("开(每格 {:.1}", gcost);
            if stall > 0 {
                t.push_str(&format!(" · 卡 {} 就成熟", stall));
            }
            if mingrow > 0 {
                t.push_str(&format!(" · 最短 {} tick", mingrow));
            }
            if bud {
                t.push_str(" · 出芽");
            }
            t.push(')');
            t
        } else {
            "关".to_string()
        }
    );
    println!("（同参数多跑几个种子取平均：这模拟在临界点上，单点差 4 倍很常见）\n");

    // ── 批次 47：体型收益扫描 ──
    //
    // 这是「结构性收益」的验收：`svr`（体表/体积比）必须真的被选择压推高，
    // 而且推高的方式应该是**变紧凑**，不只是**变多格**。
    // 判读顺序：先看「成体体型」有没有涨 → 再看「紧凑%」有没有涨 →
    // 最后看「个体数」有没有崩。只有前两个涨、第三个不崩，这条机制才算成立。
    {
        let ks: Vec<f64> = {
            let v: Vec<f64> = args
                .iter()
                .filter_map(|a| a.strip_prefix("expk=").and_then(|v| v.parse().ok()))
                .collect();
            if v.is_empty() {
                vec![0.0, 0.2, 0.3, 0.5]
            } else {
                v
            }
        };
        let ups: Vec<f64> = {
            let v: Vec<f64> = args
                .iter()
                .filter_map(|a| a.strip_prefix("expu=").and_then(|v| v.parse().ok()))
                .collect();
            if v.is_empty() {
                vec![0.0]
            } else {
                v
            }
        };
        println!("【0】体型收益：光合产出 × svr^k（svr = 8×格数 ÷ 暴露面数，1 格恒为 1.00）");
        println!(
            "{:>6}  {:>6}  {:>7}  {:>9}  {:>9}  {:>9}  {:>8}  {:>7}  {:>9}  {:>7}  {:>5}  {:>7}  {:>7}  {:>7}",
            "k产出", "k维持", "个体", "平均体型", "成体体型", "最大体型", "svr", "产出倍率",
            "紧凑%(svr>=2)", "附肢%", "深度", "存活种", "物种聚", "异养%"
        );
        for &u in &ups {
            for &k in &ks {
                let r = run_seeds(
                    1.0, 0, ticks, &seeds, free, tries, min_cells, cost_mul, upkeep_p, heat,
                    temp_dmg, photo, ambient, comfort, growth, gcost, stall, cloud, rich, mingrow,
                    bud, budper, budfeed, oldbite, bite, shade, oldmouth, day, night, sizeadv, kin,
                    spdist, kinspec, brain, starve, produce, soil, soilr, soilf, rec, rot, flat,
                    cave, cooldown, egg, hatch, spec, k, u,
                );
                println!(
                    "{:>6.2}  {:>6.2}  {:>7.0}  {:>9.2}  {:>9.2}  {:>9}  {:>9.2}  {:>8.2}  {:>9.1}%  {:>6.1}%  {:>5}  {:>7.1}  {:>7.1}  {:>7.1}",
                    k, u, r.pop, r.body_avg, r.body_adult, r.body_max, r.svr, r.prod_mult,
                    r.compact, r.limbs, r.depth_max, r.species, r.species_clustered, r.hetero
                );
            }
        }
        println!();
    }
    if exponly {
        return;
    }

    // ── 第一轮：只动指数 p，不限体型 ──
    println!("【1】繁殖成本指数 p（max_body_cells = 0，不限）");
    println!(
        "{:>5}  {:>7}  {:>9}  {:>9}  {:>7}  {:>8}  {:>7}  {:>6}  {:>7}  {:>9}  {:>6}  {:>7}  {:>6}  {:>6}",
        "p", "个体", "平均体型", "最大体型", "存活种", "有光占用", "异养%", "嘴%", "杀手%",
        "成体体型", "芽%", "物种(聚类)", "移动%", "地力%"
    );
    // 往 1 以下扫是重点：p < 1 让大体型便宜，才有向上的选择压力。
    // 只留 4 个点是为了控功耗——这台机器长时间满载会过热（clock_watchdog_timeout），
    // 要更细的曲线就手动加回来，然后盯着跑。
    // `p=X`（可多次）只跑指定的指数，省时间。默认跑整条曲线。
    let ps: Vec<f64> = {
        let v: Vec<f64> = args
            .iter()
            .filter_map(|a| a.strip_prefix("p=").and_then(|v| v.parse().ok()))
            .collect();
        if v.is_empty() {
            vec![0.5, 0.6, 0.7, 0.8, 1.0]
        } else {
            v
        }
    };
    let mut rows = Vec::new();
    for &p in &ps {
        let r = run_seeds(
            p, 0, ticks, &seeds, free, tries, min_cells, cost_mul, upkeep_p, heat, temp_dmg, photo,
            ambient, comfort, growth, gcost, stall, cloud, rich, mingrow, bud, budper, budfeed,
            oldbite, bite, shade, oldmouth, day, night, sizeadv, kin, spdist, kinspec, brain, starve, produce, soil, soilr, soilf, rec, rot, flat, cave, cooldown, egg, hatch, spec,
            expprod, expup,
        );
        println!(
            "{:>5.2}  {:>7.0}  {:>9.2}  {:>9}  {:>7.1}  {:>7.1}%  {:>7.1}  {:>6.1}  {:>7.1}  {:>9.2}  {:>6.1}%  {:>7.1}  {:>6.1}  {:>6.1}",
            p, r.pop, r.body_avg, r.body_max, r.species, r.occ, r.hetero, r.mouth, r.killer,
            r.body_adult, r.buds, r.species_clustered, r.movers, r.soil
        );
        rows.push((p, r));
    }

    // 繁殖受阻：这是「p 为什么推不动体型」的直接答案。
    //
    // `reproduce_organism` 是**先扣能量再检查空间**的，所以每一次"受阻"都意味着
    // 个体白付了一次全款（`food_needed`）。如果受阻率高，那「降低繁殖门槛（p < 1）」
    // 只会让个体更频繁地撞墙、更亏——规模经济和这个惩罚打架，后者赢。
    println!("\n【1b】繁殖受阻（累计次数 · 括号里是占尝试的百分比）");
    println!(
        "{:>5}  {:>8}  {:>16}  {:>16}  {:>16}  {:>13}  {:>13}",
        "p", "尝试", "成功", "没整块空地", "路径被挡", "到达上限", "体型太小"
    );
    for (p, r) in &rows {
        let pct = |v: u64| {
            if r.tries == 0 {
                0.0
            } else {
                v as f64 * 100.0 / r.tries as f64
            }
        };
        println!(
            "{:>5.2}  {:>8}  {:>8} ({:>5.1}%)  {:>8} ({:>5.1}%)  {:>8} ({:>5.1}%)  {:>7} ({:>4.1}%)  {:>7} ({:>4.1}%)",
            p,
            r.tries,
            r.ok,
            pct(r.ok),
            r.blk_space,
            pct(r.blk_space),
            r.blk_path,
            pct(r.blk_path),
            r.blk_cap,
            pct(r.blk_cap),
            r.blk_small,
            pct(r.blk_small)
        );
    }

    // 光照质量分带：富贫斑块有没有分化出两种体型
    println!("
【1d】按所在格天空光分带（占满照比例 · 个体数 / 平均体型）");
    print!("{:>5} ", "p");
    for b in 0..4 {
        print!("  {:>3}–{:>3}%        ", b * 25, b * 25 + 24);
    }
    println!();
    for (p, r) in &rows {
        print!("{:>5.2} ", p);
        for b in 0..4 {
            let (c, a) = r.light_bands[b];
            if c == 0 {
                print!("{:>5} {:>5}        ", "—", "—");
            } else {
                print!("{:>5} {:>5.2}        ", c, a);
            }
        }
        println!();
    }

    // 温度分带：生态位到底有没有分化的直接读数
    println!("\n【1c】按所在格温度分带（每带 52/255 · 个体数 / 平均体型）");
    print!("{:>5} ", "p");
    for b in 0..5 {
        print!("  {:>3}–{:>3}      ", b * 52, b * 52 + 51);
    }
    println!();
    for (p, r) in &rows {
        print!("{:>5.2} ", p);
        for b in 0..5 {
            let (c, a) = r.bands[b];
            if c == 0 {
                print!("{:>5} {:>5}      ", "—", "—");
            } else {
                print!("{:>5} {:>5.2}      ", c, a);
            }
        }
        println!();
    }

    // 收入来源：这是"到底有没有捕食者"的**行为**读数
    println!("\n【1e】按累计能量收入判定（占个体百分比）");
    println!(
        "{:>5}  {:>7}  {:>7}  {:>7}  {:>7}  {:>7}  {:>9}  {:>11}  {:>9}  {:>9}  {:>11}  {:>7}",
        "p", "光合", "吃食物", "咬活体", "供养", "没收入", "咬>50%", "异养(结构)",
        "其中纯异养", "其中混养", "异养(结构·成体)", "芽%"
    );
    for (p, r) in &rows {
        println!(
            "{:>5.2}  {:>6.1}%  {:>6.1}%  {:>6.1}%  {:>6.1}%  {:>6.1}%  {:>8.1}%  {:>10.1}%  {:>8.1}%  {:>8.1}%  {:>10.1}%  {:>6.1}%",
            p,
            r.income[0],
            r.income[1],
            r.income[2],
            r.income[3],
            r.income[4],
            r.pred_share,
            r.hetero,
            r.pred_pure,
            r.pred_mix,
            r.hetero_adult,
            r.buds
        );
    }

    // ── 批次 38：移动者 vs 非移动者的一阶量 ──
    //
    // 为什么单列这一节：种群是二阶量，3 种子的噪声能到 ±30%（批次 37 实测）。
    // 这一节是「每个个体自己的账」——收入用 `累计收入 / 年龄`，寿命用死者年龄的平均。
    // 几百个个体平均下来，噪声比种群小一个量级，才量得出「策略好坏」。
    println!("\n【1f】移动者 vs 非移动者（一阶量 · 收入 = 累计收入 ÷ 年龄）");
    println!(
        "{:>5}  {:>7}  {:>7}  {:>9}  {:>11}  {:>9}  {:>8}  {:>9}  {:>11}  {:>9}  {:>9}",
        "p", "移动数", "移动%", "移动·年龄", "移动·收入/tick", "移动·寿命", "移动·地力%",
        "静止·年龄", "静止·收入/tick", "静止·寿命", "移动/静止"
    );
    for (p, r) in &rows {
        println!(
            "{:>5.2}  {:>7.0}  {:>6.1}%  {:>9.0}  {:>11.4}  {:>9.0}  {:>8.1}  {:>9.0}  {:>11.4}  {:>9.0}  {:>8.2}x",
            p,
            r.mover_n,
            if r.pop > 0.0 { r.mover_n * 100.0 / r.pop } else { 0.0 },
            r.mover_age,
            r.mover_income,
            r.mover_life,
            r.mover_soil,
            r.still_age,
            r.still_income,
            r.still_life,
            // **这一列才是判据**：移动到底划不划算。
            // 1.0 = 打平；> 1.5 才算"移动真的值"（批次 38 实测 1.12）。
            if r.still_income > 1e-9 {
                r.mover_income / r.still_income
            } else {
                0.0
            }
        );
    }

    // 噪声：把每个种子的个体数摊开看。判断「差异」时先看这个再下结论。
    println!("\n【噪声】每个种子的个体数（这一行比上面任何一行都重要）");
    for (p, r) in &rows {
        if r.per_seed_pop.is_empty() {
            continue;
        }
        let mut v = r.per_seed_pop.clone();
        v.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let lo = *v.first().unwrap();
        let hi = *v.last().unwrap();
        let mean = v.iter().sum::<f64>() / v.len() as f64;
        let spread = if mean > 0.0 { (hi - lo) * 100.0 / mean } else { 0.0 };
        print!("  p={:.2}  ", p);
        for x in &v {
            print!("{:.0}  ", x);
        }
        println!("→ {:.0}~{:.0}（极差 {:.0}%）", lo, hi, spread);
        let mut m = r.per_seed_minc.clone();
        m.sort_by(|a, b| a.partial_cmp(b).unwrap());
        print!("           移动者收入/tick：");
        for x in &m {
            print!("{:.4}  ", x);
        }
        println!();
    }

    // 体型分布：看 p 是不是真的把分布推走
    println!("\n【2】体型分布（占个体百分比）");
    print!("{:>5} ", "p");
    for k in 1..=10 {
        print!(" {:>4}格", k);
    }
    println!();
    for (p, r) in &rows {
        print!("{:>5.2} ", p);
        for v in r.hist.iter() {
            print!(" {:>5.1}", v);
        }
        println!();
    }

    // ── 第二轮：扫体型上限 ──
    //
    // 上限的意义取决于"有没有巨无霸"。p = 1.0 时最大才 5 格，上限给多少都一样；
    // 只有在体型真的能膨胀起来时，上限才是个真问题。
    println!("\n【3】体型上限（p = 1.0）");
    println!(
        "  {:>6}  {:>7}  {:>9}  {:>9}  {:>7}",
        "上限", "个体", "平均体型", "最大体型", "存活种"
    );
    for cap in [0usize, 8, 16] {
        let r = run_seeds(
            1.0, cap, ticks, &seeds, free, tries, min_cells, cost_mul, upkeep_p, heat, temp_dmg, photo,
            ambient, comfort, growth, gcost, stall, cloud, rich, mingrow, bud, budper, budfeed,
            oldbite, bite, shade, oldmouth, day, night, sizeadv, kin, spdist, kinspec, brain, starve, produce, soil, soilr, soilf, rec, rot, flat, cave, cooldown, egg, hatch, spec,
            expprod, expup,
        );
        println!(
            "  {:>6}  {:>7.0}  {:>9.2}  {:>9}  {:>7.1}",
            if cap == 0 {
                "不限".to_string()
            } else {
                cap.to_string()
            },
            r.pop,
            r.body_avg,
            r.body_max,
            r.species
        );
    }

    println!("\n判读要点：");
    println!("- 平均体型随 p 下降而上升 → p < 1 才能造出大体型，这就是想要的杠杆");
    println!("- 个体数骤降 → p 给太大，繁殖成本把种群压死了");
    println!("- 最大体型 ≈ 平均体型 → 没有巨无霸，体型上限没必要设");
    println!("- **异养% 才是判据**：体型涨上去但异养还是 0，说明「猎物变厚 → 捕食可行」这条推论错了");
}
