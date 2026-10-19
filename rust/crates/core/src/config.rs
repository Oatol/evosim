//! 全局参数表 —— 与 JS 版 `src/core/config.js` 的默认值逐项对应。

use serde::{Deserialize, Serialize};

use crate::cell_type::{self, ct};

/// 每种身体细胞的可调属性
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CellAttrs {
    // ── 通用：所有身体细胞都参与结算（按格累加）──
    pub hp: f64,
    pub cost: f64,
    pub food_value: f64,
    // ── 专属：只有对应类型会读 ──
    /// `KILLER` 每次攻击的伤害 / `SPIKE` 反弹的伤害
    pub damage: f64,
    /// `EYE` 的视野距离（0 = 用全局 `sense.look_range`）
    pub sense_range: f64,
    // ── 倍率：乘在**全局基值**上，默认 1.0 = 完全等价旧行为 ──
    //
    // 为什么用倍率而不是绝对值：这几项原本是全局参数（`life.photosynthesis_rate` 等），
    // 直接下放成每细胞的绝对值就得给 16 种细胞都填一个"合理"的数，
    // 而且默认值一变黄金快照立刻漂移。倍率写法让默认配置**逐位不变**，
    // 只有用户主动拖滑块才偏离 1.x 基线。
    /// 嘴：一份食物换多少能量的倍率
    pub bite_mult: f64,
    /// 生产者：造食概率的倍率
    pub prod_mult: f64,
    /// 再生：每 tick 抹掉伤害的倍率
    pub regen_mult: f64,
    /// 储食：繁殖阈值抬高量的倍率
    pub storage_mult: f64,
    /// 装甲：伤害减免比例（0..1）。**1.0 = 完全免疫**，也就是旧行为。
    pub armor_resist: f64,
}

impl Default for CellAttrs {
    fn default() -> Self {
        CellAttrs {
            hp: 1.0,
            cost: 0.0,
            damage: 0.0,
            sense_range: 0.0,
            food_value: 1.0,
            bite_mult: 1.0,
            prod_mult: 1.0,
            regen_mult: 1.0,
            storage_mult: 1.0,
            armor_resist: 1.0,
        }
    }
}

impl CellAttrs {
    /// 按属性键取值。UI 的滑块和 `Config::attr` 都走这里，
    /// 免得"面板改的是这个字段、模拟读的是那个字段"这种错位。
    #[inline]
    pub fn get(&self, f: AttrField) -> f64 {
        match f {
            AttrField::Hp => self.hp,
            AttrField::Cost => self.cost,
            AttrField::Damage => self.damage,
            AttrField::SenseRange => self.sense_range,
            AttrField::FoodValue => self.food_value,
            AttrField::BiteMult => self.bite_mult,
            AttrField::ProdMult => self.prod_mult,
            AttrField::RegenMult => self.regen_mult,
            AttrField::StorageMult => self.storage_mult,
            AttrField::ArmorResist => self.armor_resist,
        }
    }

    #[inline]
    pub fn set(&mut self, f: AttrField, v: f64) {
        match f {
            AttrField::Hp => self.hp = v,
            AttrField::Cost => self.cost = v,
            AttrField::Damage => self.damage = v,
            AttrField::SenseRange => self.sense_range = v,
            AttrField::FoodValue => self.food_value = v,
            AttrField::BiteMult => self.bite_mult = v,
            AttrField::ProdMult => self.prod_mult = v,
            AttrField::RegenMult => self.regen_mult = v,
            AttrField::StorageMult => self.storage_mult = v,
            AttrField::ArmorResist => self.armor_resist = v,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorldCfg {
    pub cols: usize,
    pub rows: usize,
    pub wrap_edges: bool,
    pub food_drop_prob: f64,
    /// 食物每 tick 的相对腐败速率（百分比，默认 1.0）。
    ///
    /// **大块尸体靠它慢慢烂**：`e -= max(e × rate/100, floor)`。
    /// 旧的 2.0 是"每 tick 有 2% 概率整格消失"，断崖式，看不出过程。
    pub food_decay_rate: f64,
    /// 食物每 tick 的绝对衰减下限（能量单位，默认 0.005）。
    ///
    /// 没有它，小块食物的指数衰减尾巴会拖得极长——0.001 点能量赖在格子上不走，
    /// 既占地方又让"食物格数"这个指标失真。
    ///
    /// 出厂值从 0.02 降到 0.005 是**为了让「肥尸体」活得久一点**：
    /// 一块 1.0 能量的肉，0.02 时每 tick 掉 0.02（50 tick 就没了），
    /// 0.005 时前 100 tick 由相对项主导、之后才落到绝对项，寿命翻了一倍多。
    /// 守尸 / 抢食这类依赖「尸体是一块能吃很久的资源」的行为需要这个时间尺度。
    pub food_decay_floor: f64,
    pub clear_walls_on_reset: bool,
    pub auto_reset: bool,
    /// 天空光强度（0 = 永夜，1 = 满照）。
    ///
    /// 天空光就是「世界顶部的一排光源」。**拨到 0 之后世界里只剩玩家放的点光源**
    /// ——这是「光从哪来」完全交给玩家的开关，也是让世界不再被强行分成上下的关键。
    pub sun_intensity: f64,
}

impl Default for WorldCfg {
    fn default() -> Self {
        WorldCfg {
            cols: 200,
            rows: 150,
            wrap_edges: true,
            food_drop_prob: 0.0,
            food_decay_rate: 1.0,
            food_decay_floor: 0.005,
            clear_walls_on_reset: false,
            auto_reset: false,
            sun_intensity: 1.0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LifeCfg {
    pub lifespan_multiplier: f64,
    pub extra_mover_food_cost: f64,
    pub max_organisms: i64,
    /// 光合速率：满照（光强 255）下一个生产者细胞每 tick 产出多少能量。
    ///
    /// 批次 7 之前这里是 `food_prod_prob`（凭空在相邻空格造食物的概率），
    /// 那是台永动机——造食物不消耗任何东西。现在光成了唯一的能量来源，
    /// 生产者是**转换器**：产出正比于它所在格的光强，暗处产出为 0。
    pub photosynthesis_rate: f64,
    /// 遮挡强度 k（0 = 关闭，默认 0.25）。
    ///
    /// 生产者正上方同列 8 行内，每有一个身体格（**包括自己的身体**），天空光乘 `1-k`。
    /// 光不知道格子归谁，所以数全部——这也让"生产者该长在最上面"第一次有了价格。
    ///
    /// 这是给「光合生物把有光区挤成一道墙」定价的那一项：墙内部的细胞照不到光，
    /// 却照样付 upkeep，于是墙从内部塌掉。实测（10000 tick、200×150）：
    /// 有光区占用 27% → 21%，**致密格 9.3% → 1.3%**——被削掉的是"墙"，不是"人口"。
    pub shade_strength: f64,
    /// 每个细胞提供的能量容量。光合直充体内之后必须有上限，否则无限膨胀。
    pub energy_per_cell: f64,
    /// 每个储食细胞额外提供的能量容量（电池）。
    ///
    /// 储食细胞因此同时是收益和风险：装得多 = 熬得过饥荒 / 黑夜，
    /// 但被杀死时对方从尸体上吃到的也更多。
    pub storage_capacity: f64,
    pub movers_can_produce: bool,
    pub turn_cooldown: i32,
    pub metabolism_scale: f64,
    pub food_energy_scale: f64,
    pub starvation_ticks: i32,
    pub storage_food_bonus: f64,
    pub regen_rate: f64,
    /// 繁殖成本的体型指数 p：`food_needed = cell_count^p`。
    ///
    /// 1.0 = 线性，也就是旧行为（黄金快照用的就是它）。p > 1 让大体型**超线性**
    /// 地贵，体型就不再是"收益和成本同倍增长"的中性漂移，而会收敛到一个最优值。
    ///
    /// 默认必须是 1.0：`tests/golden.rs` 拿 `Config::default()` 跟 JS 基线逐位比对，
    /// 改这个数就等于有意让 2.0 和 1.x 分叉。
    pub breed_cost_exponent: f64,
    /// 体型上限（细胞数）。0 = 不限，旧行为。
    ///
    /// 变异里的"加细胞"算子在达到上限后直接放弃。这个上限只挡变异，
    /// 不影响读档/工坊里手工造的超大个体——那是用户明确画出来的东西。
    pub max_body_cells: usize,
}

impl Default for LifeCfg {
    fn default() -> Self {
        LifeCfg {
            lifespan_multiplier: 100.0,
            extra_mover_food_cost: 0.0,
            max_organisms: -1,
            photosynthesis_rate: 0.12,
            shade_strength: 0.25,
            energy_per_cell: 2.0,
            storage_capacity: 6.0,
            movers_can_produce: false,
            turn_cooldown: 4,
            metabolism_scale: 0.0,
            food_energy_scale: 1.0,
            starvation_ticks: 60,
            storage_food_bonus: 1.0,
            regen_rate: 0.1,
            breed_cost_exponent: 1.0,
            max_body_cells: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
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

impl TerrainCfg {
    /// 按参数键取值。和 `CellAttrs::get` 对称，面板不直接摸字段。
    #[inline]
    pub fn get(&self, f: TerrainField) -> f64 {
        match f {
            TerrainField::LavaEnter => self.lava_enter_damage,
            TerrainField::LavaStand => self.lava_stand_damage,
            TerrainField::SwampCost => self.swamp_move_cost,
            TerrainField::RadDamage => self.rad_damage,
            TerrainField::RadMutation => self.rad_mutation_boost,
            TerrainField::HumusFood => self.humus_food_prob,
        }
    }

    #[inline]
    pub fn set(&mut self, f: TerrainField, v: f64) {
        match f {
            TerrainField::LavaEnter => self.lava_enter_damage = v,
            TerrainField::LavaStand => self.lava_stand_damage = v,
            TerrainField::SwampCost => self.swamp_move_cost = v,
            TerrainField::RadDamage => self.rad_damage = v,
            TerrainField::RadMutation => self.rad_mutation_boost = v,
            TerrainField::HumusFood => self.humus_food_prob = v,
        }
    }
}

/// 每种地形的可调参数键 —— `AttrField` 的地形版。
///
/// `AttrField` 指向 `Config.cells[id]` 的一个字段，这里指向 `Config.terrain` 的一个字段。
/// 两者都通过 `ParamRef` 进 `CELL_DEFS[i].params`，所以「类型参数」面板只有一套渲染代码。
///
/// **加一个地形参数 = 给 `TerrainCfg` 加字段 + 这里加变体 + 在对应地形的 `params` 里声明。**
/// 漏掉最后一步就是个死旋钮（面板上看不见，或者看见了拖不动），
/// `every_terrain_field_is_declared` 测试会先炸。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TerrainField {
    LavaEnter,
    LavaStand,
    SwampCost,
    RadDamage,
    RadMutation,
    HumusFood,
}

impl TerrainField {
    /// 全部变体。测试拿它做"有没有漏声明"的守门。
    pub const ALL: [TerrainField; 6] = [
        TerrainField::LavaEnter,
        TerrainField::LavaStand,
        TerrainField::SwampCost,
        TerrainField::RadDamage,
        TerrainField::RadMutation,
        TerrainField::HumusFood,
    ];

    pub fn label(self) -> &'static str {
        match self {
            TerrainField::LavaEnter => "踏入伤害",
            TerrainField::LavaStand => "停留伤害",
            TerrainField::SwampCost => "移动能耗",
            TerrainField::RadDamage => "每 tick 伤害",
            TerrainField::RadMutation => "变异倍率",
            TerrainField::HumusFood => "冒食概率",
        }
    }

    /// `(lo, hi, step, decimals)`，和 `AttrField::range` 同构。
    ///
    /// 量程要贴住出厂值：出厂 0.25 配 0~100 的话拖柄永远趴在左端，
    /// 看不出当前值也调不动。
    pub fn range(self) -> (f64, f64, f64, usize) {
        match self {
            TerrainField::LavaEnter => (0.0, 10.0, 0.5, 1),
            TerrainField::LavaStand => (0.0, 10.0, 0.5, 1),
            TerrainField::SwampCost => (0.0, 5.0, 0.1, 1),
            TerrainField::RadDamage => (0.0, 5.0, 0.05, 2),
            // 1 是下限：0 倍变异率等于关掉辐射带的另一半功能，没意义
            TerrainField::RadMutation => (1.0, 20.0, 0.5, 1),
            TerrainField::HumusFood => (0.0, 5.0, 0.1, 1),
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            TerrainField::LavaEnter => "踩进岩浆那一下的冲击伤害（默认 2）。",
            TerrainField::LavaStand => "待在岩浆里每 tick 的持续伤害（默认 1）。",
            TerrainField::SwampCost => {
                "在沼泽里走一步额外消耗的能量（默认 0.5）。能量不够会落到 0 并进入饥饿计时。"
            }
            TerrainField::RadDamage => "待在辐射带里每 tick 扣的血（默认 0.25）。",
            TerrainField::RadMutation => "站在辐射里的个体，**后代**的变异率乘这个倍数（默认 3）。",
            TerrainField::HumusFood => {
                "每 tick 往相邻空格（含自己那格）长食物的概率，单位 %（默认 0.5）。"
            }
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
            // 前 5 项从 CELL_DEFS 取（出厂值唯一真源），后面的倍率一律走默认 1.0
            // —— 以后再加字段也不用回来改这里。
            cells[d.id as usize] = CellAttrs {
                hp: d.hp,
                cost: d.cost,
                damage: d.damage,
                sense_range: d.sense_range,
                food_value: d.food_value,
                ..CellAttrs::default()
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
        self.cells[(type_id as usize).min(15)].get(f)
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

/// 单个细胞属性的取用键。
///
/// 前 5 个是 1.x 就有的；后 5 个是 2.0 新增的**倍率**（默认 1.0，乘在全局基值上）。
/// `CELL_DEFS[i].params` 声明每种细胞暴露哪几个——不是每种细胞都该有全部 10 项：
/// `damage` 只对杀手/刺有意义，`sense_range` 只对眼睛有意义，其余的改了没反应。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttrField {
    Hp,
    Cost,
    Damage,
    SenseRange,
    FoodValue,
    BiteMult,
    ProdMult,
    RegenMult,
    StorageMult,
    ArmorResist,
}

impl AttrField {
    /// 全部变体。测试拿它做"get/set 有没有漏分支"的守门。
    pub const ALL: [AttrField; 10] = [
        AttrField::Hp,
        AttrField::Cost,
        AttrField::Damage,
        AttrField::SenseRange,
        AttrField::FoodValue,
        AttrField::BiteMult,
        AttrField::ProdMult,
        AttrField::RegenMult,
        AttrField::StorageMult,
        AttrField::ArmorResist,
    ];

    /// 面板上的名字
    pub fn label(self) -> &'static str {
        match self {
            AttrField::Hp => "血量",
            AttrField::Cost => "消耗",
            AttrField::Damage => "伤害",
            AttrField::SenseRange => "感知距离",
            AttrField::FoodValue => "食物值",
            AttrField::BiteMult => "摄取倍率",
            AttrField::ProdMult => "生产速度",
            AttrField::RegenMult => "再生速率",
            AttrField::StorageMult => "储备倍率",
            AttrField::ArmorResist => "伤害减免",
        }
    }

    /// 滑块的量程与精度：`(lo, hi, step, decimals)`
    ///
    /// 每条都要贴住真实取值域——量程给太宽，拖柄会永远趴在最左端看不出当前值
    /// （细胞参数面板第一版就栽在"血量 1.0 配 0~20 量程"上）。
    pub fn range(self) -> (f64, f64, f64, usize) {
        match self {
            // 量程要贴住真实取值域：出厂值大致落在量程 10%~40% 处。
            // 血量出厂 1.0 配 0~20 的话拖柄永远趴在 5%，既看不出当前值也调不动
            // ——细胞参数面板第一版就栽在这上面。
            AttrField::Hp => (0.0, 5.0, 0.1, 1),
            // 消耗出厂 0.002~0.008，步长必须比它细，否则会被吸附成 0
            AttrField::Cost => (0.0, 0.05, 0.001, 3),
            AttrField::Damage => (0.0, 5.0, 0.1, 1),
            AttrField::SenseRange => (0.0, 10.0, 1.0, 0),
            AttrField::FoodValue => (0.0, 10.0, 0.1, 1),
            // 倍率统一 0~5：1.0 是出厂值，落在量程 20% 处
            AttrField::BiteMult => (0.0, 5.0, 0.05, 2),
            AttrField::ProdMult => (0.0, 5.0, 0.05, 2),
            AttrField::RegenMult => (0.0, 5.0, 0.05, 2),
            AttrField::StorageMult => (0.0, 5.0, 0.05, 2),
            // 减免是比例，满量程就是"完全免疫"
            AttrField::ArmorResist => (0.0, 1.0, 0.01, 2),
        }
    }

    /// 面板上的说明
    pub fn hint(self) -> &'static str {
        match self {
            AttrField::Hp => "被打掉多少伤害这个细胞才会死。0 = 一碰就碎。",
            AttrField::Cost => "每 tick 的能量开销（会再乘上全局消耗量倍率）。",
            AttrField::Damage => "攻击型细胞每次命中的伤害。",
            AttrField::SenseRange => "眼睛能看多远。0 = 用全局视野距离。",
            AttrField::FoodValue => "被吃掉时提供多少能量（会再乘上食物量倍率）。",
            AttrField::BiteMult => "一份食物换多少能量的倍率（× 全局「食物量倍率」）。",
            AttrField::ProdMult => "光合产出的倍率（× 全局「光合速率」）。",
            AttrField::RegenMult => "每 tick 抹掉累积伤害的倍率（× 全局「再生速率」）。",
            AttrField::StorageMult => "繁殖阈值抬高量的倍率（× 全局「储食加成」）。",
            AttrField::ArmorResist => "受到攻击时的伤害减免比例。1.00 = 完全免疫（出厂值）。",
        }
    }
}

/// 参数面板里一项可调参数的**来源**。
///
/// 这是「类型参数」窗口能用一套代码画完细胞和地形的原因：面板只认
/// `(当前选中的类型, ParamRef)`，值到底从 `Config.cells[id]` 还是 `Config.terrain` 取，
/// 由变体自己决定。
///
/// 为什么不让 `CellAttrs` 直接带上地形字段：那等于给 16 种细胞各存一份没用的地形参数，
/// 而且 `Config.terrain` 是模拟里真正被读的那份，两份真源必然对不上。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParamRef {
    Cell(AttrField),
    Terrain(TerrainField),
}

impl ParamRef {
    /// `const` 构造器：`CELL_DEFS` 是 `static`，里面的 `params` 必须是常量表达式，
    /// 不能用 `From`/`Into`（那是运行时 trait）。
    pub const fn cell(f: AttrField) -> Self {
        ParamRef::Cell(f)
    }

    pub const fn terrain(f: TerrainField) -> Self {
        ParamRef::Terrain(f)
    }

    pub fn label(self) -> &'static str {
        match self {
            ParamRef::Cell(f) => f.label(),
            ParamRef::Terrain(f) => f.label(),
        }
    }

    pub fn range(self) -> (f64, f64, f64, usize) {
        match self {
            ParamRef::Cell(f) => f.range(),
            ParamRef::Terrain(f) => f.range(),
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            ParamRef::Cell(f) => f.hint(),
            ParamRef::Terrain(f) => f.hint(),
        }
    }

    /// 这一项是不是"每种细胞一份"的属性（而不是"每种地形一份"）。
    #[inline]
    pub fn is_cell(self) -> bool {
        matches!(self, ParamRef::Cell(_))
    }
}

/// 默认世界尺寸常量（避免在别处写魔法数）
pub const DEFAULT_COLS: usize = 200;
pub const DEFAULT_ROWS: usize = 150;
pub const _UNUSED: u8 = ct::EMPTY;
