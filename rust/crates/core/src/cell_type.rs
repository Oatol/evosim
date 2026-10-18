//! 细胞类型注册表 —— 与 JS 版 `src/life/CellType.js` 一一对应。
//!
//! 加一种新细胞 = 往 `CELL_DEFS` 里加一项（需要新行为再补一个 match 分支）。
//! 网格读写、配色、感知编码、结构变异、存档、参数面板都从这里读。

use crate::config::AttrField;

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

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EnterFx {
    Burn,
    Bog,
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
    /// 这种细胞在参数面板里暴露哪几项，**顺序即显示顺序**。
    ///
    /// 这是"按细胞类型给参数"的单一真源：不是每种细胞都该有全部 10 项。
    /// `damage` 只对杀手/刺有意义、`sense_range` 只对眼睛有意义，其余类型给出来
    /// 就是死旋钮（拖了没反应）。空数组 = 这种细胞没有可调参数（地形走 `Config.terrain`）。
    pub params: &'static [AttrField],
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
pub static CELL_DEFS: [CellDef; 16] = [
    // 索引必须等于 id：def(id) 直接按下标取，顺序错了会拿到别人的定义
    CellDef {
        desc: "真空，生物可以自由穿过。",
        independent: true,
        paintable: true,
        colors: [0x0b1020, 0x000000, 0x0d1117],
        ..base(ct::EMPTY, "EMPTY", "empty", "空")
    },
    CellDef {
        desc: "一份食物，嘴吃掉后换成能量；会缓慢腐败。",
        independent: true,
        paintable: true,
        visible: true,
        colors: [0x7d93a8, 0xc8d4e0, 0x9aa7b4],
        ..base(ct::FOOD, "FOOD", "food", "食物")
    },
    CellDef {
        desc: "实心墙，谁也穿不过去，也不会被生物啃掉。",
        independent: true,
        paintable: true,
        visible: true,
        terrain: true,
        colors: [0x3a4150, 0x5c6470, 0x454c57],
        ..base(ct::WALL, "WALL", "wall", "墙")
    },
    CellDef {
        desc: "吃掉四邻域里的食物，每份食物按「食物量倍率」换算能量。",
        is_body_cell: true,
        paintable: true,
        visible: true,
        flag: Some(flag::MOUTH),
        behavior: Some(Behavior::Mouth),
        params: &[
            AttrField::Hp,
            AttrField::Cost,
            AttrField::BiteMult,
            AttrField::FoodValue,
        ],
        cost: 0.002,
        colors: [0xe07b39, 0xff9500, 0xe69f00],
        ..base(ct::MOUTH, "MOUTH", "mouth", "嘴")
    },
    CellDef {
        desc: "以「生产者造食率」的概率在相邻空格长出食物；会动的个体默认不能用。",
        is_body_cell: true,
        paintable: true,
        visible: true,
        flag: Some(flag::PRODUCER),
        behavior: Some(Behavior::Producer),
        params: &[
            AttrField::Hp,
            AttrField::Cost,
            AttrField::ProdMult,
            AttrField::FoodValue,
        ],
        cost: 0.002,
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
        params: &[AttrField::Hp, AttrField::Cost, AttrField::FoodValue],
        cost: 0.008,
        colors: [0x38bdf8, 0x00b0ff, 0x56b4e9],
        ..base(ct::MOVER, "MOVER", "mover", "移动器")
    },
    CellDef {
        desc: "每 tick 伤害四邻域里属于其他生物的格子；装甲格免疫。",
        is_body_cell: true,
        paintable: true,
        visible: true,
        flag: Some(flag::KILLER),
        behavior: Some(Behavior::Killer),
        params: &[
            AttrField::Hp,
            AttrField::Cost,
            AttrField::Damage,
            AttrField::FoodValue,
        ],
        damage: 1.0,
        cost: 0.006,
        colors: [0xef4444, 0xff1744, 0xd55e00],
        ..base(ct::KILLER, "KILLER", "killer", "杀手")
    },
    CellDef {
        desc: "免疫杀手的伤害。让身体更难被吃掉，但自己不产生任何收益。",
        is_body_cell: true,
        paintable: true,
        visible: true,
        flag: Some(flag::ARMOR),
        params: &[
            AttrField::Hp,
            AttrField::Cost,
            AttrField::ArmorResist,
            AttrField::FoodValue,
        ],
        cost: 0.004,
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
            AttrField::Hp,
            AttrField::Cost,
            AttrField::SenseRange,
            AttrField::FoodValue,
        ],
        cost: 0.006,
        colors: [0xd8b4fe, 0xffffff, 0xf0e442],
        ..base(ct::EYE, "EYE", "eye", "眼睛")
    },
    CellDef {
        desc: "岩浆。踏进去挨一记冲击伤害，待在里面每 tick 继续烧。",
        independent: true,
        paintable: true,
        terrain: true,
        passable: true,
        enter: Some(EnterFx::Burn),
        stand: Some(StandFx::Burn),
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
        colors: [0xd4f000, 0xeeff00, 0xc8d400],
        ..base(ct::RAD, "RAD", "rad", "辐射带")
    },
    CellDef {
        desc: "腐殖质。以一定概率往相邻空格冒食物，等于一块会自己长东西的地。",
        independent: true,
        paintable: true,
        terrain: true,
        passable: true,
        tick: Some(TickFx::Fertilize),
        colors: [0x6b4f2a, 0x8d6e3a, 0x7a5c33],
        ..base(ct::HUMUS, "HUMUS", "humus", "腐殖质")
    },
    CellDef {
        desc: "抬高繁殖阈值：攒够更多能量才繁殖，但只花基础成本，差额留在身上当储备。",
        is_body_cell: true,
        paintable: true,
        flag: Some(flag::STORAGE),
        params: &[
            AttrField::Hp,
            AttrField::Cost,
            AttrField::StorageMult,
            AttrField::FoodValue,
        ],
        cost: 0.004,
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
            AttrField::Hp,
            AttrField::Cost,
            AttrField::RegenMult,
            AttrField::FoodValue,
        ],
        cost: 0.005,
        colors: [0x5eead4, 0x00ffc8, 0x44d7b6],
        ..base(ct::REGEN, "REGEN", "regen", "再生")
    },
    CellDef {
        desc: "被杀手攻击时把伤害反弹回去。反伤不吃装甲减免。",
        is_body_cell: true,
        paintable: true,
        flag: Some(flag::SPIKE),
        params: &[
            AttrField::Hp,
            AttrField::Cost,
            AttrField::Damage,
            AttrField::FoodValue,
        ],
        damage: 1.0,
        cost: 0.004,
        colors: [0x94a3b8, 0xe0e6ee, 0xb8c0cc],
        ..base(ct::SPIKE, "SPIKE", "spike", "刺")
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
pub static TERRAIN_BRUSH_TYPES: [u8; 7] = [
    ct::EMPTY,
    ct::FOOD,
    ct::WALL,
    ct::LAVA,
    ct::SWAMP,
    ct::RAD,
    ct::HUMUS,
];

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

    #[test]
    fn key_flags_are_wired() {
        assert_eq!(def(ct::STORAGE).unwrap().flag, Some(flag::STORAGE));
        assert_eq!(def(ct::REGEN).unwrap().flag, Some(flag::REGEN));
        assert_eq!(def(ct::SPIKE).unwrap().flag, Some(flag::SPIKE));
        assert!(def(ct::LAVA).unwrap().terrain);
        assert!(def(ct::HUMUS).unwrap().tick.is_some());
    }
}
