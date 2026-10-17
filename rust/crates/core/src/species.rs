//! 物种与化石记录。与 JS 版 `src/stats/Species.js` 对应。
//!
//! 物种判据是**身体结构指纹**：只有结构变异才产生新物种，权重变异不算。

use std::collections::HashMap;
use std::sync::Arc;

use crate::anatomy::Anatomy;

pub const MAX_SPECIES: usize = 4000;

#[derive(Clone, Debug)]
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
