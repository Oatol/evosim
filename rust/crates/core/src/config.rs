//! 全局参数表 —— 与 JS 版 `src/core/config.js` 的默认值逐项对应。

use serde::{Deserialize, Serialize};

use crate::cell_type::{self, ct};

/// 每种身体细胞的可调属性
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct CellAttrs {
    pub hp: f64,
    pub cost: f64,
    pub damage: f64,
    pub sense_range: f64,
    pub food_value: f64,
}

impl Default for CellAttrs {
    fn default() -> Self {
        CellAttrs {
            hp: 1.0,
            cost: 0.0,
            damage: 0.0,
            sense_range: 0.0,
            food_value: 1.0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorldCfg {
    pub cols: usize,
    pub rows: usize,
    pub wrap_edges: bool,
    pub food_drop_prob: f64,
    pub food_decay_rate: f64,
    pub clear_walls_on_reset: bool,
    pub auto_reset: bool,
}

impl Default for WorldCfg {
    fn default() -> Self {
        WorldCfg {
            cols: 200,
            rows: 150,
            wrap_edges: true,
            food_drop_prob: 0.0,
            food_decay_rate: 2.0,
            clear_walls_on_reset: false,
            auto_reset: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LifeCfg {
    pub lifespan_multiplier: f64,
    pub extra_mover_food_cost: f64,
    pub max_organisms: i64,
    pub food_prod_prob: f64,
    pub movers_can_produce: bool,
    pub turn_cooldown: i32,
    pub metabolism_scale: f64,
    pub food_energy_scale: f64,
    pub starvation_ticks: i32,
    pub storage_food_bonus: f64,
    pub regen_rate: f64,
}

impl Default for LifeCfg {
    fn default() -> Self {
        LifeCfg {
            lifespan_multiplier: 100.0,
            extra_mover_food_cost: 0.0,
            max_organisms: -1,
            food_prod_prob: 5.0,
            movers_can_produce: false,
            turn_cooldown: 4,
            metabolism_scale: 0.0,
            food_energy_scale: 1.0,
            starvation_ticks: 60,
            storage_food_bonus: 1.0,
            regen_rate: 0.1,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TerrainCfg {
    pub lava_enter_damage: f64,
    pub lava_stand_damage: f64,
    pub swamp_move_cost: f64,
    pub rad_damage: f64,
    pub rad_mutation_boost: f64,
    pub humus_food_prob: f64,
}

impl Default for TerrainCfg {
    fn default() -> Self {
        TerrainCfg {
            lava_enter_damage: 2.0,
            lava_stand_damage: 1.0,
            swamp_move_cost: 0.5,
            rad_damage: 0.25,
            rad_mutation_boost: 3.0,
            humus_food_prob: 0.5,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MutationCfg {
    pub add_prob: f64,
    pub change_prob: f64,
    pub remove_prob: f64,
    pub weight_mut_rate: f64,
    pub weight_mut_sigma: f64,
    pub weight_reset_rate: f64,
    pub use_global_mutability: bool,
    pub global_mutability: f64,
    pub initial_mutability: f64,
    pub mutability_sigma: f64,
}

impl Default for MutationCfg {
    fn default() -> Self {
        MutationCfg {
            add_prob: 25.0,
            change_prob: 25.0,
            remove_prob: 25.0,
            weight_mut_rate: 0.02,
            weight_mut_sigma: 0.20,
            weight_reset_rate: 0.01,
            use_global_mutability: false,
            global_mutability: 5.0,
            initial_mutability: 5.0,
            mutability_sigma: 0.6,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SenseCfg {
    pub look_range: i32,
    pub see_through_self: bool,
}

impl Default for SenseCfg {
    fn default() -> Self {
        SenseCfg {
            look_range: 20,
            see_through_self: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CombatCfg {
    pub insta_kill: bool,
    pub food_blocks_reproduction: bool,
}

impl Default for CombatCfg {
    fn default() -> Self {
        CombatCfg {
            insta_kill: false,
            food_blocks_reproduction: true,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StatsCfg {
    pub interval: u64,
    pub record_size: usize,
    pub min_discard: u32,
}

impl Default for StatsCfg {
    fn default() -> Self {
        StatsCfg {
            interval: 100,
            record_size: 2000,
            min_discard: 10,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BrainCfg {
    pub in_size: usize,
    pub hidden_size: usize,
    pub out_size: usize,
}

impl Default for BrainCfg {
    fn default() -> Self {
        BrainCfg {
            in_size: 40,
            hidden_size: 12,
            out_size: 5,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SimCfg {
    pub base_ticks_per_second: u32,
    pub max_steps_per_frame: u32,
    pub seed: u32,
}

impl Default for SimCfg {
    fn default() -> Self {
        SimCfg {
            base_ticks_per_second: 60,
            max_steps_per_frame: 400,
            seed: 20_261_015,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Config {
    pub world: WorldCfg,
    pub life: LifeCfg,
    pub terrain: TerrainCfg,
    pub mutation: MutationCfg,
    pub sense: SenseCfg,
    pub combat: CombatCfg,
    pub stats: StatsCfg,
    pub brain: BrainCfg,
    pub sim: SimCfg,
    /// 按类型 id 索引的细胞属性（JS 版是 `cells[KEY][attr]`，这里直接用 id 查，更快）
    pub cells: [CellAttrs; 16],
}

impl Default for Config {
    fn default() -> Self {
        let mut cells = [CellAttrs::default(); 16];
        for d in cell_type::CELL_DEFS.iter() {
            cells[d.id as usize] = CellAttrs {
                hp: d.hp,
                cost: d.cost,
                damage: d.damage,
                sense_range: d.sense_range,
                food_value: d.food_value,
            };
        }
        Config {
            world: WorldCfg::default(),
            life: LifeCfg::default(),
            terrain: TerrainCfg::default(),
            mutation: MutationCfg::default(),
            sense: SenseCfg::default(),
            combat: CombatCfg::default(),
            stats: StatsCfg::default(),
            brain: BrainCfg::default(),
            sim: SimCfg::default(),
            cells,
        }
    }
}

impl Config {
    #[inline]
    pub fn attr(&self, type_id: u8, f: AttrField) -> f64 {
        let a = &self.cells[(type_id as usize).min(15)];
        match f {
            AttrField::Hp => a.hp,
            AttrField::Cost => a.cost,
            AttrField::Damage => a.damage,
            AttrField::SenseRange => a.sense_range,
            AttrField::FoodValue => a.food_value,
        }
    }

    /// 一份食物能提供多少能量（兜底成 1，与 JS 版一致）
    #[inline]
    pub fn food_energy_gain(&self) -> f64 {
        let v = self.life.food_energy_scale;
        if v.is_finite() && v > 0.0 {
            v
        } else {
            1.0
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttrField {
    Hp,
    Cost,
    Damage,
    SenseRange,
    FoodValue,
}

/// 参数元数据（面板渲染用）。第一阶段只用得上少数几项。
pub struct FieldMeta {
    pub key: &'static str,
    pub label: &'static str,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub note: &'static str,
}

pub const KEY_FIELDS: [FieldMeta; 6] = [
    FieldMeta {
        key: "metabolism_scale",
        label: "全局消耗量倍率",
        min: 0.0,
        max: 5.0,
        step: 0.05,
        note: "0 = 关闭。调高会带来真实的选择压力",
    },
    FieldMeta {
        key: "food_energy_scale",
        label: "食物量倍率",
        min: 0.1,
        max: 10.0,
        step: 0.1,
        note: "一份食物能提供多少能量",
    },
    FieldMeta {
        key: "food_prod_prob",
        label: "生产者造食率",
        min: 0.0,
        max: 100.0,
        step: 1.0,
        note: "生产者细胞每 tick 生成食物的概率（%）",
    },
    FieldMeta {
        key: "global_mutability",
        label: "全局变异率",
        min: 1.0,
        max: 100.0,
        step: 1.0,
        note: "需勾选「启用全局变异率」才生效",
    },
    FieldMeta {
        key: "max_organisms",
        label: "种群上限",
        min: -1.0,
        max: 5000.0,
        step: 50.0,
        note: "-1 = 无上限",
    },
    FieldMeta {
        key: "lifespan_multiplier",
        label: "寿命倍率",
        min: 1.0,
        max: 1000.0,
        step: 1.0,
        note: "寿命 = 细胞数 × 倍率",
    },
];

impl Config {
    /// 按 key 读关键参数（第一阶段只暴露这几个）
    pub fn get_key(&self, key: &str) -> f64 {
        match key {
            "metabolism_scale" => self.life.metabolism_scale,
            "food_energy_scale" => self.life.food_energy_scale,
            "food_prod_prob" => self.life.food_prod_prob,
            "global_mutability" => self.mutation.global_mutability,
            "max_organisms" => self.life.max_organisms as f64,
            "lifespan_multiplier" => self.life.lifespan_multiplier,
            _ => 0.0,
        }
    }

    pub fn set_key(&mut self, key: &str, v: f64) {
        match key {
            "metabolism_scale" => self.life.metabolism_scale = v,
            "food_energy_scale" => self.life.food_energy_scale = v,
            "food_prod_prob" => self.life.food_prod_prob = v,
            "global_mutability" => self.mutation.global_mutability = v,
            "max_organisms" => self.life.max_organisms = v as i64,
            "lifespan_multiplier" => self.life.lifespan_multiplier = v,
            _ => {}
        }
    }
}

/// 默认世界尺寸常量（避免在别处写魔法数）
pub const DEFAULT_COLS: usize = 200;
pub const DEFAULT_ROWS: usize = 150;
pub const _UNUSED: u8 = ct::EMPTY;
