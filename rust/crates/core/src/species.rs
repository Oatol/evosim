//! 物种与化石记录。与 JS 版 `src/stats/Species.js` 对应。
//!
//! 物种判据是**身体结构指纹**：只有结构变异才产生新物种，权重变异不算。

use serde::{Deserialize, Serialize};

use std::collections::HashMap;
use std::sync::Arc;

use crate::anatomy::Anatomy;

pub const MAX_SPECIES: usize = 4000;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Species {
    pub id: u32,
    pub fingerprint: Arc<str>,
    pub name: String,
    pub description: String,
    pub cell_count: usize,
    pub ancestor_id: Option<u32>,
    pub start_tick: u64,
    pub end_tick: i64,
    pub population: u32,
    pub peak_population: u32,
    pub cumulative_pop: u32,
    pub alive: bool,
    pub last_seen_tick: u64,
}

impl Species {
    pub fn lifespan(&self) -> u64 {
        let end = if self.end_tick >= 0 {
            self.end_tick as u64
        } else {
            self.last_seen_tick
        };
        end.saturating_sub(self.start_tick)
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SpeciesRegistry {
    pub map: HashMap<Arc<str>, usize>,
    pub list: Vec<Species>,
    pub next_id: u32,
    pub alive_count: usize,
    pub peak_alive_count: usize,
    pub total_created: u32,
    pub total_extinct: u32,
}

impl Default for SpeciesRegistry {
    fn default() -> Self {
        SpeciesRegistry {
            map: HashMap::new(),
            list: Vec::new(),
            next_id: 1,
            alive_count: 0,
            peak_alive_count: 0,
            total_created: 0,
            total_extinct: 0,
        }
    }
}

impl SpeciesRegistry {
    pub fn size(&self) -> usize {
        self.list.len()
    }

    pub fn by_fingerprint(&self, fp: &str) -> Option<&Species> {
        self.map.get(fp).map(|&i| &self.list[i])
    }

    /// 登记一个个体（繁殖与投放都走这里）
    pub fn observe(
        &mut self,
        fingerprint: &Arc<str>,
        parent_fingerprint: Option<&Arc<str>>,
        anatomy: &Anatomy,
        tick: u64,
    ) -> usize {
        if let Some(&idx) = self.map.get(fingerprint) {
            let sp = &mut self.list[idx];
            sp.cumulative_pop += 1;
            sp.last_seen_tick = tick;
            return idx;
        }

        let ancestor = parent_fingerprint
            .and_then(|fp| self.map.get(fp).copied())
            .map(|i| self.list[i].id);

        let id = self.next_id;
        self.next_id += 1;
        let sp = Species {
            id,
            fingerprint: fingerprint.clone(),
            name: format!("S{}", id),
            description: anatomy.describe(),
            cell_count: anatomy.cell_count(),
            ancestor_id: ancestor,
            start_tick: tick,
            end_tick: -1,
            population: 0,
            peak_population: 0,
            cumulative_pop: 1,
            alive: true,
            last_seen_tick: tick,
        };
        self.list.push(sp);
        self.map.insert(fingerprint.clone(), self.list.len() - 1);
        self.total_created += 1;
        self.list.len() - 1
    }

    /// 重新统计每个物种的当前个体数并标记灭绝（按 statsInterval 调用）
    pub fn update<'a, I>(&mut self, fingerprints: I, tick: u64) -> usize
    where
        I: Iterator<Item = &'a Arc<str>>,
    {
        for sp in self.list.iter_mut() {
            sp.population = 0;
        }
        for fp in fingerprints {
            if let Some(&i) = self.map.get(fp.as_ref() as &str) {
                self.list[i].population += 1;
            }
        }

        let mut alive = 0;
        for sp in self.list.iter_mut() {
            if sp.population > 0 {
                sp.alive = true;
                sp.last_seen_tick = tick;
                if sp.population > sp.peak_population {
                    sp.peak_population = sp.population;
                }
                alive += 1;
            } else if sp.alive {
                sp.alive = false;
                sp.end_tick = tick as i64;
                self.total_extinct += 1;
            }
        }
        self.alive_count = alive;
        if alive > self.peak_alive_count {
            self.peak_alive_count = alive;
        }
        alive
    }

    /// 剪枝：物种数超上限时丢弃一批（先丢"灭绝且累计少"的噪声）
    pub fn prune(&mut self, max: usize, min_discard: u32) -> usize {
        let over = self.list.len().saturating_sub(max);
        if over == 0 {
            return 0;
        }
        let mut removed = 0;
        let mut kept: Vec<Species> = Vec::with_capacity(self.list.len());
        for sp in self.list.drain(..) {
            if removed < over && !sp.alive && sp.cumulative_pop < min_discard {
                self.map.remove(&sp.fingerprint);
                removed += 1;
            } else {
                kept.push(sp);
            }
        }
        self.list = kept;

        if removed < over {
            let mut kept2 = Vec::with_capacity(self.list.len());
            for sp in self.list.drain(..) {
                if removed < over && !sp.alive {
                    self.map.remove(&sp.fingerprint);
                    removed += 1;
                } else {
                    kept2.push(sp);
                }
            }
            self.list = kept2;
        }

        // 索引重排
        self.map.clear();
        for (i, sp) in self.list.iter().enumerate() {
            self.map.insert(sp.fingerprint.clone(), i);
        }
        removed
    }
}

#[cfg(test)]
mod timeline_tests {
    use super::*;
    use crate::cell_type::ct;

    /// 物种时间轴要能画出"存续区间"：活着的 end_tick = -1，灭绝后记下真实的结束 tick。
    ///
    /// 物种面板的横条就是靠 `start_tick` / `end_tick` 画的——这两个字段错了，
    /// 时间轴会变成一堆零宽条或者全顶到最右。
    #[test]
    fn extinct_species_get_an_end_tick() {
        let mut reg = SpeciesRegistry::default();
        let fp = Arc::<str>::from("fp-a");
        let mut anatomy = Anatomy::default();
        anatomy.add_cell(ct::MOUTH, 0, 0, 0);
        anatomy.recompute();

        reg.observe(&fp, None, &anatomy, 10);
        assert_eq!(reg.list[0].start_tick, 10);
        assert_eq!(reg.list[0].end_tick, -1, "刚登记的物种还没结束");

        // 有一个个体 → 存活
        reg.update([&fp].into_iter(), 20);
        assert_eq!(reg.list[0].population, 1);
        assert!(reg.list[0].alive);
        assert_eq!(reg.list[0].end_tick, -1);

        // 一个都没有 → 灭绝，记下 end_tick
        reg.update(std::iter::empty::<&Arc<str>>(), 30);
        let sp = &reg.list[0];
        assert!(!sp.alive, "没有个体了应该标成灭绝");
        assert_eq!(sp.end_tick, 30, "灭绝 tick 必须记下来，否则时间轴画不出右端");
        assert!(sp.lifespan() >= 20, "存续时长 = 结束 - 开始，实际 {}", sp.lifespan());
    }

    /// 峰值存活、累计、已灭绝三个计数要各自独立变化
    #[test]
    fn counters_track_separately() {
        let mut reg = SpeciesRegistry::default();
        let mut anatomy = Anatomy::default();
        anatomy.add_cell(ct::MOUTH, 0, 0, 0);
        anatomy.recompute();

        let a = Arc::<str>::from("fp-a");
        let b = Arc::<str>::from("fp-b");
        reg.observe(&a, None, &anatomy, 0);
        reg.observe(&b, Some(&a), &anatomy, 5);

        reg.update([&a, &b].into_iter(), 10);
        assert_eq!(reg.alive_count, 2);
        assert_eq!(reg.peak_alive_count, 2);

        reg.update([&a].into_iter(), 20);
        assert_eq!(reg.alive_count, 1);
        assert_eq!(reg.total_extinct, 1);
        assert_eq!(reg.peak_alive_count, 2, "峰值不能被后来下降的存活数覆盖");
        assert_eq!(reg.total_created, 2);
    }
}
