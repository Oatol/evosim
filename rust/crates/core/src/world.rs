//! 世界环境：网格 + 生物集合 + 环境规则 + 两阶段主循环。
//! 与 JS 版 `src/core/World.js` 对应。
//!
//! 借用策略：**纯计算**在 `organism.rs`，**写世界的动作**在本文件。
//! 原因是 Rust 不允许"拿着 `&mut 自己` 再去借 `&mut 世界`"，
//! 所以 act / reproduce / die / harm 都写成了 `&mut self` 的方法，按索引访问个体。

use std::collections::HashMap;
use std::sync::Arc;

use crate::anatomy::{BodyCell, Flags};
use crate::cell_type::{
    self, ct, Behavior, EnterFx, StandFx, DIR_VECTORS, TERRAIN_MASK, TICKS_TERRAIN,
    TICKING_TERRAIN_TYPES,
};
use crate::config::AttrField;
use crate::config::Config;
use crate::grid::Grid;
use crate::mutation::mutate_organism;
use crate::organism::Organism;
use crate::rng::Rng;
use crate::species::SpeciesRegistry;

const NEIGHBORS_4: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];

/// 一套 swap-remove 索引（食物 / 腐殖质都用这个套路）
#[derive(Default, Clone)]
pub struct TerrainSet {
    pub cells: Vec<usize>,
    pub pos: Vec<i32>,
}

pub struct World {
    pub config: Config,
    pub rng: Rng,
    pub grid: Grid,
    pub tick: u64,

    pub organisms: Vec<Organism>,
    pub id_index: HashMap<u32, usize>,
    pub next_organism_id: u32,

    /// 食物索引：food_cells 存扁平索引，food_pos 存反向位置（O(1) 增删）
    pub food_cells: Vec<usize>,
    pub food_pos: Vec<i32>,

    /// 带 tick 行为的地形索引（目前只有腐殖质）
    pub terrain: HashMap<u8, TerrainSet>,

    pub species: SpeciesRegistry,

    /// 阶段 B 的随机结算顺序（复用，避免每 tick 分配）
    order: Vec<usize>,
}

impl World {
    pub fn new(config: Config, rng: Rng) -> Self {
        let grid = Grid::new(config.world.cols, config.world.rows, config.world.wrap_edges);
        let mut w = World {
            grid,
            tick: 0,
            organisms: Vec::new(),
            id_index: HashMap::new(),
            next_organism_id: 1,
            food_cells: Vec::new(),
            food_pos: Vec::new(),
            terrain: HashMap::new(),
            species: SpeciesRegistry::default(),
            order: Vec::new(),
            config,
            rng,
        };
        w.init_food_index();
        w.init_terrain_index();
        w
    }

    // ────────────────────────────── 食物索引 ──────────────────────────────

    pub fn init_food_index(&mut self) {
        let n = self.grid.len;
        self.food_pos = vec![-1; n];
        self.food_cells.clear();
        for i in 0..n {
            if self.grid.types[i] == ct::FOOD {
                self.food_pos[i] = self.food_cells.len() as i32;
                self.food_cells.push(i);
            }
        }
    }

    fn add_food(&mut self, index: usize) {
        if self.food_pos[index] >= 0 {
            return;
        }
        self.food_pos[index] = self.food_cells.len() as i32;
        self.food_cells.push(index);
    }

    fn remove_food(&mut self, index: usize) {
        let p = self.food_pos[index];
        if p < 0 {
            return;
        }
        let p = p as usize;
        let last = self.food_cells.pop().unwrap();
        if p < self.food_cells.len() {
            self.food_cells[p] = last;
            self.food_pos[last] = p as i32;
        }
        self.food_pos[index] = -1;
    }

    // ────────────────────────────── 地形索引 ──────────────────────────────

    pub fn init_terrain_index(&mut self) {
        let n = self.grid.len;
        self.terrain.clear();
        for &t in TICKING_TERRAIN_TYPES.iter() {
            self.terrain.insert(
                t,
                TerrainSet {
                    cells: Vec::new(),
                    pos: vec![-1; n],
                },
            );
        }
        for i in 0..n {
            let f = self.grid.floor[i];
            if TICKS_TERRAIN[f as usize] {
                self.add_terrain_cell(i, f);
            }
        }
    }

    fn add_terrain_cell(&mut self, index: usize, ty: u8) {
        if let Some(set) = self.terrain.get_mut(&ty) {
            if set.pos[index] >= 0 {
                return;
            }
            set.pos[index] = set.cells.len() as i32;
            set.cells.push(index);
        }
    }

    fn remove_terrain_cell(&mut self, index: usize, ty: u8) {
        if let Some(set) = self.terrain.get_mut(&ty) {
            let p = set.pos[index];
            if p < 0 {
                return;
            }
            let p = p as usize;
            let last = set.cells.pop().unwrap();
            if p < set.cells.len() {
                set.cells[p] = last;
                set.pos[last] = p as i32;
            }
            set.pos[index] = -1;
        }
    }

    fn retag_terrain(&mut self, index: usize, old_ty: u8, new_ty: u8) {
        if TICKS_TERRAIN[old_ty as usize] {
            self.remove_terrain_cell(index, old_ty);
        }
        if TICKS_TERRAIN[new_ty as usize] {
            self.add_terrain_cell(index, new_ty);
        }
    }

    // ────────────────────────────── 格子读写 ──────────────────────────────

    /// 唯一的写入口：同时维护地板层、食物索引、地形索引。
    pub fn set_cell_at(&mut self, index: usize, ty: u8, owner: i32) {
        if index >= self.grid.len {
            return;
        }
        if self.grid.types[index] == ty && self.grid.owners[index] == owner {
            return;
        }

        let was_food = self.grid.types[index] == ct::FOOD;
        self.grid.types[index] = ty;
        self.grid.owners[index] = owner;

        if TERRAIN_MASK[ty as usize] {
            if self.grid.floor[index] != ty {
                let old = self.grid.floor[index];
                self.grid.floor[index] = ty;
                self.retag_terrain(index, old, ty);
            }
        } else if ty == ct::EMPTY && owner < 0 && self.grid.floor[index] != ct::EMPTY {
            let old = self.grid.floor[index];
            self.grid.floor[index] = ct::EMPTY;
            self.retag_terrain(index, old, ct::EMPTY);
        }

        if ty == ct::FOOD {
            if !was_food {
                self.add_food(index);
            }
        } else if was_food {
            self.remove_food(index);
        }
    }

    pub fn set_cell(&mut self, col: i32, row: i32, ty: u8, owner: i32) -> Option<usize> {
        let index = self.grid.index(col, row)?;
        self.set_cell_at(index, ty, owner);
        Some(index)
    }

    /// 把地板重新铺回顶层（生物离开 / 食物被吃掉时用）
    pub fn reveal_floor(&mut self, index: usize) {
        if index >= self.grid.len {
            return;
        }
        let ty = self.grid.floor[index];
        self.set_cell_at(index, ty, -1);
    }

    pub fn floor_at(&self, col: i32, row: i32) -> u8 {
        match self.grid.index(col, row) {
            Some(i) => self.grid.floor[i],
            None => ct::EMPTY,
        }
    }

    pub fn type_at(&self, col: i32, row: i32) -> u8 {
        self.grid.type_at(col, row)
    }

    pub fn fill_grid(&mut self, ty: u8, keep_terrain: bool) {
        self.grid.fill(ty, keep_terrain);
        self.init_food_index();
        self.init_terrain_index();
    }

    // ────────────────────────────── 生物集合 ──────────────────────────────

    pub fn add_organism(&mut self, mut org: Organism) -> u32 {
        let id = self.next_organism_id;
        self.next_organism_id += 1;
        org.id = id;
        if org.species_fingerprint.is_none() {
            org.species_fingerprint = Some(Arc::from(org.anatomy.fingerprint().as_str()));
        }
        let idx = self.organisms.len();
        self.organisms.push(org);
        self.id_index.insert(id, idx);
        self.write_grid(idx);

        let fp = self.organisms[idx].species_fingerprint.clone().unwrap();
        let parent_fp = self.organisms[idx].parent_fingerprint.clone();
        let anatomy_desc_fp = fp.clone();
        let _ = anatomy_desc_fp;
        let tick = self.tick;
        // 需要 anatomy 的描述与细胞数：先算好再登记
        let desc_anatomy = self.organisms[idx].anatomy.clone();
        self.species
            .observe(&fp, parent_fp.as_ref(), &desc_anatomy, tick);
        id
    }

    /// 直接收编一个已构造好的个体，**不登记物种**（读档用）
    pub fn adopt_organism(&mut self, mut org: Organism) -> u32 {
        let id = self.next_organism_id;
        self.next_organism_id += 1;
        org.id = id;
        if org.species_fingerprint.is_none() {
            org.species_fingerprint = Some(Arc::from(org.anatomy.fingerprint().as_str()));
        }
        let idx = self.organisms.len();
        self.organisms.push(org);
        self.id_index.insert(id, idx);
        self.write_grid(idx);
        id
    }

    pub fn organism_index(&self, id: u32) -> Option<usize> {
        self.id_index.get(&id).copied()
    }

    pub fn organism(&self, id: u32) -> Option<&Organism> {
        self.organism_index(id).map(|i| &self.organisms[i])
    }

    pub fn can_add_organism(&self) -> bool {
        let max = self.config.life.max_organisms;
        max < 0 || (self.organisms.len() as i64) < max
    }

    pub fn clear_organisms(&mut self) {
        for i in 0..self.organisms.len() {
            self.kill_organism(i);
        }
        self.organisms.clear();
        self.id_index.clear();
    }

    /// 压缩掉已死亡的个体。
    ///
    /// **必须稳定**（保持原有相对顺序）：阶段 B 的结算顺序是"对数组做一次洗牌"，
    /// 数组顺序一变，洗牌出来的访问序列就跟着变，黄金快照立刻对不上。
    fn compact_organisms(&mut self) {
        self.organisms.retain(|o| o.alive);
        self.id_index.clear();
        for (i, org) in self.organisms.iter().enumerate() {
            self.id_index.insert(org.id, i);
        }
    }

    // ────────────────────────────── 网格写入 ──────────────────────────────

    pub fn write_grid(&mut self, i: usize) {
        let n = self.organisms[i].anatomy.cells.len();
        for k in 0..n {
            let cell = self.organisms[i].anatomy.cells[k];
            let col = self.organisms[i].real_col(&cell);
            let row = self.organisms[i].real_row(&cell);
            if let Some(index) = self.grid.index(col, row) {
                let id = self.organisms[i].id as i32;
                self.set_cell_at(index, cell.type_id, id);
            }
        }
    }

    fn clear_grid(&mut self, i: usize) {
        let n = self.organisms[i].anatomy.cells.len();
        for k in 0..n {
            let cell = self.organisms[i].anatomy.cells[k];
            let col = self.organisms[i].real_col(&cell);
            let row = self.organisms[i].real_row(&cell);
            if let Some(index) = self.grid.index(col, row) {
                self.reveal_floor(index);
            }
        }
    }

    // ────────────────────────────── 环境规则 ──────────────────────────────

    pub fn apply_food_decay(&mut self) {
        let rate = self.config.world.food_decay_rate;
        if rate <= 0.0 {
            return;
        }
        let expected = (self.food_cells.len() as f64 * rate) / 100.0;
        if expected <= 0.0 {
            return;
        }
        let mut n = expected.floor() as usize;
        if self.rng.float() < expected - n as f64 {
            n += 1;
        }
        for _ in 0..n {
            if self.food_cells.is_empty() {
                break;
            }
            let j = self.rng.int(self.food_cells.len());
            let index = self.food_cells[j];
            self.reveal_floor(index);
        }
    }

    pub fn apply_terrain_tick(&mut self) {
        for &ty in TICKING_TERRAIN_TYPES.iter() {
            let len = self.terrain.get(&ty).map(|s| s.cells.len()).unwrap_or(0);
            for k in 0..len {
                let index = match self.terrain.get(&ty) {
                    Some(set) => match set.cells.get(k) {
                        Some(&v) => v,
                        None => break,
                    },
                    None => break,
                };
                self.run_terrain_tick(ty, index);
            }
        }
    }

    fn run_terrain_tick(&mut self, ty: u8, index: usize) {
        let fx = match cell_type::def(ty).and_then(|d| d.tick) {
            Some(f) => f,
            None => return,
        };
        match fx {
            cell_type::TickFx::Fertilize => {
                let prob = self.config.terrain.humus_food_prob;
                if prob <= 0.0 {
                    return;
                }
                if !self.rng.chance(prob) {
                    return;
                }
                let (col, row) = self.grid.coords(index);
                let n = NEIGHBORS_4[self.rng.int(4)];
                if let Some(target) = self.grid.index(col + n.0, row + n.1) {
                    if self.grid.types[target] == ct::EMPTY {
                        self.set_cell_at(target, ct::FOOD, -1);
                    }
                }
            }
        }
    }

    pub fn apply_food_drop(&mut self) {
        let prob = self.config.world.food_drop_prob;
        if prob <= 0.0 {
            return;
        }
        let area = self.grid.len as f64;
        let count = ((area * prob) / 50000.0).floor().max(1.0) as usize;
        for _ in 0..count {
            if !self.rng.chance(prob) {
                continue;
            }
            let col = self.rng.int(self.grid.cols) as i32;
            let row = self.rng.int(self.grid.rows) as i32;
            if let Some(index) = self.grid.index(col, row) {
                if self.grid.types[index] == ct::EMPTY {
                    self.set_cell_at(index, ct::FOOD, -1);
                }
            }
        }
    }

    // ────────────────────────────── 主循环 ──────────────────────────────

    pub fn step(&mut self) {
        self.tick += 1;

        // ── 阶段 A：全体感知与决策（只读世界）──
        //
        // 这一阶段**只读世界**、只写个体自己的 plan/缓冲，所以可以安全并行。
        // 关键：`think()` 里不消耗 RNG（已确认），因此并行不会改变随机序列，
        // 黄金快照依然逐位一致。
        self.think_all();

        // ── 阶段 B：随机顺序结算（写世界）──
        self.order.clear();
        self.order.extend(0..self.organisms.len());
        let mut order = std::mem::take(&mut self.order);
        self.rng.shuffle(&mut order);

        for &i in order.iter() {
            if i < self.organisms.len() && self.organisms[i].alive {
                self.act_organism(i);
            }
        }
        self.order = order;

        self.compact_organisms();

        // ── 环境更新 ──
        self.apply_terrain_tick();
        self.apply_food_decay();
        self.apply_food_drop();

        // ── 物种统计 ──
        //
        // 1.x 是在统计采样时调 `species.update()` 的；2.0 漏了这一步，
        // 结果所有物种的 population 恒为 0 —— 物种面板是空的、存活/峰值/灭绝全是 0。
        // 这里按 statsInterval 重算一次（不消耗 RNG、不写网格，所以不影响黄金快照）。
        if self.config.stats.interval > 0 && self.tick % self.config.stats.interval == 0 {
            let World {
                organisms,
                species,
                tick,
                config,
                ..
            } = self;
            let fps: Vec<std::sync::Arc<str>> = organisms
                .iter()
                .filter_map(|o| o.species_fingerprint.clone())
                .collect();
            species.update(fps.iter(), *tick);
            if species.list.len() > crate::species::MAX_SPECIES {
                species.prune(crate::species::MAX_SPECIES, config.stats.min_discard);
            }
        }
    }

    /// 阶段 A：全体感知 + 决策。
    ///
    /// 拆字段借用后用 rayon 并行——这一阶段占满载开销的七成左右，
    /// 且完全不碰 RNG，所以并行既安全又不破坏可复现性。
    pub fn think_all(&mut self) {
        use rayon::prelude::*;
        let World {
            grid,
            config,
            organisms,
            ..
        } = self;
        organisms
            .par_iter_mut()
            .filter(|o| o.alive)
            .for_each(|o| o.think(grid, config));
    }

    /// 单个体版（测试与串行对照用）
    pub fn think_organism(&mut self, i: usize) {
        let World {
            grid,
            config,
            organisms,
            ..
        } = self;
        organisms[i].think(grid, config);
    }

    /// 阶段 B：结算
    pub fn act_organism(&mut self, i: usize) {
        // 年龄
        self.organisms[i].age += 1;
        let lifespan = self.organisms[i].lifespan(&self.config);
        if self.organisms[i].age as f64 > lifespan {
            self.kill_organism(i);
            return;
        }

        self.organisms[i].in_radiation = false;

        // 代谢
        let upkeep = self.organisms[i].upkeep_cost(&self.config);
        if upkeep > 0.0 {
            let limit = self.config.life.starvation_ticks;
            let mut starved = false;
            {
                let org = &mut self.organisms[i];
                org.energy -= upkeep;
                if org.energy <= 0.0 {
                    org.energy = 0.0;
                    org.starve_ticks += 1;
                    if org.starve_ticks >= limit {
                        starved = true;
                    }
                } else {
                    org.starve_ticks = 0;
                }
            }
            if starved {
                self.kill_organism(i);
                return;
            }
        }

        if self.organisms[i].anatomy.has(Flags::MOVER) {
            self.act_move(i);
        }
        if !self.organisms[i].alive {
            return;
        }

        // 驻留效果（移动之后）
        self.apply_terrain_stand(i);
        if !self.organisms[i].alive {
            return;
        }

        // 细胞功能
        let n = self.organisms[i].anatomy.cells.len();
        for k in 0..n {
            let cell = self.organisms[i].anatomy.cells[k];
            self.run_cell_function(i, cell);
            if !self.organisms[i].alive {
                return;
            }
        }

        // 繁殖
        let at = self.organisms[i].reproduce_at(&self.config);
        if self.organisms[i].energy >= at {
            self.reproduce_organism(i);
        }
    }

    fn act_move(&mut self, i: usize) {
        if self.organisms[i].turn_cooldown > 0 {
            self.organisms[i].turn_cooldown -= 1;
        }

        if self.organisms[i].plan_turn {
            if self.attempt_rotate(i) {
                self.organisms[i].turn_cooldown = self.config.life.turn_cooldown;
                return;
            }
            let d = self.organisms[i].plan_direction;
            self.organisms[i].direction = d & 3;
            return;
        }

        let d = self.organisms[i].plan_direction;
        self.organisms[i].direction = d & 3;
        if self.attempt_move(i) {
            return;
        }

        if self.attempt_rotate(i) {
            self.organisms[i].turn_cooldown = self.config.life.turn_cooldown;
        } else {
            let d = self.organisms[i].plan_direction;
            self.organisms[i].direction = d & 3;
        }
    }

    fn attempt_move(&mut self, i: usize) -> bool {
        let (nc, nr) = self.organisms[i].step_target();
        let rot = self.organisms[i].rotation;
        if !self.organisms[i].is_clear(&self.grid, &self.config, nc, nr, rot) {
            return false;
        }

        // 目的地的地形要在 write_grid 之前读（write_grid 会盖住顶层）
        let dest_type = match self.grid.index(nc, nr) {
            Some(index) => self.grid.floor[index],
            None => ct::EMPTY,
        };

        self.clear_grid(i);
        self.organisms[i].col = nc;
        self.organisms[i].row = nr;
        self.write_grid(i);
        {
            let grid = &self.grid;
            self.organisms[i].normalize_position(grid);
        }

        if dest_type != ct::EMPTY {
            self.apply_terrain_enter(i, dest_type);
        }
        true
    }

    fn attempt_rotate(&mut self, i: usize) -> bool {
        let new_rot = self.rng.int(4) as u8;
        let (c, r) = (self.organisms[i].col, self.organisms[i].row);
        if !self.organisms[i].is_clear(&self.grid, &self.config, c, r, new_rot) {
            return false;
        }
        self.clear_grid(i);
        self.organisms[i].rotation = new_rot;
        self.organisms[i].direction = self.rng.int(4) as u8;
        self.write_grid(i);
        true
    }

    // ────────────────────────────── 地形效果 ──────────────────────────────

    fn apply_terrain_enter(&mut self, i: usize, ty: u8) {
        let fx = match cell_type::def(ty).and_then(|d| d.enter) {
            Some(f) => f,
            None => return,
        };
        match fx {
            EnterFx::Burn => {
                let dmg = self.config.terrain.lava_enter_damage;
                if dmg > 0.0 {
                    let id = self.organisms[i].id;
                    self.harm_organism(id, dmg, None);
                }
            }
            EnterFx::Bog => {
                let cost = self.config.terrain.swamp_move_cost;
                if cost > 0.0 {
                    let org = &mut self.organisms[i];
                    org.energy -= cost;
                    if org.energy < 0.0 {
                        org.energy = 0.0;
                    }
                }
            }
        }
    }

    fn apply_terrain_stand(&mut self, i: usize) {
        let index = match self.grid.index(self.organisms[i].col, self.organisms[i].row) {
            Some(x) => x,
            None => return,
        };
        let ty = self.grid.floor[index];
        if ty == ct::EMPTY {
            return;
        }
        let fx = match cell_type::def(ty).and_then(|d| d.stand) {
            Some(f) => f,
            None => return,
        };
        match fx {
            StandFx::Burn => {
                let dmg = self.config.terrain.lava_stand_damage;
                if dmg > 0.0 {
                    let id = self.organisms[i].id;
                    self.harm_organism(id, dmg, None);
                }
            }
            StandFx::Irradiate => {
                let dmg = self.config.terrain.rad_damage;
                if dmg > 0.0 {
                    let id = self.organisms[i].id;
                    self.harm_organism(id, dmg, None);
                }
                if self.organisms[i].alive {
                    self.organisms[i].in_radiation = true;
                }
            }
        }
    }

    // ────────────────────────────── 细胞功能 ──────────────────────────────

    fn run_cell_function(&mut self, i: usize, cell: BodyCell) {
        let behavior = match cell_type::def(cell.type_id).and_then(|d| d.behavior) {
            Some(b) => b,
            None => return,
        };
        match behavior {
            Behavior::Mouth => {
                let col = self.organisms[i].real_col(&cell);
                let row = self.organisms[i].real_row(&cell);
                let gain = self.config.food_energy_gain();
                for k in 0..4 {
                    let n = NEIGHBORS_4[k];
                    if let Some(index) = self.grid.index(col + n.0, row + n.1) {
                        if self.grid.types[index] == ct::FOOD {
                            self.reveal_floor(index);
                            self.organisms[i].energy += gain;
                        }
                    }
                }
            }
            Behavior::Producer => {
                if self.organisms[i].anatomy.has(Flags::MOVER)
                    && !self.config.life.movers_can_produce
                {
                    return;
                }
                let prob = self.config.life.food_prod_prob;
                if prob <= 0.0 {
                    return;
                }
                if !self.rng.chance(prob) {
                    return;
                }
                let col = self.organisms[i].real_col(&cell);
                let row = self.organisms[i].real_row(&cell);
                let n = NEIGHBORS_4[self.rng.int(4)];
                if let Some(index) = self.grid.index(col + n.0, row + n.1) {
                    if self.grid.types[index] == ct::EMPTY {
                        self.set_cell_at(index, ct::FOOD, -1);
                    }
                }
            }
            Behavior::Killer => {
                let damage = self.config.attr(cell.type_id, AttrField::Damage);
                if damage <= 0.0 {
                    return;
                }
                let col = self.organisms[i].real_col(&cell);
                let row = self.organisms[i].real_row(&cell);
                let my_id = self.organisms[i].id;
                for k in 0..4 {
                    let n = NEIGHBORS_4[k];
                    if let Some(index) = self.grid.index(col + n.0, row + n.1) {
                        let owner = self.grid.owners[index];
                        if owner < 0 || owner == my_id as i32 {
                            continue;
                        }
                        if self.grid.types[index] == ct::ARMOR {
                            continue; // 装甲格打不动
                        }
                        self.harm_organism(owner as u32, damage, Some(my_id));
                    }
                }
            }
            Behavior::Regen => {
                if self.organisms[i].damage <= 0.0 {
                    return;
                }
                let rate = self.config.life.regen_rate;
                if !(rate > 0.0) {
                    return;
                }
                let org = &mut self.organisms[i];
                org.damage -= rate;
                if org.damage < 0.0 {
                    org.damage = 0.0;
                }
            }
            Behavior::Eye => {}
        }
    }

    // ────────────────────────────── 伤害与死亡 ──────────────────────────────

    pub fn harm_organism(&mut self, target_id: u32, amount: f64, attacker_id: Option<u32>) {
        let ti = match self.organism_index(target_id) {
            Some(i) => i,
            None => return,
        };
        if !self.organisms[ti].alive {
            return;
        }
        self.organisms[ti].damage += amount;

        if let Some(ai) = attacker_id {
            if ai != target_id {
                let attacker_alive = self
                    .organism_index(ai)
                    .map(|k| self.organisms[k].alive)
                    .unwrap_or(false);
                if attacker_alive {
                    let back = self.organisms[ti].spike_damage(&self.config);
                    if back > 0.0 {
                        // 不带 attacker → 不再连锁反伤（递归深度恒为 1）
                        self.harm_organism(ai, back, None);
                    }
                }
            }
        }

        let insta = self.config.combat.insta_kill;
        let hp = self.organisms[ti].max_health(&self.config);
        if insta || self.organisms[ti].damage >= hp {
            self.kill_organism(ti);
        }
    }

    pub fn kill_organism(&mut self, i: usize) {
        if i >= self.organisms.len() || !self.organisms[i].alive {
            return;
        }
        self.organisms[i].alive = false;

        let n = self.organisms[i].anatomy.cells.len();
        for k in 0..n {
            let cell = self.organisms[i].anatomy.cells[k];
            let col = self.organisms[i].real_col(&cell);
            let row = self.organisms[i].real_row(&cell);
            if let Some(index) = self.grid.index(col, row) {
                self.set_cell_at(index, ct::FOOD, -1);
            }
        }

        let total = self.organisms[i].food_value_total(&self.config);
        let extra = total.round() as i64 - n as i64;
        if extra > 0 {
            let (col, row) = (self.organisms[i].col, self.organisms[i].row);
            self.drop_extra_food(col, row, extra as usize);
        }
    }

    /// 在 (col,row) 周围的空格里投放 `count` 份额外食物
    fn drop_extra_food(&mut self, col: i32, row: i32, count: usize) {
        let max_r = self.grid.cols.max(self.grid.rows) as i32;
        let mut placed = 0usize;
        let mut r = 1i32;
        while r <= max_r && placed < count {
            let mut dy = -r;
            while dy <= r && placed < count {
                let mut dx = -r;
                while dx <= r && placed < count {
                    if dx.abs().max(dy.abs()) != r {
                        dx += 1;
                        continue;
                    }
                    if let Some(index) = self.grid.index(col + dx, row + dy) {
                        if self.grid.types[index] == ct::EMPTY {
                            self.set_cell_at(index, ct::FOOD, -1);
                            placed += 1;
                        }
                    }
                    dx += 1;
                }
                dy += 1;
            }
            r += 1;
        }
    }

    // ────────────────────────────── 繁殖 ──────────────────────────────

    fn reproduce_organism(&mut self, i: usize) -> bool {
        let need = self.organisms[i].food_needed(&self.config);
        {
            let org = &mut self.organisms[i];
            org.energy -= need;
            if org.energy < 0.0 {
                org.energy = 0.0;
            }
        }

        if !self.can_add_organism() {
            return false;
        }

        let v = DIR_VECTORS[self.rng.int(4)];
        let distance = self.organisms[i].anatomy.birth_distance + self.rng.int(3) as i32;
        let new_col = self.organisms[i].col + v.0 * distance;
        let new_row = self.organisms[i].row + v.1 * distance;
        let rotation = self.rng.int(4) as u8;

        // 第一道预检用母体蓝图（便宜）
        if !self
            .organisms[i]
            .is_clear(&self.grid, &self.config, new_col, new_row, rotation)
        {
            return false;
        }
        let (pc, pr, pid) = (
            self.organisms[i].col,
            self.organisms[i].row,
            self.organisms[i].id,
        );
        if !self
            .organisms[i]
            .is_straight_path(&self.grid, new_col, new_row, pc, pr, pid)
        {
            return false;
        }

        let mut child = Organism::new(&self.config, &mut self.rng, &self.grid, new_col, new_row);
        child.anatomy.copy_from(&self.organisms[i].anatomy);
        child.brain.copy_from(&self.organisms[i].brain);
        child.rotation = rotation;
        child.direction = self.rng.int(4) as u8;

        {
            let parent = &self.organisms[i];
            let cfg = &self.config;
            mutate_organism(&mut child, parent, &mut self.rng, cfg);
        }

        if !child.can_place_at(
            &self.grid,
            &self.config,
            new_col,
            new_row,
            rotation,
            &child.anatomy,
        ) {
            return false;
        }

        self.add_organism(child);
        true
    }

    // ────────────────────────────── 重置 ──────────────────────────────

    pub fn reset(&mut self, keep_terrain: bool) {
        self.clear_organisms();
        self.grid.wrap = self.config.world.wrap_edges;
        self.grid.fill(ct::EMPTY, keep_terrain);
        self.init_food_index();
        self.init_terrain_index();
        self.species = SpeciesRegistry::default();
        self.next_organism_id = 1;
        self.tick = 0;
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.config.world.cols = cols;
        self.config.world.rows = rows;
        self.organisms.clear();
        self.id_index.clear();
        self.grid = Grid::new(cols, rows, self.config.world.wrap_edges);
        self.init_food_index();
        self.init_terrain_index();
        self.species = SpeciesRegistry::default();
        self.next_organism_id = 1;
        self.tick = 0;
    }

    // ────────────────────────────── 统计 ──────────────────────────────

    pub fn snapshot_stats(&self) -> WorldStats {
        let counts = self.grid.count_by_type();
        let food = self.food_cells.len();
        let mut body = 0usize;
        for &t in cell_type::LIVING_CELL_TYPES.iter() {
            body += counts[t as usize] as usize;
        }
        WorldStats {
            tick: self.tick,
            cols: self.grid.cols,
            rows: self.grid.rows,
            cells: self.grid.len,
            non_empty: food + counts[ct::WALL as usize] as usize + body,
            food,
            wall: counts[ct::WALL as usize] as usize,
            body,
            organisms: self.organisms.len(),
        }
    }

    pub fn organism_stats(&self) -> OrgStats {
        let n = self.organisms.len();
        if n == 0 {
            return OrgStats::default();
        }
        let mut cells = 0usize;
        let mut max_cells = 0usize;
        let (mut movers, mut eyes, mut killers) = (0usize, 0usize, 0usize);
        let mut age = 0f64;
        let mut mutability = 0f64;
        for org in &self.organisms {
            let c = org.cell_count();
            cells += c;
            if c > max_cells {
                max_cells = c;
            }
            if org.anatomy.has(Flags::MOVER) {
                movers += 1;
            }
            if org.anatomy.has(Flags::EYE) {
                eyes += 1;
            }
            if org.anatomy.has(Flags::KILLER) {
                killers += 1;
            }
            age += org.age as f64;
            mutability += org.mutability;
        }
        OrgStats {
            count: n,
            avg_cells: cells as f64 / n as f64,
            max_cells,
            movers,
            eyes,
            killers,
            avg_age: age / n as f64,
            avg_mutability: mutability / n as f64,
        }
    }

    /// 状态的稳定指纹（黄金快照对照用）：
    /// 网格类型 + 归属 + 地形 + 每个个体的关键字段
    pub fn state_digest(&self) -> String {
        let mut h: u32 = 0x811c_9dc5;
        let mut mix = |v: u32| {
            h ^= v;
            h = h.wrapping_mul(0x0100_0193);
        };
        for i in 0..self.grid.len {
            mix(self.grid.types[i] as u32);
            mix(self.grid.owners[i] as u32);
            mix(self.grid.floor[i] as u32);
        }
        for org in &self.organisms {
            mix(org.id);
            mix(org.col as u32);
            mix(org.row as u32);
            mix(org.rotation as u32);
            mix(org.direction as u32);
            mix(org.age as u32);
            mix((org.energy * 1000.0).round() as i32 as u32);
            mix((org.damage as i32) as u32);
            mix(org.anatomy.cells.len() as u32);
            for c in &org.anatomy.cells {
                mix(c.type_id as u32);
                mix(c.dx as u32);
                mix(c.dy as u32);
                mix(c.dir as u32);
            }
        }
        format!("{:08x}", h)
    }
}

#[derive(Debug, Default, Clone)]
pub struct WorldStats {
    pub tick: u64,
    pub cols: usize,
    pub rows: usize,
    pub cells: usize,
    pub non_empty: usize,
    pub food: usize,
    pub wall: usize,
    pub body: usize,
    pub organisms: usize,
}

#[derive(Debug, Default, Clone)]
pub struct OrgStats {
    pub count: usize,
    pub avg_cells: f64,
    pub max_cells: usize,
    pub movers: usize,
    pub eyes: usize,
    pub killers: usize,
    pub avg_age: f64,
    pub avg_mutability: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::factory::{seed_origin_of_life, spawn_population};
    use crate::worldgen::generate_demo_world;

    /// 回归：物种统计必须被 step 更新
    ///
    /// 曾经的 bug：`species.update()` 从来没被调用，于是 population 恒为 0 ——
    /// 物种面板空、存活/峰值/灭绝全是 0。
    #[test]
    fn species_stats_are_updated_by_step() {
        let mut w = World::new(Config::default(), Rng::new(1001));
        generate_demo_world(&mut w, 1.0, 1.0);
        seed_origin_of_life(&mut w);
        spawn_population(&mut w, 8, 3);

        for _ in 0..100 {
            w.step();
        }

        assert!(w.species.alive_count > 0, "存活物种数应大于 0");
        assert!(w.species.peak_alive_count > 0, "峰值存活应大于 0");

        let total: u32 = w.species.list.iter().map(|s| s.population).sum();
        assert_eq!(
            total as usize,
            w.organisms.len(),
            "各物种个体数之和应等于种群数"
        );

        // 每个存活物种的 population 都应该 > 0
        let alive = w.species.list.iter().filter(|s| s.population > 0).count();
        assert_eq!(alive, w.species.alive_count, "存活计数应与实际相符");
    }
}
