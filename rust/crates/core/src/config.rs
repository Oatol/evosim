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
    /// 嘴：吃食物 / 咬活体时，能量的倍率
    pub bite_mult: f64,
    /// 生产者：光合产出的倍率
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
    /// **云影强度** 0~1（默认 0 = 关闭，与旧行为逐位相同）。
    ///
    /// 天空光乘一张平滑噪声图：亮的成"光岛"，暗的成"暗廊"。
    /// 目的不是让世界变暗，而是把**明暗交界**从一条水平线（200 格）
    /// 拉成几千格的锯齿——异养者靠的就是那条交界。
    ///
    /// 批次 10 的结论是「光太均匀」才是专性异养的瓶颈，这一项就是冲着它去的。
    ///
    /// **代价**：透射率恒 ≤ 1，所以开云影一定降低总光照（`p = 1` 时平均约 0.5）。
    /// 想保住总能量就同时把「天空光」或「光合速率」提上去。
    pub sky_patchiness: f64,
    /// **富斑块 / 贫斑块**强度（0~1，默认 0 = 关闭，逐位等于旧行为）。
    ///
    /// 世界被一张低频噪声图切成「光岛」和「暗廊」，这一项决定**暗廊能有多穷**：
    /// `patch_floor = 1 - patch_richness`，也就是暗廊里还剩多少比例的天空光。
    ///
    /// | 取值 | 暗廊的光 | 生态后果 |
    /// |---|---|---|
    /// | 0（默认） | 100%（无效果） | 环境均匀，全局一套策略 |
    /// | 0.5 | 50% | 暗廊能活但长得慢 |
    /// | 0.7 | 30% | **推荐**：暗廊只够维持、不够长身体 |
    /// | 0.9 | 10% | 暗廊接近收支平衡，极少数能活 |
    /// | 1.0 | 0% | 暗廊全黑，等于回到「云影」的极端 |
    ///
    /// **为什么这个数必须落在「能活但长不大」那个窗口里**：满照一个生产者挣
    /// 0.12/tick、成本 0.010，净 0.11；长一格要 1.0 能量，也就是 9 tick。
    /// 芽的「提前成熟」阈值是 30 tick，所以净收入掉到 0.033/tick 以下才会卡住——
    /// 对应**满照的 30% 左右**。低于 20% 就连维持都困难，那里会变成坟场而不是生态位。
    ///
    /// 它和 `sky_patchiness` 共用同一张噪声图（同一个种子），所以
    /// 「光岛」和「富斑块」在空间上是**同一批地方**——这在生态上也对：
    /// 阳光充足的地方本来就是食物链的起点。
    pub patch_richness: f64,
    /// **环境温度（摄氏度）**。温度场的 0 就是这个世界温度。
    ///
    /// 温度场存的是"**高于环境的温差**"（0–255），不是绝对温度：
    /// 局部温度 = `ambient_temp + (场值/255) × temp_span`。
    /// 所以调高这一项 = 整个世界变热，**所有生物都开始受热**，不只是岩浆附近。
    /// 岩浆本身钉在满值，永远是 `ambient + span` 摄氏度。
    pub ambient_temp: f64,
    /// 温度场满量程（255）对应的温差，摄氏度。
    ///
    /// 默认 100：岩浆 ≈ 122℃（环境 22℃ 时）。调小 = 岩浆没那么烫，
    /// 但同样的场值会被解释成更大的温差，热区更危险。
    pub temp_span: f64,
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
            sky_patchiness: 0.0,
            patch_richness: 0.0,
            ambient_temp: 22.0,
            temp_span: 100.0,
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
    /// 嘴咬活体：每个嘴细胞每 tick 从相邻活体身上**抽走**多少能量（默认 0.3，0 = 关闭）。
    ///
    /// 这是「捕食要有正收益」的地基。批次 9 的诊断发现：杀手能杀人，但尸体按格
    /// 撒在死者身上，凶手只贴着其中一格——大部分能量它够不着，于是捕食的实际回报
    /// 远低于当生产者，杀手和移动器一起灭绝。嘴咬活体把回报直接接到嘴上：
    /// 抽多少立刻到账，不用等对方死，也不用绕到尸体旁边。
    ///
    /// 抽走量被**对方当时身上剩的能量**卡住，所以"咬一口空壳"什么也得不到——
    /// 长期吸一个生产者等于只吸走它的产出（0.12/tick），那是放牧，不是屠宰。
    /// 真正的大餐是储食个体：容量 6.0 意味着能连吸 20 tick。
    pub bite_rate: f64,
    /// 抽走的能量有多少变成自己的，其余进热池（默认 0.8）。
    ///
    /// **必须小于 1**：捕食不是无损搬运，消化要付代价。这一项同时是
    /// 「营养级每升一级只剩 10~20%」这条生态学规律里我们可以直接拧的那个数。
    /// 批次 10 的守恒审计会把 `(1 - 效率)` 那部分记成"离开生物圈"。
    pub bite_efficiency: f64,
    /// 每个细胞提供的能量容量。光合直充体内之后必须有上限，否则无限膨胀。
    pub energy_per_cell: f64,
    /// 每个储食细胞额外提供的能量容量（电池）。
    ///
    /// 储食细胞因此同时是收益和风险：装得多 = 熬得过饥荒 / 黑夜，
    /// 但被杀死时对方从尸体上吃到的也更多。
    pub storage_capacity: f64,
    pub movers_can_produce: bool,
    pub turn_cooldown: i32,
    /// 全局消耗量倍率 —— **「活着的价格」**（默认 1.0 = 按细胞类型参数里写的开销付账）。
    ///
    /// 1.x 里它是 0，等于关掉了代谢：细胞不吃饭也不会饿死。后果是
    /// 「养一个生产者细胞」不要钱，光合就成了严格占优策略——
    /// 批次 9.5 实测异养占比恒为 0，正是因为这个。
    ///
    /// 批次 10 把它拨到 1.0，并把每种细胞的 `cost` 改成**真实量级**
    /// （生产者 0.010/tick 对满照收入 0.12/tick，约 8%）。
    /// 之前那些 0.002~0.008 是"装饰值"：乘 0 永远是 0，乘 5 也才占收入的百分之几。
    ///
    /// **量级是怎么定的**：满照下一个生产者细胞挣 0.12/tick。
    /// 消耗必须让「每格平均开销」落在收入的一半以下，否则移动器（0.040）
    /// 这种贵细胞会直接把个体拖死。实测扫描（10000 tick、200×150）：
    /// 倍率 1.0 世界健康（~1100 个体 / 致密 1%），2.0 开始稀疏（~600），
    /// 4.0 已经濒临灭绝。所以默认 1.0，滑块上限 5 留给"想看灭绝"的人。
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
    /// 繁殖失败要不要照样扣能量。
    ///
    /// `false`（默认）= **先检查空间，通过了才扣钱**——"找不到地方就继续攒着"。
    /// `true` = 旧行为：`reproduce_organism` 先扣钱再检查空间，
    /// 所以"想生但没地方"的个体白付一次 `food_needed`。
    ///
    /// 实测（15000 tick、3 种子、p = 1.0）**86.5% 的繁殖尝试失败**：
    /// 63.6% 卡在"没整块空地"、22.9% 卡在"路径被挡"、成功只有 13.5%。
    /// 也就是说繁殖成本的实际支出是名义值的 **7.4 倍**，而且大体型付得更多
    /// （它要更多连通空地）。这一项单独就把体型锁死在 3 格，
    /// 让「繁殖成本指数 p < 1」的规模经济完全失效——
    /// p 从 1.0 压到 0.5，平均体型 2.99 → 3.07，纹丝不动。
    ///
    /// 两种都要能跑：这是个有争议的设计选择（失败该不该罚），
    /// 而且它和 `breed_cost_exponent` 是耦合的（p < 1 只有在失败不罚时才有意义）。
    ///
    /// **默认已翻成 `false`**（批次 11）：先检查再收费更符合实际——真实的繁殖
    /// 不会因为"找不到产房"就把身上的能量烧掉。旧行为还有一个副作用：
    /// 失败把能量清到 0 → 进入饥饿计时 → 反复失败几次就饿死，
    /// 于是**繁殖失败本身成了主要死因**（实测人口 956 vs 1784）。
    ///
    /// **代价**：失败的个体每 tick 都会重试，尝试次数涨约 20 倍，
    /// tick 耗时约 1.67 倍。要压回去就得加"失败冷却"，那是个新旋钮。
    pub repro_fail_costs_energy: bool,
    /// 每次繁殖尝试最多试几个候选位置。`1` = 旧行为（随机挑一个方向 + 距离，
    /// 不行就放弃）。
    ///
    /// 为什么需要这个旋钮：实测（10000 tick、3 种子）**86% 的繁殖尝试失败**，
    /// 其中 62% 是「整块空地找不到」，而同一时刻全局格子占用只有 **5%**。
    /// 这两个数矛盾，说明瓶颈不是「世界太挤」，而是「只看 12 个候选位置
    /// （4 方向 × 距离 4~6），它们恰好被邻居占了」——这是**局部**拥挤，
    /// 不是全局拥挤。
    ///
    /// 调大之后个体在放弃前会多试几个位置。它不改变「后代要一整块空地」
    /// 这个语义，只是让搜索更努力。想彻底消掉那个语义得做「出芽」。
    pub repro_search_tries: usize,
    /// **最小可繁殖体型**（细胞数）。`1` = 旧行为（任何体型都能繁殖）。
    ///
    /// 为什么需要它：这个世界的经济学是**规模中性**的——产出、维持成本、
    /// 能量容量、繁殖门槛全部随体型线性增长，所以繁殖速率 `0.11n / n`
    /// 与体型无关。而唯一非线性的一项是「出生要一整块连通空地」，
    /// 它**惩罚大体型**（3 格要 3 格空地，1 格只要 1 格）。
    ///
    /// 于是：给大体型发奖励（`breed_cost_exponent < 1` 的规模经济）永远赢不了。
    /// 实测 p 从 1.0 压到 0.5，平均体型 2.99 → 3.07，纹丝不动；
    /// 把繁殖位置搜索范围从 1 个候选提到 12 个，体型反而从 2.67 掉到 1.89
    /// （2 格占 76.9%）——因为搜索越充分，小体型越占优。
    ///
    /// 所以只能反过来：**让"太小"付出代价**。调到 3 之后，1~2 格的个体
    /// 攒再多能量也不能繁殖，基因传不下去。
    pub min_breed_cells: usize,
    /// 维持成本随体型的指数：`总开销 = Σ细胞cost × 体型^(p-1)`。
    ///
    /// `1.0` = 旧行为（每格成本相同，总开销线性于体型）。
    /// `< 1` = **Kleiber 定律**：单位体重的代谢率随体型下降（`总开销 ∝ n^0.75`）。
    ///
    /// 为什么这条在生物学上比「繁殖成本指数」更根本：真实动物的大半摄入能量
    /// 都花在维持上（基础代谢），所以「单位体重代谢率」直接决定**每天的净收入**，
    /// 而净收入决定繁殖速率。`breed_cost_exponent` 只改一次性成本，
    /// 对「每 tick 挣多少」毫无影响——这就是它实测无效的原因之一。
    ///
    /// **但它单独也不够**：现在细胞 `cost` 只占满照收入的 8.3%，
    /// 所以 n^0.75 的折扣最多让净收入涨 4%（10 格 vs 1 格）。
    /// 要让「代谢效率」真正有分量，`cost` 必须占到收入的 40% 以上——
    /// 这也更接近真实生物（基础代谢占摄入能量的大头）。
    pub upkeep_exponent: f64,
    /// 环境温度造成的每 tick 伤害（`0` = 关闭，默认）。
    ///
    /// 温度场 `fields.temp` 是 0–255（0 = 本底，岩浆 = 255）。
    /// 超过 `temp_comfort` 的部分按**平方**放大，所以越靠近热源越致命。
    ///
    /// **这条是「表面积/体积比」的另一半**：`upkeep_exponent` 管「每天吃多少」，
    /// 这一条管「能住在哪」。真实动物体型越大，表面积/体积比越小，
    /// 越不容易被环境温度左右（老鼠要不停吃来产热，野牛不用）。
    pub temp_damage: f64,
    /// **舒适温度（摄氏度）**。局部温度低于它不受伤害。
    ///
    /// 局部温度 = `world.ambient_temp + (场值/255) × world.temp_span`。
    /// 默认 35℃、环境 22℃ → 场值 0 的常温世界是安全的，
    /// 靠近岩浆才开始疼。把 `ambient_temp` 调到 40℃ 以上，
    /// 整个世界都会开始扣血——这就是"全球变暖"旋钮。
    pub comfort_temp: f64,
    /// 伤害随体型的指数：`伤害 ∝ n^temp_size_exponent`。
    ///
    /// **默认 `-0.25`**，理由是"体温"而不是"接触面积"：
    ///
    /// ```text
    /// 热容 ∝ 体积 ∝ n
    /// 散热/吸热 ∝ 表面积 ∝ n^0.75
    /// 升温速率 ∝ 表面积/体积 ∝ n^-0.25   ← 小个体升得快
    /// ```
    ///
    /// 血量 `max_health` = 每格 1 HP，所以 `= n`。于是生存时间
    /// `∝ n / n^-0.25 = n^1.25`：**10 格个体在热区能撑 17.8 倍于 1 格个体**。
    ///
    /// 为什么不用 `+0.75`（"伤害 ∝ 接触面积"）：那会给出生存时间 ∝ `n^0.25`，
    /// 10 格只有 1.78 倍优势——在"体型只有 1~5 格"的世界里根本筛不动任何东西
    /// （实测：热区里一个生物都没有，全被赶出去了，没有梯度）。
    ///
    /// `0` = 伤害与体型无关（大个体只靠血厚，线性优势）。
    pub temp_size_exponent: f64,
    /// **成长过程**：后代以 1 格的「芽」出生，消耗能量一格格长到母体体型。
    ///
    /// `false`（默认）= 旧行为：繁殖直接复制整个身体，出生即成年。
    ///
    /// 为什么值得单独做：现在的繁殖要**整块连通空地**，实测 86% 的尝试
    /// 都失败在这个几何约束上。芽只要 1 格，出生难度直接掉一个量级。
    /// 而且「长成之前不能繁殖」把体型变成了**过程中的量**——
    /// 这是 r/K 分化的地基（环境可塑是下一步）。
    ///
    /// **注意它不保证体型变大**：出生更容易会偏向小体型（小身体要的空地少）。
    /// 能不能推大，取决于 `growth_cost` 和体型收益的相对大小——见批次 12 实测。
    pub growth_enabled: bool,
    /// 每长一格要花的能量。`0` = 只花时间不花能量。
    ///
    /// 默认 1.0：和「繁殖门槛 = 体型」同一个量级，所以长成一个 n 格的成体
    /// 总共花 `n-1` 能量，和旧行为一次性付 `n` 差不多。
    /// 调大 = 大体型更贵（体型变小）；调小 = 生长几乎免费（体型可能变大）。
    pub growth_cost: f64,
    /// **亲代投资**：母体付的繁殖成本里，有多大比例转给芽当启动能量（0–1）。
    ///
    /// 默认 0.5：3 格母体付 3 能量，芽带 1.5 出生，够它立刻长一格还有余。
    ///
    /// 为什么必需：芽出生时是 1 格 1 HP、能量 0——**完美的猎物**。
    /// 实测 `growth_seed_fraction = 0` 时，杀手占比从 14% 暴涨到 54%，
    /// 人口从 1359 崩到 117：芽刚落地就被吃，捕食者在"繁殖环节"大爆发。
    /// 给一点启动能量能显著提高幼体存活率，而且这本身就是 r/K 的核心
    /// （投多少给后代，是一个真实的权衡）。
    pub growth_seed_fraction: f64,
    /// **环境可塑**：芽连续多少个 tick 长不动就「提前成熟」（`0` = 关闭，默认）。
    ///
    /// 长不动 = 能量不够（食物稀缺）**或**目标格被占（拥挤）。两者都算，
    /// 因为它们在生态上是一回事：**这个环境撑不起这么大的身体**。
    ///
    /// 提前成熟 = 放弃长满，以当前体型成年、立刻能繁殖。后代的目标体型
    /// 继承的是**成熟时的实际体型**，所以：
    ///
    /// - 食物充足 → 长满 → 大体型 → 后代也是大体型（K 策略）
    /// - 食物稀缺 → 提前成熟 → 小体型 → 后代也是小体型（r 策略）
    ///
    /// **同一个基因型，在不同环境下长成不同体型** —— 这是真正的 r/K 分化，
    /// 而不是靠一条"最小体型"规则硬掰。
    ///
    /// 建议值 30~60（饥饿致死是 `starvation_ticks = 60`，太大会等不到成熟就饿死）。
    pub growth_stall_mature: u32,
    /// **最短成长期**（tick）。芽至少要活过这么多 tick 才能成年。
    ///
    /// **默认 20**。它只在 `growth_enabled` 打开时才起作用，所以对旧世界
    /// 零影响——但一旦打开成长，`0` 会让整个机制塌成"1 格最优"（实测平均体型
    /// 1.69），而 `20` 能到 4~8 格。这个默认值不是口味问题，是功能是否成立的问题。
    ///
    /// 为什么需要它：`growth_plan` 是**母体成熟时的体型**，所以 1 格母体的后代
    /// 目标就是 1 格——它**出生那一 tick 就满足"长满了"**，立刻成年、立刻繁殖。
    /// 而 3 格目标要花 20~40 tick 去长。
    ///
    /// 结果是 r 基因型（小目标）在**任何环境里**都赢：批次 12/12.1/13 三次实测
    /// 都撞在这堵墙上（富贫斑块造出真实的光梯度，体型仍然没有分化，
    /// 见 §9.7.14）。这不是"小更好"，而是"小**免费**"。
    ///
    /// 打开之后，即使 1 格目标也要等 `N` tick 才成年，「生得多」和「长得快」
    /// 第一次有了取舍。注意它是**下限**不是**加时**：3 格身体本来就要长
    /// 20~40 tick，`N = 20` 对它没有额外惩罚，只惩罚"什么都不用长"的个体。
    ///
    /// 建议值 15~30（饥饿致死是 `starvation_ticks = 60`，别超过它）。
    pub growth_min_ticks: u32,
    /// **出芽**：芽不生在 4~6 格外的空地，而是**紧贴母体的身体**，
    /// 并且在未成熟期间由母体每 tick 供养。
    ///
    /// `false`（默认）= 旧行为：后代生在外面的空地上，自己养活自己。
    ///
    /// 为什么这是「大个体第一次能赢」的机制：现在的繁殖成本是
    /// `food_needed = 细胞数`，也就是**随母体体型线性增长**；而后代出生时只有 1 格，
    /// 根本不需要那么多启动资金。于是"每产一个后代的成本"被体型除掉了，
    /// 繁殖速率对体型完全中性（`0.11n / n = 0.11`，见 §9.7.13）。
    ///
    /// 出芽把这件事改回来：
    /// - 启动成本是**固定**的（`bud_start_cost`，只够 1 格芽用），不随体型涨；
    /// - 芽位随体型涨（`bud_cells_per_slot`），大个体能**同时**挂多个芽；
    /// - 芽挂在母体上，母体每 tick 喂它 `bud_feed`，所以芽长得比自己攒快得多。
    ///
    /// 于是后代产出速率 ∝ 芽位数 ∝ 体型。**这是目前唯一一个超线性机制**：
    /// 其他所有方案（Kleiber、繁殖折扣、温度、富贫斑块、最短成长期）都是
    /// 线性缩放或一次性惩罚。
    pub budding_enabled: bool,
    /// 每多少格身体换一个**芽位**（同时能挂几个未成熟的芽）。默认 **8.0**。
    ///
    /// `floor(n / 这个数)`，**没有下限**：不到这个体型就没有芽位、退回普通繁殖。
    ///
    /// 这个数必须够大，否则出芽会**反过来压小体型**——实测（6000 tick）：
    ///
    /// | 每芽位格数 | 平均体型 | 说明 |
    /// |---|---|---|
    /// | 2 | 1.80 | 2 格就能出芽，小体型白拿一个"出生免检" |
    /// | 4 | 2.59 | |
    /// | 6 | 3.15 | |
    /// | 8 | 3.70 | 只有 ≥8 格才出芽 |
    ///
    /// 原因是出芽最直接的效果是**"出生不用再找 4~6 格外的空地"**
    /// （那件事失败率 70%），而"让出生变容易"永远是偏向小体型的
    /// （批次 12 的结论）。所以芽位必须稀缺到只有大体型拿得到。
    pub bud_cells_per_slot: f64,
    /// 产一个芽的**一次性**成本（固定，不随体型增长）。默认 1.0。
    ///
    /// 这正是超线性的来源：1 格母体付 1.0，10 格母体也只付 1.0。
    /// 注意它和 `food_needed = 细胞数` 是两套账，出芽模式下不再用后者。
    pub bud_start_cost: f64,
    /// 母体每 tick 给**每个**未成熟芽的能量。默认 **0.5**。
    ///
    /// 满照一个生产者自己挣 0.12/tick，所以 0.5 是它的 4 倍——芽几乎是**被母体喂大的**。
    /// 这一点很关键：喂得少（0.03）时芽主要靠自己攒，而"自己攒多久"正比于目标体型，
    /// 于是大体型的芽长得慢、芽位长期占满，出芽的优势被吃光（实测体型 3.70）。
    /// 喂得足（0.5）时芽的成长时间被压到「最短成长期」那个固定值上，
    /// 芽位周转快，大体型才真正赢（实测体型 5.57）。
    pub bud_feed: f64,
}

impl Default for LifeCfg {
    fn default() -> Self {
        LifeCfg {
            lifespan_multiplier: 100.0,
            extra_mover_food_cost: 0.0,
            max_organisms: -1,
            photosynthesis_rate: 0.12,
            shade_strength: 0.25,
            bite_rate: 0.3,
            bite_efficiency: 0.8,
            energy_per_cell: 2.0,
            storage_capacity: 6.0,
            movers_can_produce: false,
            turn_cooldown: 4,
            metabolism_scale: 1.0,
            food_energy_scale: 1.0,
            starvation_ticks: 60,
            storage_food_bonus: 1.0,
            regen_rate: 0.1,
            breed_cost_exponent: 1.0,
            max_body_cells: 0,
            repro_fail_costs_energy: false,
            repro_search_tries: 1,
            min_breed_cells: 1,
            upkeep_exponent: 1.0,
            temp_damage: 0.0,
            comfort_temp: 35.0,
            temp_size_exponent: -0.25,
            growth_enabled: true,
            growth_cost: 1.0,
            growth_seed_fraction: 0.5,
            growth_stall_mature: 0,
            growth_min_ticks: 20,
            budding_enabled: true,
            bud_cells_per_slot: 8.0,
            bud_start_cost: 1.0,
            bud_feed: 0.5,
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
    /// **整口咬合**：一个个体每 tick 对**同一个目标**只造成一次伤害。
    ///
    /// 伤害大小 = `damage × 这个个体的杀手细胞数`，**不按接触面积叠加**。
    ///
    /// `true`（默认）= 新行为。`false` = 旧行为：每贴着一个目标格子各算一次。
    ///
    /// 为什么必须改：血量是**体积**（每格 1 HP，所以 n 格 = n HP），
    /// 而旧逻辑的伤害是**接触面积**——一个 20 格的猎物会同时被 3~5 个杀手格
    /// 咬到，于是每 tick 挨 3~5 点，20 点血 4~5 tick 就没了。
    /// 也就是说**击杀时间 ∝ n / 接触格数 ≈ √n**，体型几乎买不到任何生存时间。
    ///
    /// 改成整口之后：**击杀时间 = 目标血量 / 攻击者咬合力**，
    /// 1 格猎物 1 tick、5 格 5 tick、20 格 20 tick。体型第一次成为真实的防御。
    /// 而杀手要更快，就得多来几只（每只各自贡献一份咬合力）——「围攻」有了意义。
    pub killer_bite_capped: bool,
    /// 食物是否挡住**繁殖**（`can_place_at`）。
    ///
    /// 默认 `true`：新生儿不能生在一格食物里。允许的话那一格食物会被直接压掉，
    /// 而"生孩子顺手把饭砸了"不是个有意思的机制。
    pub food_blocks_reproduction: bool,
    /// 食物是否挡住**移动**（`is_clear`）。
    ///
    /// 默认 `true`（与 1.x 一致）。调成 `false` 之后生物可以走进/穿过食物格——
    /// 这是给「捕食者要穿过一整片尸体堆去追猎物」这种场景准备的。
    ///
    /// **代价必须知道**：身体盖上去会把那一格食物压碎，`types` 是单层，
    /// 食物的能量没地方存。压碎的能量记进 `heat_pool`（不是吃掉，是毁掉），
    /// 所以"穿过尸体堆"本身是有成本的——**踩碎的比吃到的多**。
    pub food_blocks_movement: bool,
}

impl Default for CombatCfg {
    fn default() -> Self {
        CombatCfg {
            insta_kill: false,
            killer_bite_capped: true,
            food_blocks_reproduction: true,
            food_blocks_movement: true,
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
            // 消耗出厂 0.010~0.040（批次 10 起是真实量级，对满照收入 0.12/tick），
            // 步长必须比最小值细，否则 0.010 会被吸附成 0
            AttrField::Cost => (0.0, 0.08, 0.001, 3),
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
            AttrField::Hp => "这一格贡献多少血量。整只生物的血量是各格之和，累积伤害超过它才死；0 = 这一格不贡献血量。",
            AttrField::Cost => {
                "每 tick 的能量开销（会再乘上全局消耗量倍率）。满照下一个生产者细胞挣 0.12。"
            }
            AttrField::Damage => "每个杀手细胞贡献的咬合力。一个个体每 tick 对同一目标只咬一口，所以总咬合力 = 单格伤害 × 杀手细胞数。",
            AttrField::SenseRange => "眼睛能看多远。0 = 用全局视野距离。",
            AttrField::FoodValue => "死亡时这一格最多能装多少能量（残骸容量）。体内的能量按格平均分配，装不下的撒到周围。",
            AttrField::BiteMult => "吃食物和咬活体都乘它：一份食物换多少能量 / 一口能抽走多少。",
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
