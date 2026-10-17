//! 生物蓝图 —— 手绘的身体 + 可选的大脑权重，用于"工坊造物"与"生物库"。
//!
//! 与 1.x 的区别：2.0 把蓝图做成了**一等公民**（带序列化），
//! 因为存档格式要重做（用户已同意不兼容旧档），蓝图正好复用它。

use serde::{Deserialize, Serialize};

use crate::anatomy::{Anatomy, BodyCell};
use crate::cell_type::is_directional;
use crate::organism::Organism;
use crate::world::World;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CellSpec {
    pub t: u8,
    pub dx: i32,
    pub dy: i32,
    #[serde(default)]
    pub dir: u8,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Blueprint {
    #[serde(default)]
    pub name: String,
    pub cells: Vec<CellSpec>,
    /// 大脑权重（可空：手绘蓝图第一次投放时随机初始化）
    #[serde(default)]
    pub w1: Vec<f32>,
    #[serde(default)]
    pub b1: Vec<f32>,
    #[serde(default)]
    pub w2: Vec<f32>,
    #[serde(default)]
    pub b2: Vec<f32>,
}

impl Blueprint {
    pub fn from_anatomy(name: &str, anatomy: &Anatomy) -> Self {
        Blueprint {
            name: name.to_string(),
            cells: anatomy
                .cells
                .iter()
                .map(|c| CellSpec {
                    t: c.type_id,
                    dx: c.dx,
                    dy: c.dy,
                    dir: c.dir,
                })
                .collect(),
            ..Default::default()
        }
    }

    /// 从一个存活个体身上取蓝图（含大脑权重）
    pub fn from_organism(org: &Organism) -> Self {
        Blueprint {
            name: format!("个体 #{}", org.id),
            cells: org
                .anatomy
                .cells
                .iter()
                .map(|c| CellSpec {
                    t: c.type_id,
                    dx: c.dx,
                    dy: c.dy,
                    dir: c.dir,
                })
                .collect(),
            w1: org.brain.w1.clone(),
            b1: org.brain.b1.clone(),
            w2: org.brain.w2.clone(),
            b2: org.brain.b2.clone(),
        }
    }

    pub fn has_brain(&self) -> bool {
        !self.w1.is_empty() && !self.w2.is_empty()
    }

    pub fn cell_count(&self) -> usize {
        self.cells.len()
    }

    /// 可读的结构描述，例如 "嘴×1 生产者×2"
    pub fn describe(&self) -> String {
        let mut anatomy = Anatomy::default();
        for c in &self.cells {
            anatomy.add_cell(c.t, c.dx, c.dy, c.dir);
        }
        anatomy.describe()
    }

    pub fn fingerprint(&self) -> String {
        let mut anatomy = Anatomy::default();
        for c in &self.cells {
            anatomy.add_cell(c.t, c.dx, c.dy, c.dir);
        }
        anatomy.fingerprint()
    }
}

/// 按蓝图在世界里投放一个个体。
///
/// 蓝图带大脑就照抄权重（同物种加人口），不带就保留随机初始化的大脑
/// （相当于"这个身体第一次出现"）。
pub fn spawn_from_blueprint(
    world: &mut World,
    col: i32,
    row: i32,
    bp: &Blueprint,
) -> Option<u32> {
    if bp.cells.is_empty() {
        return None;
    }

    let mut org = Organism::new(&world.config, &mut world.rng, &world.grid, col, row);
    for c in &bp.cells {
        org.anatomy.add_cell(c.t, c.dx, c.dy, c.dir);
    }
    org.anatomy.recompute();

    if bp.has_brain() && bp.w1.len() == org.brain.w1.len() {
        org.brain.w1.copy_from_slice(&bp.w1);
        org.brain.b1.copy_from_slice(&bp.b1);
        org.brain.w2.copy_from_slice(&bp.w2);
        org.brain.b2.copy_from_slice(&bp.b2);
    }

    org.rotation = world.rng.int(4) as u8;
    org.direction = world.rng.int(4) as u8;

    if !org.is_clear(&world.grid, &world.config, col, row, org.rotation) {
        return None;
    }
    Some(world.add_organism(org))
}

/// 蓝图里的细胞是否带朝向（眼睛）——工坊画布用
pub fn spec_is_directional(t: u8) -> bool {
    is_directional(t)
}

/// 把蓝图转成解剖结构（预览/校验用）
pub fn blueprint_anatomy(bp: &Blueprint) -> Anatomy {
    let mut a = Anatomy::default();
    for c in &bp.cells {
        a.add_cell(c.t, c.dx, c.dy, c.dir);
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell_type::ct;
    use crate::config::Config;
    use crate::rng::Rng;

    fn world() -> World {
        let mut cfg = Config::default();
        cfg.world.cols = 32;
        cfg.world.rows = 32;
        World::new(cfg, Rng::new(1))
    }

    #[test]
    fn spawn_from_blueprint_creates_same_species() {
        let mut w = world();
        w.fill_grid(ct::EMPTY, false);

        let bp = Blueprint {
            name: "测试".into(),
            cells: vec![
                CellSpec { t: ct::MOUTH, dx: 0, dy: 0, dir: 0 },
                CellSpec { t: ct::PRODUCER, dx: 1, dy: 1, dir: 0 },
            ],
            ..Default::default()
        };

        let id = spawn_from_blueprint(&mut w, 10, 10, &bp).expect("应该投放成功");
        let org = w.organism(id).unwrap();
        assert_eq!(org.cell_count(), 2);
        assert_eq!(org.species_fingerprint.as_deref(), Some(bp.fingerprint().as_str()));
    }

    #[test]
    fn blueprint_without_brain_still_spawns() {
        let mut w = world();
        w.fill_grid(ct::EMPTY, false);
        let bp = Blueprint {
            name: "无脑".into(),
            cells: vec![CellSpec { t: ct::MOUTH, dx: 0, dy: 0, dir: 0 }],
            ..Default::default()
        };
        assert!(!bp.has_brain());
        assert!(spawn_from_blueprint(&mut w, 5, 5, &bp).is_some());
    }

    #[test]
    fn json_roundtrip() {
        let bp = Blueprint {
            name: "往返".into(),
            cells: vec![CellSpec { t: ct::EYE, dx: 0, dy: -1, dir: 2 }],
            ..Default::default()
        };
        let s = serde_json::to_string(&bp).unwrap();
        let back: Blueprint = serde_json::from_str(&s).unwrap();
        assert_eq!(back.cells.len(), 1);
        assert_eq!(back.cells[0].dir, 2);
        assert_eq!(back.name, "往返");
    }
}
