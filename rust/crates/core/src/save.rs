//! 世界存档 —— 2.0 的全新二进制格式（**不兼容 1.x**，用户已批准断代）。
//!
//! ## 文件布局
//!
//! ```text
//! ┌────────┬─────────┬───────┬───────┬────────┬────────┬──────────┐
//! │ magic  │ version │ cols  │ rows  │  tick  │ orgs   │ species  │  36 字节
//! │ 8 B    │ u32     │ u32   │ u32   │  u64   │ u32    │ u32      │  小端
//! └────────┴─────────┴───────┴───────┴────────┴────────┴──────────┘
//! │ bincode(SaveFile) …                                          │
//! ```
//!
//! 为什么要有这 36 字节的头：世界窗口要列存档（尺寸 / tick / 个体数），
//! 一份存档 4–6 MB，为了显示一行摘要去解整份文件太蠢。头是**定长小端**，
//! 读一次 `seek(0)` + 36 字节就够。
//!
//! ## 两个"顺序即状态"的索引
//!
//! 这是本模块最容易被写错的地方，两条都踩过（1.x 的 phase5-test 抓出来的）：
//!
//! 1. **`food_cells`** —— 食物腐败是 `rng.int(food_cells.len())` 取**数组位置**，
//!    位置变了，同一颗随机数就落到另一格食物上。
//! 2. **`terrain`（腐殖质索引）** —— `apply_terrain_tick` 按 `set.cells` 的
//!    数组顺序遍历，每格消耗 RNG。顺序变了，哪一格先掷骰子就变了。
//!
//! 所以两者都**按原样存数组**，读档直接塞回去，**绝不能调 `init_food_index()`
//! 或 `init_terrain_index()` 重建**（那是按网格索引升序，顺序必然不同）。
//!
//! ## 版本策略
//!
//! 加字段就升 [`VERSION`]，旧档直接拒绝并明确报错——**不做静默兼容**。
//! 开发期存档没有保留价值，断代比写迁移代码便宜得多。

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::fields::Fields;
use crate::grid::Grid;
use crate::light::Light;
use crate::organism::Organism;
use crate::rng::Rng;
use crate::species::SpeciesRegistry;
use crate::world::{TerrainSet, World};

pub const MAGIC: &[u8; 8] = b"EVOSIM2\0";
/// 存档格式版本。**任何字段增删都要 +1**，否则旧档会被解成垃圾数据。
/// v2：`Organism` 加了 `last_move_ok` / `energy_gained_last`（检视器用）。
/// 这两个字段不参与模拟，但 bincode 是定长结构编码，少字段就解不出来，
/// 所以必须升版本——旧档直接报错，不做静默兼容。
/// v3：`LifeCfg` 加了 `breed_cost_exponent` / `max_body_cells`（大体型）。同理。
/// v4：`CellAttrs` 加了 5 个每细胞倍率（`bite_mult` / `prod_mult` / `regen_mult` /
/// `storage_mult` / `armor_resist`），让参数按细胞类型分化。同理。
/// v5：`SaveFile` 加了 `light`（光场，批次 7）。同理。
/// v6：`Light` 加了 `sources`（可放置的点光源）/ `sky`（天空光开关），批次 7.5。同理。
/// v7：`Grid` 加了 `food`（每格食物能量），`WorldCfg` 加了 `food_decay_floor`，
/// `World` 加了 `heat_pool`，批次 8。同理。
/// v8：`Light` 拆成 `sky_level` / `lamp_level` 两层，`LifeCfg` 加了 `shade_strength`，
/// 批次 9（遮挡）。同理。
/// v9：`LifeCfg` 加了 `bite_rate` / `bite_efficiency`，批次 9.5（嘴咬活体）。同理。
/// v10：`CombatCfg` 加了 `food_blocks_movement`，批次 9.6（食物算不算路障）。同理。
/// v11：`Grid.food` 从 `Vec<f32>` 改成 `Vec<f64>`，`SaveFile` 加了三个能量池
/// （`heat_pool` / `energy_wasted` / `energy_injected`），批次 10（维持成本 + 守恒审计）。
/// 三个池子以前**根本没存**——读档之后账目从 0 重新开始，审计会直接红。
/// v12：`WorldCfg` 加了 `sky_patchiness`，`Light` 加了 `patchiness`（批次 10.5 云影）。同理。
/// v13：`LifeCfg` 加了 7 个字段——`repro_fail_costs_energy` / `repro_search_tries` /
/// `min_breed_cells` / `upkeep_exponent` / `temp_damage` / `temp_comfort` /
/// `temp_size_exponent`（批次 11 体型经济学 + 环境温度）。同理。
/// v14：`WorldCfg` 加了 `ambient_temp` / `temp_span`，`LifeCfg::temp_comfort`
/// （0–1 比例）改名并换成 `comfort_temp`（摄氏度）。温度场从"绝对 0–255"
/// 变成"高于环境的温差"。
/// v15：`LifeCfg` 加了 `growth_enabled` / `growth_cost` / `growth_seed_fraction`，
/// `Organism` 加了 `growth_plan`（批次 12 成长过程）。
/// v16：`LifeCfg` 加了 `growth_stall_mature`，`Organism` 加了 `grow_stall`
/// （批次 12.1 环境可塑 / 提前成熟）。
/// v17：`WorldCfg` 加了 `patch_richness`，`Light` 加了 `patch_floor`
/// （批次 13 富贫斑块）。
/// v18：`LifeCfg` 加了 `growth_min_ticks`，`Organism` 加了 `grow_age`
/// （批次 14 最短成长期）。
pub const VERSION: u32 = 18;
/// 定长头：8 + 4 + 4 + 4 + 8 + 4 + 4
pub const HEADER_LEN: usize = 36;
pub const EXT: &str = "evosim";

// ────────────────────────────── 存档内容 ──────────────────────────────

/// 存档的完整内容。
///
/// 这里几乎是把 `World` 直接摊平：**刻意不复用 `World` 本身**，因为
/// `id_index`（可从 organisms 重建）和 `order`（每 tick 复用的缓冲）不该进档。
/// 其余字段直接借运行时的类型，好处是**加字段时不会忘记同步存档**。
#[derive(Serialize, Deserialize)]
pub struct SaveFile {
    pub config: Config,
    pub tick: u64,
    pub next_organism_id: u32,
    pub rng: Rng,
    pub grid: Grid,
    /// 食物索引的原始顺序（见模块文档）
    pub food_cells: Vec<usize>,
    /// 地形索引的原始顺序（见模块文档）。`HashMap` 无序，所以摊成 `Vec`。
    pub terrain: Vec<(u8, TerrainSet)>,
    pub organisms: Vec<Organism>,
    pub species: SpeciesRegistry,
    pub fields: Fields,
    /// 光场。**存下来而不是读档重算**：重算只依赖地形，理论上等价，
    /// 但存档应该能完整还原「用户看到的那一帧」，别在这里省 30KB。
    pub light: Light,
    /// 能量账的三个池子（批次 10）。必须存，否则读档后守恒审计从 0 开始算，
    /// `注入 == 存量` 这条等式立刻不成立。
    pub heat_pool: f64,
    pub energy_wasted: f64,
    pub energy_injected: f64,
}

/// 只读文件头就能拿到的摘要（世界窗口列存档用）
#[derive(Clone, Debug)]
pub struct SaveInfo {
    pub path: PathBuf,
    pub name: String,
    pub cols: u32,
    pub rows: u32,
    pub tick: u64,
    pub organisms: u32,
    pub species: u32,
    pub bytes: u64,
    pub modified: Option<SystemTime>,
}

#[derive(Debug)]
pub enum SaveError {
    BadMagic,
    Truncated,
    Version { found: u32, expected: u32 },
    Corrupt(String),
    Io(io::Error),
}

impl std::fmt::Display for SaveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SaveError::BadMagic => write!(f, "不是 evosim 存档（文件头不匹配）"),
            SaveError::Truncated => write!(f, "存档不完整（文件被截断）"),
            SaveError::Version { found, expected } => write!(
                f,
                "存档版本 {} 与本程序 {} 不兼容——请新建世界",
                found, expected
            ),
            SaveError::Corrupt(e) => write!(f, "存档数据损坏：{}", e),
            SaveError::Io(e) => write!(f, "读写失败：{}", e),
        }
    }
}

impl std::error::Error for SaveError {}

impl From<io::Error> for SaveError {
    fn from(e: io::Error) -> Self {
        SaveError::Io(e)
    }
}

// ────────────────────────────── 编解码 ──────────────────────────────

/// 把世界编成字节（含 36 字节头）
pub fn encode(world: &World) -> Vec<u8> {
    let save = SaveFile {
        config: world.config.clone(),
        tick: world.tick,
        next_organism_id: world.next_organism_id,
        rng: world.rng.clone(),
        grid: world.grid.clone(),
        food_cells: world.food_cells.clone(),
        terrain: world
            .terrain
            .iter()
            .map(|(&k, v)| (k, v.clone()))
            .collect(),
        organisms: world.organisms.clone(),
        species: world.species.clone(),
        fields: world.fields.clone(),
        light: world.light.clone(),
        heat_pool: world.heat_pool,
        energy_wasted: world.energy_wasted,
        energy_injected: world.energy_injected,
    };

    let body = bincode::serialize(&save).expect("存档序列化失败");

    let mut out = Vec::with_capacity(HEADER_LEN + body.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(world.grid.cols as u32).to_le_bytes());
    out.extend_from_slice(&(world.grid.rows as u32).to_le_bytes());
    out.extend_from_slice(&world.tick.to_le_bytes());
    out.extend_from_slice(&(world.organisms.len() as u32).to_le_bytes());
    out.extend_from_slice(&(world.species.size() as u32).to_le_bytes());
    debug_assert_eq!(out.len(), HEADER_LEN);
    out.extend_from_slice(&body);
    out
}

/// 从字节还原世界
pub fn decode(bytes: &[u8]) -> Result<World, SaveError> {
    let head = read_header(bytes)?;
    let save: SaveFile =
        bincode::deserialize(&bytes[HEADER_LEN..]).map_err(|e| SaveError::Corrupt(e.to_string()))?;

    // 头与体必须自洽，否则说明文件被动过 / 版本错位
    if save.grid.cols as u32 != head.cols || save.grid.rows as u32 != head.rows {
        return Err(SaveError::Corrupt(format!(
            "头部尺寸 {}×{} 与数据 {}×{} 不符",
            head.cols, head.rows, save.grid.cols, save.grid.rows
        )));
    }
    if save.grid.len != save.grid.cols * save.grid.rows
        || save.grid.types.len() != save.grid.len
        || save.grid.owners.len() != save.grid.len
        || save.grid.floor.len() != save.grid.len
        || save.grid.food.len() != save.grid.len
    {
        return Err(SaveError::Corrupt("网格三层长度与尺寸不符".into()));
    }
    if save.fields.len != save.grid.len {
        return Err(SaveError::Corrupt("环境场长度与网格不符".into()));
    }

    Ok(into_world(save))
}

struct Header {
    cols: u32,
    rows: u32,
}

fn read_header(bytes: &[u8]) -> Result<Header, SaveError> {
    if bytes.len() < HEADER_LEN {
        return Err(if bytes.len() >= MAGIC.len() && &bytes[..MAGIC.len()] != MAGIC {
            SaveError::BadMagic
        } else {
            SaveError::Truncated
        });
    }
    if &bytes[..MAGIC.len()] != MAGIC {
        return Err(SaveError::BadMagic);
    }
    let u32_at = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap());
    let version = u32_at(8);
    if version != VERSION {
        return Err(SaveError::Version {
            found: version,
            expected: VERSION,
        });
    }
    Ok(Header {
        cols: u32_at(12),
        rows: u32_at(16),
    })
}

fn into_world(save: SaveFile) -> World {
    let SaveFile {
        config,
        tick,
        next_organism_id,
        rng,
        grid,
        food_cells,
        terrain,
        organisms,
        species,
        fields,
        light,
        heat_pool,
        energy_wasted,
        energy_injected,
    } = save;

    // 先按 config 建一个空世界（拿到尺寸正确的索引表），随后整体覆盖
    let mut world = World::new(config, rng);
    world.tick = tick;
    world.next_organism_id = next_organism_id;
    world.grid = grid;

    // ── 食物索引：**原顺序**恢复，不重建 ──
    world.food_pos = vec![-1; world.grid.len];
    for (p, &idx) in food_cells.iter().enumerate() {
        if idx < world.food_pos.len() {
            world.food_pos[idx] = p as i32;
        }
    }
    world.food_cells = food_cells;

    // ── 地形索引：**原顺序**恢复，不重建 ──
    world.terrain = terrain.into_iter().collect();

    world.organisms = organisms;
    world.id_index = world
        .organisms
        .iter()
        .enumerate()
        .map(|(i, o)| (o.id, i))
        .collect();
    world.species = species;
    world.fields = fields;
    world.light = light;
    // 云影的噪声图**不进存档**（30k 个 f32 太重，而且它是纯函数）：
    // 种子永远等于世界 RNG 的种子，读档时重新灌一次、重建时现算。
    world.light.mask_seed = world.rng.seed();
    world.heat_pool = heat_pool;
    world.energy_wasted = energy_wasted;
    world.energy_injected = energy_injected;

    world
}

// ────────────────────────────── 文件 ──────────────────────────────

/// 存档目录 = **程序所在目录** 下的 `saves/`。
///
/// 与 1.x 的"导出落程序目录"保持一致：exe 拷到哪，存档就在哪，不散落在
/// 系统 AppData 里（用户能直接看到、备份、发给别人）。
pub fn saves_dir() -> PathBuf {
    let base = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("saves")
}

/// 清洗文件名：只留字母数字、汉字、`-` `_`，其余一律换成 `_`。
///
/// **必须做**——名字来自输入框，`../` 这种能写到存档目录外面去。
/// 连 `.` 也不放行：文件名不需要它，而放行就等于给 `..` 留了口子。
pub fn sanitize_name(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.trim().chars().take(64) {
        if ch.is_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        "world".to_string()
    } else {
        out
    }
}

/// 保存到 `saves/<name>.evosim`，返回实际路径
pub fn save_to(world: &World, name: &str) -> Result<PathBuf, SaveError> {
    let dir = saves_dir();
    fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.{}", sanitize_name(name), EXT));
    fs::write(&path, encode(world))?;
    Ok(path)
}

/// 从磁盘读档
pub fn load_from(path: &Path) -> Result<World, SaveError> {
    let bytes = fs::read(path)?;
    decode(&bytes)
}

/// 扫描存档目录，按修改时间倒序
pub fn list_saves() -> Vec<SaveInfo> {
    let dir = saves_dir();
    let Ok(rd) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for ent in rd.flatten() {
        let path = ent.path();
        if path.extension().and_then(|e| e.to_str()) != Some(EXT) {
            continue;
        }
        let meta = match ent.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("?")
            .to_string();
        let (cols, rows, tick, organisms, species) = read_summary(&path).unwrap_or((0, 0, 0, 0, 0));
        out.push(SaveInfo {
            path,
            name,
            cols,
            rows,
            tick,
            organisms,
            species,
            bytes: meta.len(),
            modified: meta.modified().ok(),
        });
    }
    out.sort_by(|a, b| b.modified.cmp(&a.modified));
    out
}

/// 只读 36 字节头。返回 `(cols, rows, tick, organisms, species)`
fn read_summary(path: &Path) -> Option<(u32, u32, u64, u32, u32)> {
    use std::io::Read;
    let mut f = fs::File::open(path).ok()?;
    let mut buf = [0u8; HEADER_LEN];
    f.read_exact(&mut buf).ok()?;
    if &buf[..MAGIC.len()] != MAGIC {
        return None;
    }
    let u32_at = |o: usize| u32::from_le_bytes(buf[o..o + 4].try_into().unwrap());
    let u64_at = |o: usize| u64::from_le_bytes(buf[o..o + 8].try_into().unwrap());
    Some((u32_at(12), u32_at(16), u64_at(20), u32_at(28), u32_at(32)))
}

// ────────────────────────────── 测试 ──────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell_type::ct;
    use crate::factory::{seed_origin_of_life, spawn_population};
    use crate::worldgen::generate_demo_world;

    fn demo_world(seed: u32) -> World {
        let mut w = World::new(Config::default(), Rng::new(seed));
        generate_demo_world(&mut w, 1.0, 1.0);
        seed_origin_of_life(&mut w);
        spawn_population(&mut w, 8, 3);
        w
    }

    /// 往返一次后，世界的可观察状态必须一模一样
    #[test]
    fn roundtrip_preserves_state() {
        let mut w = demo_world(4242);
        for _ in 0..200 {
            w.step();
        }
        let bytes = encode(&w);
        let back = decode(&bytes).expect("读档失败");

        assert_eq!(back.tick, w.tick);
        assert_eq!(back.organisms.len(), w.organisms.len());
        assert_eq!(back.grid.types, w.grid.types);
        assert_eq!(back.grid.owners, w.grid.owners);
        assert_eq!(back.grid.floor, w.grid.floor);
        assert_eq!(back.food_cells, w.food_cells, "食物索引顺序必须原样保留");
        assert_eq!(back.species.size(), w.species.size());
        assert_eq!(back.fields.temp, w.fields.temp);
        assert_eq!(back.fields.rad, w.fields.rad);
        // 能量账的三个池子必须一起存下来，否则读档后守恒审计从 0 重新开始
        assert_eq!(back.heat_pool.to_bits(), w.heat_pool.to_bits());
        assert_eq!(back.energy_wasted.to_bits(), w.energy_wasted.to_bits());
        assert_eq!(back.energy_injected.to_bits(), w.energy_injected.to_bits());
        assert_eq!(back.grid.food, w.grid.food, "食物能量必须逐位还原");

        for (a, b) in w.organisms.iter().zip(back.organisms.iter()) {
            assert_eq!(a.id, b.id);
            assert_eq!((a.col, a.row), (b.col, b.row));
            assert_eq!(a.energy.to_bits(), b.energy.to_bits());
            assert_eq!(a.age, b.age);
            assert_eq!(a.brain.w1, b.brain.w1);
            assert_eq!(a.anatomy.cells, b.anatomy.cells);
        }
    }

    /// **最关键的一条**：读档后继续演化必须与不读档完全一致。
    ///
    /// 1.x 就是靠这条抓出了"食物索引重建"和"isPassable 把食物当可通行"两个 bug。
    /// 只要有一处顺序 / 状态没存对，几十 tick 内就会分叉。
    #[test]
    fn evolution_continues_identically_after_load() {
        let mut a = demo_world(777);
        for _ in 0..300 {
            a.step();
        }

        let mut b = decode(&encode(&a)).expect("读档失败");

        for i in 0..200 {
            a.step();
            b.step();
            assert_eq!(
                a.state_digest(),
                b.state_digest(),
                "第 {} tick 分叉（存档后 +{}）",
                a.tick,
                i + 1
            );
        }
    }

    /// 尺寸不同的世界也要能往返（新建世界之后立刻存档的场景）
    #[test]
    fn roundtrip_odd_size() {
        let mut cfg = Config::default();
        cfg.world.cols = 64;
        cfg.world.rows = 48;
        let mut w = World::new(cfg, Rng::new(9));
        generate_demo_world(&mut w, 1.0, 1.0);
        seed_origin_of_life(&mut w);
        for _ in 0..50 {
            w.step();
        }

        let back = decode(&encode(&w)).expect("读档失败");
        assert_eq!((back.grid.cols, back.grid.rows), (64, 48));
        assert_eq!(back.grid.types, w.grid.types);
        assert_eq!(back.fields.len, 64 * 48);
    }

    #[test]
    fn rejects_garbage_and_wrong_version() {
        assert!(matches!(decode(b"nonsense"), Err(SaveError::BadMagic)));
        let mut bytes = encode(&demo_world(1));
        bytes[8] = 99; // version 字节
        assert!(matches!(decode(&bytes), Err(SaveError::Version { .. })));
    }

    #[test]
    fn sanitize_strips_path_traversal() {
        assert_eq!(sanitize_name("../../etc/passwd"), "______etc_passwd");
        assert_eq!(sanitize_name("我的世界 01"), "我的世界_01");
        assert_eq!(sanitize_name("   "), "world");
        assert!(!sanitize_name("a/b\\c:d").contains(['/', '\\', ':']));
    }

    /// 存档里必须真的有地形与食物，否则上面那些"顺序"断言就是在测空气
    #[test]
    fn demo_world_has_content_to_save() {
        let mut w = demo_world(3);
        for _ in 0..100 {
            w.step();
        }
        assert!(w.organisms.len() > 3, "应该已经繁殖出一些个体");
        assert!(w.grid.types.iter().any(|&t| t != ct::EMPTY), "网格不该全空");
    }
}
