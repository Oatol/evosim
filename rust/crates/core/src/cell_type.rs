//! 细胞类型注册表 —— 与 JS 版 `src/life/CellType.js` 一一对应。
//!
//! 加一种新细胞 = 往 `CELL_DEFS` 里加一项（需要新行为再补一个 match 分支）。
//! 网格读写、配色、感知编码、结构变异、存档、参数面板都从这里读。

use serde::{Deserialize, Serialize};

use crate::config::{AttrField, ParamRef, TerrainField};

/// 细胞类型 id。**写进存档，定下就不能改。**
pub mod ct {
    pub const EMPTY: u8 = 0;
    pub const FOOD: u8 = 1;
    pub const WALL: u8 = 2;

    pub const MOUTH: u8 = 3;
    pub const PRODUCER: u8 = 4;
    pub const MOVER: u8 = 5;
    pub const KILLER: u8 = 6;
    pub const ARMOR: u8 = 7;
    pub const EYE: u8 = 8;

    pub const LAVA: u8 = 9;
    pub const SWAMP: u8 = 10;
    pub const RAD: u8 = 11;
    pub const HUMUS: u8 = 12;

    pub const STORAGE: u8 = 13;
    pub const REGEN: u8 = 14;
    pub const SPIKE: u8 = 15;
    /// 卵（批次 27）：母体产下的、**占一格**的中间态。孵化成 1 格芽，
    /// 在此之前可以被嘴吃掉、也可以挡路。
    pub const EGG: u8 = 16;

    /// 水（批次 43）：可穿过，但**每一步都更贵**。地形版的"阻力"。
    pub const WATER: u8 = 17;
    /// 冰（批次 43）：可穿过，进入后**顺着原方向再滑一格**。
    /// 唯一一种影响"位置"而不是"收支"的地形。
    pub const ICE: u8 = 18;
}

/// 能力标志位（Anatomy 用它做派生缓存）
pub mod flag {
    pub const MOUTH: u16 = 1 << 0;
    pub const PRODUCER: u16 = 1 << 1;
    pub const MOVER: u16 = 1 << 2;
    pub const KILLER: u16 = 1 << 3;
    pub const ARMOR: u16 = 1 << 4;
    pub const EYE: u16 = 1 << 5;
    pub const STORAGE: u16 = 1 << 6;
    pub const REGEN: u16 = 1 << 7;
    pub const SPIKE: u16 = 1 << 8;
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Behavior {
    Mouth,
    Producer,
    Killer,
    Eye,
    Regen,
}

/// **细胞器向量**（阶段 3）：一格身体"有多像某种器官"，0..=255 连续。
///
/// 为什么需要它：`type_id` 是**封闭枚举**，一格细胞要么是生产者、要么是嘴。
/// 现实里没有这种二值性——光合能力和摄食能力是可以同时存在的连续量。
/// 批次 30 量到 99.7% 的成体都是"生产者 + 嘴"的混养体，但那是在**身体层面**
/// （不同的格子分工）；真正缺的是**同一格**同时是"半个生产者 + 半张嘴"。
///
/// 第一刀只切最要紧的一条轴（自养 / 异养），因为它是唯一一条
/// 已经被实测证明"卡在二值性上"的轴（见 §17.14 与批次 30）。
/// 其余器官（杀手 / 眼睛 / 再生 / 储食）仍由 `type_id` 分派，
/// 但**管线已经建好了**：加一条轴只是多一个字段 + 多一个分支。
///
/// **出厂预设只取 0 和 255 两个端点**，所以 `x / 255` 只可能是 0.0 或 1.0，
/// 对旧世界是逐位相同的（`organelle_axes_are_bit_identical` 守着这条）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Organelles {
    /// 光合能力。255 = 满速光合（旧的生产者细胞），0 = 完全不能自养。
    pub photo: u8,
    /// 摄食能力：吃食物 + 咬活体。255 = 旧嘴细胞，0 = 完全没有嘴。
    pub mouth: u8,
}

impl Organelles {
    pub const FULL: u8 = 255;

    /// 这条轴是不是"完全关闭"（0）。
    #[inline]
    pub fn is_dead(&self) -> bool {
        self.photo == 0 && self.mouth == 0
    }

    /// **器官负载**：这一格用掉了多少器官能力，0..=2。
    ///
    /// 维持成本按它缩放——这是"中间态能不能存在"的关键。
    ///
    /// 不做这件事的后果是实测出来的：成本与器官强度无关时，一个
    /// `photo = 128` 的格子**花同样的钱、只产一半**，被选择立刻淘汰，
    /// 6000 tick 之后中间态只剩 2.8%（而且那 2.8% 还是刚变异出来的，
    /// 站不住）。成本跟着强度走之后，"半个生产者"才是一个中性的选择。
    ///
    /// **没有器官的格子返回 1.0**（结构 / 装甲 / 眼睛按整格计价），
    /// 出厂预设（255,0）/（0,255）也正好是 1.0 —— 所以旧世界逐位不变。
    #[inline]
    pub fn load(&self) -> f64 {
        if self.is_dead() {
            return 1.0;
        }
        (self.photo as f64 + self.mouth as f64) / Self::FULL as f64
    }

    /// 是不是**中间态**：某条轴既不是 0 也不是 255。
    ///
    /// 这是阶段 3 的验收判据 —— 见 `DESIGN.md` §17.9 阶段 3 那一行。
    #[inline]
    pub fn is_intermediate(&self) -> bool {
        let mid = |v: u8| v > 0 && v < Self::FULL;
        mid(self.photo) || mid(self.mouth)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EnterFx {
    Burn,
    Bog,
    /// 水：踏入多耗一份能量（`TerrainCfg::water_move_cost`）。
    Slow,
    /// 冰：踏入后顺着原方向再滑一格。本身不花能量，改的是**位置**。
    Slide,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StandFx {
    Burn,
    Irradiate,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TickFx {
    Fertilize,
}

#[derive(Clone, Copy, Debug)]
pub struct CellDef {
    pub id: u8,
    pub key: &'static str,
    pub name: &'static str,
    pub label: &'static str,
    pub desc: &'static str,
    pub is_body_cell: bool,
    pub independent: bool,
    pub paintable: bool,
    pub visible: bool,
    pub directional: bool,
    pub terrain: bool,
    pub passable: bool,
    pub flag: Option<u16>,
    pub behavior: Option<Behavior>,
    pub enter: Option<EnterFx>,
    pub stand: Option<StandFx>,
    pub tick: Option<TickFx>,
    /// 这种类型在参数面板里暴露哪几项，**顺序即显示顺序**。
    ///
    /// 这是"按类型给参数"的单一真源：不是每种细胞都该有全部 10 项。
    /// `damage` 只对杀手/刺有意义、`sense_range` 只对眼睛有意义，其余类型给出来
    /// 就是死旋钮（拖了没反应）。
    ///
    /// 身体细胞声明 `ParamRef::Cell(..)`（落在 `Config.cells[id]`），
    /// 地形声明 `ParamRef::Terrain(..)`（落在 `Config.terrain`）。
    /// 空/食物/墙没有任何可调项 —— 它们的数值是写死在模拟里的，给滑块等于骗人。
    pub params: &'static [ParamRef],
    // 属性出厂默认（运行时以 Config.cells 为准）
    pub hp: f64,
    pub cost: f64,
    pub damage: f64,
    pub sense_range: f64,
    pub food_value: f64,
    /// 三套配色：classic / highContrast / colorblindSafe
    pub colors: [u32; 3],
}

const fn base(id: u8, key: &'static str, name: &'static str, label: &'static str) -> CellDef {
    CellDef {
        id,
        key,
        name,
        label,
        desc: "",
        is_body_cell: false,
        independent: false,
        paintable: false,
        visible: false,
        directional: false,
        terrain: false,
        passable: false,
        flag: None,
        behavior: None,
        enter: None,
        stand: None,
        tick: None,
        params: &[],
        hp: 1.0,
        cost: 0.0,
        damage: 0.0,
        sense_range: 0.0,
        food_value: 1.0,
        colors: [0x000000, 0x000000, 0x000000],
    }
}

/// 全部细胞类型（按 id 顺序）
pub static CELL_DEFS: [CellDef; 19] = [
    // 索引必须等于 id：def(id) 直接按下标取，顺序错了会拿到别人的定义
    CellDef {
        desc: "真空，生物可以自由穿过。",
        independent: true,
        paintable: true,
        colors: [0x0b1020, 0x000000, 0x0d1117],
        ..base(ct::EMPTY, "EMPTY", "empty", "空")
    },
    CellDef {
        desc: "一块食物，带着一份能量（尸体带的是死者攒下的）。嘴吃掉后按摄取倍率换算，剩下的会逐渐腐败流失。默认会挡住移动。",
        independent: true,
        paintable: true,
        visible: true,
        colors: [0x7d93a8, 0xc8d4e0, 0x9aa7b4],
        ..base(ct::FOOD, "FOOD", "food", "食物")
    },
    CellDef {
        desc: "实心墙：挡住移动，也挡住光（会在下面投下阴影）。不会被生物啃掉。",
        independent: true,
        paintable: true,
        visible: true,
        terrain: true,
        colors: [0x3a4150, 0x5c6470, 0x454c57],
        ..base(ct::WALL, "WALL", "wall", "墙")
    },
    CellDef {
        desc: "吃掉四邻域里的食物；也会咬相邻的活体——抽的是**能量**不是血，消化效率之外的差额散掉。",
        is_body_cell: true,
        paintable: true,
        visible: true,
        flag: Some(flag::MOUTH),
        behavior: Some(Behavior::Mouth),
        params: &[
            ParamRef::cell(AttrField::Hp),
            ParamRef::cell(AttrField::Cost),
            ParamRef::cell(AttrField::BiteMult),
            ParamRef::cell(AttrField::FoodValue),
        ],
        cost: 0.010,
        colors: [0xe07b39, 0xff9500, 0xe69f00],
        ..base(ct::MOUTH, "MOUTH", "mouth", "嘴")
    },
    CellDef {
        desc: "按所在格的光强把能量直接充进体内，暗处不产出；正上方同列的身体会挡光（包括自己）。会动的个体默认不能用。",
        is_body_cell: true,
        paintable: true,
        visible: true,
        flag: Some(flag::PRODUCER),
        behavior: Some(Behavior::Producer),
        params: &[
            ParamRef::cell(AttrField::Hp),
            ParamRef::cell(AttrField::Cost),
            ParamRef::cell(AttrField::ProdMult),
            ParamRef::cell(AttrField::FoodValue),
        ],
        cost: 0.010,
        colors: [0x4caf50, 0x00e676, 0x009e73],
        ..base(ct::PRODUCER, "PRODUCER", "producer", "生产者")
    },
    CellDef {
        desc: "赋予移动能力。有它才会动，移动方向由大脑输出决定。",
        is_body_cell: true,
        paintable: true,
        visible: true,
        flag: Some(flag::MOVER),
        // 移动器没有专属参数：移动是"一格/次"，没有速度概念。
        // 给它加速度要改移动逻辑（一次走两格？每 N tick 走一次？），那是改模拟，不是加参数。
        params: &[
            ParamRef::cell(AttrField::Hp),
            ParamRef::cell(AttrField::Cost),
            ParamRef::cell(AttrField::FoodValue),
        ],
        cost: 0.040,
        colors: [0x38bdf8, 0x00b0ff, 0x56b4e9],
        ..base(ct::MOVER, "MOVER", "mover", "移动器")
    },
    CellDef {
        desc: "每 tick 咬四邻域里的其他生物。一个个体每 tick 对同一个目标**只咬一口**，咬合力 = 单格伤害 × 自己的杀手细胞数——所以击杀时间 ≈ 目标血量 ÷ 咬合力，体型越大越耐打。装甲按减免比例挡下。",
        is_body_cell: true,
        paintable: true,
        visible: true,
        flag: Some(flag::KILLER),
        behavior: Some(Behavior::Killer),
        params: &[
            ParamRef::cell(AttrField::Hp),
            ParamRef::cell(AttrField::Cost),
            ParamRef::cell(AttrField::Damage),
            ParamRef::cell(AttrField::FoodValue),
        ],
        damage: 1.0,
        cost: 0.030,
        colors: [0xef4444, 0xff1744, 0xd55e00],
        ..base(ct::KILLER, "KILLER", "killer", "杀手")
    },
    CellDef {
        desc: "按「伤害减免」比例挡下杀手的伤害（出厂 1.00 = 完全免疫）。让身体更难被吃掉，但自己不产生任何收益。",
        is_body_cell: true,
        paintable: true,
        visible: true,
        flag: Some(flag::ARMOR),
        params: &[
            ParamRef::cell(AttrField::Hp),
            ParamRef::cell(AttrField::Cost),
            ParamRef::cell(AttrField::ArmorResist),
            ParamRef::cell(AttrField::FoodValue),
        ],
        cost: 0.020,
        colors: [0xa855f7, 0xd500f9, 0xcc79a7],
        ..base(ct::ARMOR, "ARMOR", "armor", "装甲")
    },
    CellDef {
        desc: "朝某个相对方向看出去，把看到的东西送进大脑。带朝向。",
        is_body_cell: true,
        paintable: true,
        visible: true,
        directional: true,
        flag: Some(flag::EYE),
        behavior: Some(Behavior::Eye),
        params: &[
            ParamRef::cell(AttrField::Hp),
            ParamRef::cell(AttrField::Cost),
            ParamRef::cell(AttrField::SenseRange),
            ParamRef::cell(AttrField::FoodValue),
        ],
        cost: 0.030,
        colors: [0xd8b4fe, 0xffffff, 0xf0e442],
        ..base(ct::EYE, "EYE", "eye", "眼睛")
    },
    CellDef {
        desc: "岩浆。踏进去挨一记冲击伤害，待在里面每 tick 继续烧；同时是**热源**，会把周围加热（温度伤害开启时更危险）。",
        independent: true,
        paintable: true,
        terrain: true,
        passable: true,
        enter: Some(EnterFx::Burn),
        stand: Some(StandFx::Burn),
        params: &[
            ParamRef::terrain(TerrainField::LavaEnter),
            ParamRef::terrain(TerrainField::LavaStand),
        ],
        colors: [0xff6a00, 0xff2d00, 0xe05a00],
        ..base(ct::LAVA, "LAVA", "lava", "岩浆")
    },
    CellDef {
        desc: "沼泽。走进去的那一步要额外耗能，能量不够就落到 0 进入饥饿计时。",
        independent: true,
        paintable: true,
        terrain: true,
        passable: true,
        enter: Some(EnterFx::Bog),
        params: &[ParamRef::terrain(TerrainField::SwampCost)],
        colors: [0x3f5540, 0x5a7d5a, 0x4d6b4d],
        ..base(ct::SWAMP, "SWAMP", "swamp", "沼泽")
    },
    CellDef {
        desc: "辐射带。每 tick 扣血，并让后代的变异率大幅提高。",
        independent: true,
        paintable: true,
        terrain: true,
        passable: true,
        stand: Some(StandFx::Irradiate),
        params: &[
            ParamRef::terrain(TerrainField::RadDamage),
            ParamRef::terrain(TerrainField::RadMutation),
        ],
        colors: [0xd4f000, 0xeeff00, 0xc8d400],
        ..base(ct::RAD, "RAD", "rad", "辐射带")
    },
    CellDef {
        desc: "腐殖质。以一定概率在**自己或相邻格**冒食物，等于一块会自己长东西的地。",
        independent: true,
        paintable: true,
        terrain: true,
        passable: true,
        tick: Some(TickFx::Fertilize),
        params: &[ParamRef::terrain(TerrainField::HumusFood)],
        colors: [0x6b4f2a, 0x8d6e3a, 0x7a5c33],
        ..base(ct::HUMUS, "HUMUS", "humus", "腐殖质")
    },
    CellDef {
        desc: "抬高普通繁殖的触发阈值：攒够更多能量才繁殖，但只花基础成本，差额留在身上当储备。（出芽繁殖用固定的启动成本，这一项不生效。）",
        is_body_cell: true,
        paintable: true,
        flag: Some(flag::STORAGE),
        params: &[
            ParamRef::cell(AttrField::Hp),
            ParamRef::cell(AttrField::Cost),
            ParamRef::cell(AttrField::StorageMult),
            ParamRef::cell(AttrField::FoodValue),
        ],
        cost: 0.020,
        food_value: 3.0,
        colors: [0xfbbf24, 0xffd600, 0xf0c000],
        ..base(ct::STORAGE, "STORAGE", "storage", "储食")
    },
    CellDef {
        desc: "每 tick 抹掉一点累积伤害。和装甲互补：装甲挡伤害，再生退伤害。",
        is_body_cell: true,
        paintable: true,
        flag: Some(flag::REGEN),
        behavior: Some(Behavior::Regen),
        params: &[
            ParamRef::cell(AttrField::Hp),
            ParamRef::cell(AttrField::Cost),
            ParamRef::cell(AttrField::RegenMult),
            ParamRef::cell(AttrField::FoodValue),
        ],
        cost: 0.025,
        colors: [0x5eead4, 0x00ffc8, 0x44d7b6],
        ..base(ct::REGEN, "REGEN", "regen", "再生")
    },
    CellDef {
        desc: "被杀手攻击时把伤害反弹回去。反伤不吃装甲减免。",
        is_body_cell: true,
        paintable: true,
        flag: Some(flag::SPIKE),
        params: &[
            ParamRef::cell(AttrField::Hp),
            ParamRef::cell(AttrField::Cost),
            ParamRef::cell(AttrField::Damage),
            ParamRef::cell(AttrField::FoodValue),
        ],
        damage: 1.0,
        cost: 0.020,
        colors: [0x94a3b8, 0xe0e6ee, 0xb8c0cc],
        ..base(ct::SPIKE, "SPIKE", "spike", "刺")
    },
    // ── 卵（批次 27）──
    //
    // 唯一一个"既不是地形、也不是身体细胞"的类型：它是**别人生的东西**。
    //
    // - `paintable: false`：玩家不该凭空画卵，那是模拟内部状态。
    // - `passable: false`：卵挡路。它挡住移动和繁殖，于是"产卵"第一次有了
    //   空间代价——一窝卵会把周围占满，这正是 r 策略该付的账。
    // - `visible: true` 但**不进 `VISIBLE_CELL_TYPES`**：画面上看得见，
    //   但大脑感知不到（那 8 个槽位已经满了，加第 9 个会改输入维度）。
    CellDef {
        desc: "母体产下的卵。孵化前可以被嘴吃掉——捕食压力第一次出现在繁殖环节。",
        visible: true,
        passable: false,
        colors: [0xf9a8d4, 0xffd6ec, 0xe9a8c9],
        ..base(ct::EGG, "EGG", "egg", "卵")
    },
    // ── 水（批次 43）──
    //
    // 和沼泽的分工：沼泽是"陷进去"（每步很贵），水是"阻力"（每步小贵）。
    // 两者的生态作用完全不同——沼泽是**禁区**，水是**走廊**：
    // 一条河不会挡住谁，只是让"绕路"和"蹚过去"第一次有了取舍。
    CellDef {
        desc: "可穿过，但每一步都多耗一份能量。地形的\"阻力\"，不是禁区。",
        terrain: true,
        passable: true,
        enter: Some(EnterFx::Slow),
        params: &[ParamRef::terrain(TerrainField::WaterCost)],
        colors: [0x2f4f7a, 0x4a7fb5, 0x3a6491],
        ..base(ct::WATER, "WATER", "water", "水")
    },
    // ── 冰（批次 43）──
    //
    // 唯一一种影响**位置**而不是收支的地形：踏入后顺着原方向再滑一格。
    // 于是"这一步的目标"不再等于"最终落点"——**预判两步第一次有价值**，
    // 这也是从批次 15 起一直没解决的「眼睛为什么稀有」的另一个答案。
    CellDef {
        desc: "可穿过。踏入后顺着原方向再滑一格——地形第一次影响「落点」而不只是收支。",
        terrain: true,
        passable: true,
        enter: Some(EnterFx::Slide),
        colors: [0xa8d8e8, 0xe8f6fb, 0xc8e8f2],
        ..base(ct::ICE, "ICE", "ice", "冰")
    },
];

pub const CELL_TYPE_COUNT: usize = CELL_DEFS.len();

#[inline]
pub fn def(id: u8) -> Option<&'static CellDef> {
    CELL_DEFS.get(id as usize)
}

#[inline]
pub fn label(id: u8) -> &'static str {
    def(id).map(|d| d.label).unwrap_or("未知")
}

#[inline]
pub fn is_body_cell(id: u8) -> bool {
    def(id).map(|d| d.is_body_cell).unwrap_or(false)
}

/// 这种类型的**出厂细胞器预设**。
///
/// 不往 `CELL_DEFS` 里加字段，是因为那要改 19 条记录、而且会让"类型"和"基因"
/// 两个真源并存。这里直接从已有的 `behavior` 推出来：
///
/// | `behavior` | photo | mouth |
/// |---|---|---|
/// | `Producer` | 255 | 0 |
/// | `Mouth` | 0 | 255 |
/// | 其余（含地形） | 0 | 0 |
///
/// **只取端点值**是刻意的：`x / 255` 只可能是 0.0 / 1.0，所以旧世界的
/// 行为逐位不变。要造中间态只有一条路——让变异去挪基因，而不是靠预设。
#[inline]
pub fn organelles_of(id: u8) -> Organelles {
    match def(id).and_then(|d| d.behavior) {
        Some(Behavior::Producer) => Organelles {
            photo: Organelles::FULL,
            mouth: 0,
        },
        Some(Behavior::Mouth) => Organelles {
            photo: 0,
            mouth: Organelles::FULL,
        },
        _ => Organelles::default(),
    }
}

#[inline]
pub fn is_directional(id: u8) -> bool {
    def(id).map(|d| d.directional).unwrap_or(false)
}

#[inline]
pub fn is_terrain(id: u8) -> bool {
    def(id).map(|d| d.terrain).unwrap_or(false)
}

/// 生物能否走进这一格。**只对地形生效**——食物是否阻挡由
/// `combat.foodBlocksReproduction` 单独决定（踩过的坑，见 JS 版注释）。
#[inline]
pub fn is_passable(id: u8) -> bool {
    def(id).map(|d| d.terrain && d.passable).unwrap_or(false)
}

/// id → 是不是地形（热路径查表）
pub static TERRAIN_MASK: [bool; 256] = {
    let mut m = [false; 256];
    let mut i = 0;
    while i < CELL_TYPE_COUNT {
        if CELL_DEFS[i].terrain {
            m[CELL_DEFS[i].id as usize] = true;
        }
        i += 1;
    }
    m
};

/// id → 是不是"需要每 tick 更新"的地形
pub static TICKS_TERRAIN: [bool; 256] = {
    let mut m = [false; 256];
    let mut i = 0;
    while i < CELL_TYPE_COUNT {
        if CELL_DEFS[i].tick.is_some() {
            m[CELL_DEFS[i].id as usize] = true;
        }
        i += 1;
    }
    m
};

/// 可被随机生成的生物细胞类型（结构变异用）
pub static LIVING_CELL_TYPES: [u8; 9] = [
    ct::MOUTH,
    ct::PRODUCER,
    ct::MOVER,
    ct::KILLER,
    ct::ARMOR,
    ct::EYE,
    ct::STORAGE,
    ct::REGEN,
    ct::SPIKE,
];

/// 参与大脑视觉输入的类型（**顺序即输入槽顺序，不要重排**）
pub static VISIBLE_CELL_TYPES: [u8; 8] = [
    ct::FOOD,
    ct::WALL,
    ct::MOUTH,
    ct::PRODUCER,
    ct::MOVER,
    ct::KILLER,
    ct::ARMOR,
    ct::EYE,
];

/// 可直接在世界画布上绘制的地形（画笔列表由它派生）
/// 画笔面板里的地形类型。**不含「空」**——擦除走专门的橡皮擦
/// （擦地形 / 擦生物两个），否则"画空"既能擦地形又能擦生物，
/// 两种意图混在一个笔刷里，谁都不好使。
pub static TERRAIN_BRUSH_TYPES: [u8; 8] = [
    ct::FOOD,
    ct::WALL,
    ct::LAVA,
    ct::SWAMP,
    ct::WATER,
    ct::ICE,
    ct::RAD,
    ct::HUMUS,
];

/// 有可调参数的**地形** —— 「类型参数」窗口第二段按钮由它派生，**顺序即显示顺序**。
///
/// 只有这 4 种在 `TerrainCfg` 里有对应字段。空 / 食物 / 墙的数值写死在模拟里，
/// 列进参数面板等于摆一个拖了没反应的旋钮。
pub static PARAM_TERRAIN_TYPES: [u8; 5] = [ct::LAVA, ct::SWAMP, ct::WATER, ct::RAD, ct::HUMUS];

/// 有"世界级每 tick 行为"的地形
pub static TICKING_TERRAIN_TYPES: [u8; 1] = [ct::HUMUS];

/// 细胞类型 id → 视觉槽序号（-1 = 不参与视觉）
pub static VISION_SLOT: [i8; 64] = {
    let mut arr = [-1i8; 64];
    let mut i = 0;
    while i < VISIBLE_CELL_TYPES.len() {
        arr[VISIBLE_CELL_TYPES[i] as usize] = i as i8;
        i += 1;
    }
    arr
};

// ────────────────────────────── 方向 ──────────────────────────────

pub mod dir {
    pub const UP: u8 = 0;
    pub const RIGHT: u8 = 1;
    pub const DOWN: u8 = 2;
    pub const LEFT: u8 = 3;
}

/// 方向 → (列增量, 行增量)
pub const DIR_VECTORS: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];

#[cfg(test)]
mod order_tests {
    use super::*;

    #[test]
    fn array_index_equals_id() {
        for (i, d) in CELL_DEFS.iter().enumerate() {
            assert_eq!(d.id as usize, i, "CELL_DEFS 下标必须等于 id（{} 的位置是 {}）", d.key, i);
        }
    }

    /// 每种类型都必须有描述（面板 / 工坊 / 笔刷的 tooltip 都直接用它）。
    #[test]
    fn every_type_has_a_description() {
        for d in CELL_DEFS.iter() {
            assert!(!d.desc.is_empty(), "「{}」没有描述", d.label);
            assert!(
                d.desc.chars().count() >= 6,
                "「{}」的描述太短，可能没写清楚：{}",
                d.label,
                d.desc
            );
        }
    }

    /// **描述必须跟得上机制**。
    ///
    /// 这个项目改机制很频繁，而描述是写死在 `CellDef` 里的静态字符串——
    /// 一不小心就变成"文档说 A、代码做 B"。批次 18 就抓到三处：
    ///
    /// | 类型 | 描述原来写 | 实际 |
    /// |---|---|---|
    /// | 杀手 | 每 tick 伤害四邻域里的格子；装甲格免疫 | 一个个体每 tick 对同一目标只咬一口；装甲按比例减免 |
    /// | 腐殖质 | 往**相邻**空格冒食物 | 自己那格也会冒 |
    /// | 食物 | 一份食物，嘴吃掉后换成能量 | 食物带能量存量，吃多少看存量，剩余会腐败 |
    ///
    /// 这里只钉住"改机制时最容易忘"的那几句关键词，不是逐字比对
    /// （逐字比对会让改文案变成改代码）。
    #[test]
    fn descriptions_match_current_mechanics() {
        let killer = def(ct::KILLER).unwrap().desc;
        assert!(
            killer.contains("只咬一口"),
            "杀手已经是「整口咬合」了，描述里必须说清楚：{}",
            killer
        );
        let armor = def(ct::ARMOR).unwrap().desc;
        assert!(
            armor.contains("减免"),
            "装甲已经是按比例减免（可调），不再是绝对免疫：{}",
            armor
        );
        let humus = def(ct::HUMUS).unwrap().desc;
        assert!(
            humus.contains("自己"),
            "腐殖质会在自己那格也冒食物，描述不能只说相邻格：{}",
            humus
        );
        let food = def(ct::FOOD).unwrap().desc;
        assert!(
            food.contains("腐败"),
            "食物会渐进腐败，描述里要有：{}",
            food
        );
        let producer = def(ct::PRODUCER).unwrap().desc;
        assert!(
            producer.contains("挡光"),
            "生产者的产出会被上方身体遮挡，描述里要有：{}",
            producer
        );
    }

    #[test]
    fn key_flags_are_wired() {
        assert_eq!(def(ct::STORAGE).unwrap().flag, Some(flag::STORAGE));
        assert_eq!(def(ct::REGEN).unwrap().flag, Some(flag::REGEN));
        assert_eq!(def(ct::SPIKE).unwrap().flag, Some(flag::SPIKE));
        assert!(def(ct::LAVA).unwrap().terrain);
        assert!(def(ct::HUMUS).unwrap().tick.is_some());
    }
}

#[cfg(test)]
mod organelle_tests {
    use super::*;

    /// 出厂预设**只取端点**。这是"旧世界逐位不变"的全部依据：
    /// `x / 255` 只可能是 0.0 或 1.0，乘上去等于没乘。
    #[test]
    fn presets_are_endpoints_only() {
        let p = organelles_of(ct::PRODUCER);
        assert_eq!((p.photo, p.mouth), (255, 0), "生产者：满速光合、没有嘴");

        let m = organelles_of(ct::MOUTH);
        assert_eq!((m.photo, m.mouth), (0, 255), "嘴：只有摄食");

        for id in [
            ct::MOVER,
            ct::KILLER,
            ct::ARMOR,
            ct::EYE,
            ct::STORAGE,
            ct::REGEN,
            ct::SPIKE,
        ] {
            let o = organelles_of(id);
            assert!(
                o.is_dead(),
                "{} 不该有光合/摄食能力（它靠 type_id 分派）",
                label(id)
            );
        }
        // 地形不是身体细胞，也不该有器官
        for id in [ct::EMPTY, ct::FOOD, ct::WALL, ct::LAVA, ct::WATER] {
            assert!(organelles_of(id).is_dead(), "{} 不该有器官", label(id));
        }
    }

    /// 中间态的定义：某条轴既不是 0 也不是 255。
    #[test]
    fn intermediate_means_strictly_between_the_endpoints() {
        let o = |p, m| Organelles { photo: p, mouth: m };
        assert!(!o(0, 0).is_intermediate(), "全关不是中间态");
        assert!(!o(255, 0).is_intermediate(), "满速生产者是端点");
        assert!(!o(0, 255).is_intermediate(), "满速嘴是端点");
        assert!(o(128, 0).is_intermediate(), "半速生产者是中间态");
        assert!(o(0, 64).is_intermediate(), "半张嘴是中间态");
        assert!(o(128, 128).is_intermediate(), "两轴都半开也是中间态");
    }
}
