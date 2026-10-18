//! 类型参数窗口：**选中哪个类型，就调哪个类型** —— 细胞和地形共用一套渲染。
//!
//! 和「全局参数」窗口的分工：这里的值**每种类型一份**，改「生产者」的生产速度只影响
//! 生产者细胞，改「岩浆」的踏入伤害只影响岩浆格；那边的值整个模拟只有一份
//! （世界 / 生命 / 变异 / 感知），改了对所有个体所有格子一起生效。
//!
//! 一句话：**实际值 = 全局基值（全局窗口）× 类型倍率（这里）**。所以两边不是
//! "粗调 / 细调"的关系，而是"一份 / 每种一份"的关系。
//!
//! 面板列出两类：9 种**身体细胞**（`LIVING_CELL_TYPES`）和 4 种**有参数的地形**
//! （`PARAM_TERRAIN_TYPES`）。空 / 食物 / 墙的数值写死在模拟里，列进来就是空窗。
//!
//! 每种类型有哪些旋钮由 `CELL_DEFS[i].params` 声明，顺序即显示顺序。以前是
//! 16 种 × 5 个属性一视同仁，结果 80 个滑块里 29 个是死旋钮——`damage` 只有杀手/刺会读，
//! `sense_range` 只有眼睛会读，拖别的类型完全没反应。
//!
//! 三件事需要小心：
//!
//! * **改的是活世界的配置**，立刻影响后续演化，但不是"重置世界"——已经存在的个体会
//!   按新参数结算，所以数值跨度别设太夸张，否则一次拖动就能让全种群暴毙。
//! * **默认值以 `CELL_DEFS` / `TerrainCfg::default()` 为准**，不在 UI 里写死。
//!   以后加细胞、调平衡、加地形参数，这里的「重置」会自动跟着走。
//! * **只在真的变了才写回**。egui 的 Slider 会在绘制时按 `step_by` 吸附值，
//!   无条件写回等于让"打开窗口"变成一个会改配置的动作（0.006 被吸成 0 这个坑踩过）。

use egui::{Color32, CornerRadius, Sense, Stroke, Vec2};

use evosim_core::cell_type::{self, CELL_DEFS, LIVING_CELL_TYPES, PARAM_TERRAIN_TYPES};
use evosim_core::config::{CellAttrs, ParamRef, TerrainCfg};
use evosim_core::world::World;

use crate::ui::{ACCENT, BORDER, TEXT, TEXT_DIM};

fn fmt(v: f64, decimals: usize) -> String {
    format!("{:.*}", decimals, v)
}

/// 一次「选中的类型」的全部可调值。
///
/// 面板不直接改 `World`：先把当前值拷进这个枚举，滑块改副本，最后整体写回。
/// 这样既绕开了 `&mut world.config` 和循环里 `ui` 的借用冲突，
/// 又保证"只在真的变了才写回"。
///
/// 为什么是枚举而不是两个分支各写一遍：地形和细胞的参数区长得一模一样
/// （名字 + 出厂值 + 滑块 + hint），只有"值存在哪"不同。枚举把这点差异收在一处。
#[derive(Clone, Copy)]
enum Values {
    Cell(CellAttrs),
    Terrain(TerrainCfg),
}

impl Values {
    fn get(self, f: ParamRef) -> f64 {
        match (self, f) {
            (Values::Cell(a), ParamRef::Cell(k)) => a.get(k),
            (Values::Terrain(t), ParamRef::Terrain(k)) => t.get(k),
            // 走不到：`d.params` 的变体一定和 `d` 的种类匹配，`params_kind_matches_type_kind`
            // 测试守着。兜底返回 0 而不是 panic——万一以后挂错了项，宁可那个滑块显示 0，
            // 也别让整个程序崩掉。
            _ => 0.0,
        }
    }

    fn set(&mut self, f: ParamRef, v: f64) {
        match (self, f) {
            (Values::Cell(a), ParamRef::Cell(k)) => a.set(k, v),
            (Values::Terrain(t), ParamRef::Terrain(k)) => t.set(k, v),
            _ => {}
        }
    }
}

/// 某类型的**出厂值**。
///
/// 身体细胞从 `CELL_DEFS` 取前 5 项（唯一真源），倍率一律 1.0；
/// 地形直接给整个 `TerrainCfg::default()`——它只有一份，字段本身就是出厂值。
fn factory_values(id: u8) -> Values {
    match cell_type::def(id) {
        Some(d) if d.terrain => Values::Terrain(TerrainCfg::default()),
        Some(d) => Values::Cell(CellAttrs {
            hp: d.hp,
            cost: d.cost,
            damage: d.damage,
            sense_range: d.sense_range,
            food_value: d.food_value,
            ..CellAttrs::default()
        }),
        None => Values::Cell(CellAttrs::default()),
    }
}

fn current_values(world: &World, id: u8) -> Values {
    match cell_type::def(id) {
        Some(d) if d.terrain => Values::Terrain(world.config.terrain),
        _ => Values::Cell(world.config.cells[(id as usize).min(15)]),
    }
}

fn write_back(world: &mut World, id: u8, v: Values) {
    match v {
        Values::Cell(a) => world.config.cells[(id as usize).min(15)] = a,
        Values::Terrain(t) => world.config.terrain = t,
    }
}

/// 这一类型有没有被改动过。
///
/// **逐项比较 `d.params`，不能整体比 `Values`**：地形的值全在同一个 `TerrainCfg` 里，
/// 整体比较会让"改了岩浆"显示成"沼泽也被改过"。
fn changed_values(cur: &Values, id: u8) -> bool {
    let Some(d) = cell_type::def(id) else {
        return false;
    };
    let fac = factory_values(id);
    d.params.iter().any(|f| cur.get(*f) != fac.get(*f))
}

/// 把 `cur` 里属于 `id` 的那几项恢复出厂值，**别的字段一个都不碰**。
///
/// 地形共用一份 `TerrainCfg`，"重置岩浆"必须留下沼泽的改动——这是把
/// `factory_values(id)` 整体写回做不到的。
fn reset_into(cur: &mut Values, id: u8) {
    let Some(d) = cell_type::def(id) else {
        return;
    };
    let fac = factory_values(id);
    for f in d.params {
        cur.set(*f, fac.get(*f));
    }
}

fn reset_one(world: &mut World, id: u8) {
    let mut cur = current_values(world, id);
    reset_into(&mut cur, id);
    write_back(world, id, cur);
}

fn reset_all(world: &mut World) {
    for &id in LIVING_CELL_TYPES.iter().chain(PARAM_TERRAIN_TYPES.iter()) {
        reset_one(world, id);
    }
}

/// 面板上列出的全部类型：身体细胞 + 有参数的地形。
fn listed_types() -> Vec<u8> {
    LIVING_CELL_TYPES
        .iter()
        .chain(PARAM_TERRAIN_TYPES.iter())
        .copied()
        .collect()
}

/// 一个可选类型的方块按钮（自绘：左侧色块 + 名字，选中描蓝边）
fn type_tile(ui: &mut egui::Ui, id: u8, selected: &mut u8) {
    let d = &CELL_DEFS[id as usize];
    let on = *selected == id;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(142.0, 22.0), Sense::click());
    let p = ui.painter_at(rect);
    p.rect_filled(
        rect,
        CornerRadius::same(3),
        if on {
            Color32::from_rgb(30, 48, 66)
        } else if resp.hovered() {
            Color32::from_rgb(24, 30, 40)
        } else {
            Color32::from_rgb(17, 22, 30)
        },
    );
    p.rect_stroke(
        rect,
        CornerRadius::same(3),
        Stroke::new(1.0_f32, if on { ACCENT } else { BORDER }),
        egui::StrokeKind::Inside,
    );
    // 左侧色块 = 这个类型在世界里的颜色（细胞和地形同一套取法）
    let c = d.colors[0];
    let sw = egui::Rect::from_min_size(rect.left_top() + egui::vec2(5.0, 5.0), Vec2::splat(12.0));
    p.rect_filled(
        sw,
        CornerRadius::same(2),
        Color32::from_rgb(
            ((c >> 16) & 0xff) as u8,
            ((c >> 8) & 0xff) as u8,
            (c & 0xff) as u8,
        ),
    );
    p.rect_stroke(
        sw,
        CornerRadius::same(2),
        Stroke::new(1.0_f32, Color32::from_rgb(60, 72, 90)),
        egui::StrokeKind::Inside,
    );
    p.text(
        rect.left_center() + egui::vec2(23.0, 0.0),
        egui::Align2::LEFT_CENTER,
        d.label,
        egui::FontId::proportional(11.0),
        if on { ACCENT } else { TEXT },
    );
    if resp.clicked() {
        *selected = id;
    }
}

fn tile_grid(ui: &mut egui::Ui, ids: &[u8], selected: &mut u8) {
    let cols = 3;
    for row in 0..ids.len().div_ceil(cols) {
        ui.horizontal(|ui| {
            for c in 0..cols {
                let k = row * cols + c;
                if k >= ids.len() {
                    continue;
                }
                type_tile(ui, ids[k], selected);
            }
        });
    }
}

fn section_head(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).color(TEXT_DIM).size(11.0));
    ui.add_space(2.0);
}

pub fn draw(ui: &mut egui::Ui, world: &mut World, selected: &mut u8) {
    // 选中的 id 可能没有参数（比如以后把某种地形从 PARAM_TERRAIN_TYPES 里摘掉），
    // 那样参数区会是空窗。兜底到第一个身体细胞，用户不会卡在一个死界面上。
    if cell_type::def(*selected).map_or(true, |d| d.params.is_empty()) {
        *selected = LIVING_CELL_TYPES[0];
    }

    // ── 类型选择：两段 ──
    //
    // 分两段而不是一个 13 项的网格：语义不同（身体细胞 / 地形），而且地形只有 4 个，
    // 混排会在末尾留两个空格。
    section_head(ui, "选一个类型");
    tile_grid(ui, &LIVING_CELL_TYPES, selected);
    ui.add_space(6.0);
    section_head(ui, "地形");
    tile_grid(ui, &PARAM_TERRAIN_TYPES, selected);

    ui.add_space(6.0);
    let d = &CELL_DEFS[*selected as usize];
    ui.separator();

    // ── 选中类型的说明 ──
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(d.label)
                .color(TEXT)
                .size(13.0)
                .strong(),
        );
        ui.label(
            egui::RichText::new(if d.terrain { "地形" } else { "身体细胞" })
                .color(TEXT_DIM)
                .size(10.0),
        );
        ui.label(
            egui::RichText::new(format!("{} 项参数", d.params.len()))
                .color(TEXT_DIM)
                .size(10.0),
        );
    });
    if !d.desc.is_empty() {
        ui.label(egui::RichText::new(d.desc).color(TEXT_DIM).size(10.0));
    }

    ui.add_space(4.0);

    let id = *selected;
    let before = current_values(world, id);
    let factory = factory_values(id);
    let mut cur = before;

    if d.params.is_empty() {
        // 兜底：正常走不到（列表里全是"有参数"的类型，测试也保证着）。
        // 留着是为了以后加新类型忘了填 `params` 时不至于让窗口崩掉。
        ui.label(
            egui::RichText::new("这种类型还没声明参数——去 `CELL_DEFS` 里补上 `params`")
                .color(Color32::from_rgb(251, 191, 36))
                .size(10.0),
        );
    } else {
        // 用 Grid 而不是 Slider::text()：`text()` 把名字排在**数值框之后**，
        // 一行变成「滑块 1.0 血量」，名字在最右边、还每行不等宽，扫视起来很难受。
        // Grid 让名字固定在左列，滑块起点也对齐。
        ui.spacing_mut().slider_width = 200.0;
        egui::Grid::new("type_params")
            .num_columns(2)
            .spacing([10.0, 7.0])
            .show(ui, |ui| {
                for f in d.params {
                    let (lo, hi, step, dec) = f.range();
                    let fv = factory.get(*f);
                    let mut v = cur.get(*f);
                    // 名字右边挂出厂值：不挂的话用户不知道"正常"是多少，
                    // 而拖动过之后又看不出来回没回去。
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        // 改过的项名字变蓝，一眼看出动过哪个属性
                        ui.label(
                            egui::RichText::new(f.label())
                                .color(if v != fv { ACCENT } else { TEXT })
                                .size(12.0),
                        );
                        ui.label(
                            egui::RichText::new(fmt(fv, dec))
                                .color(TEXT_DIM)
                                .size(10.0),
                        );
                    });
                    ui.add(
                        egui::Slider::new(&mut v, lo..=hi)
                            .step_by(step)
                            .fixed_decimals(dec),
                    )
                    .on_hover_text(f.hint());
                    if v != cur.get(*f) {
                        cur.set(*f, v);
                    }
                    ui.end_row();
                }
            });
        // 只在真的变了才写回：Slider 会在绘制时 clamp/吸附值，
        // 无条件写回等于让"打开窗口"变成一个会改配置的动作。
        if d.params.iter().any(|f| cur.get(*f) != before.get(*f)) {
            write_back(world, id, cur);
        }
    }

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui
            .button("重置这一项")
            .on_hover_text(if d.terrain {
                "这一种地形恢复出厂默认值（其他地形不动）"
            } else {
                "这一种细胞恢复出厂默认值"
            })
            .clicked()
        {
            reset_one(world, id);
        }
        if ui
            .button("全部重置")
            .on_hover_text("所有细胞和地形恢复出厂默认值")
            .clicked()
        {
            reset_all(world);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // right_to_left 里先加的排在最右，所以先垫一格空白，
            // 否则计数会紧贴窗口右边框（看着像被裁了一半）。
            ui.add_space(6.0);
            // 显示"改没改过"，省得用户忘了自己动过手。
            // 地形也算进来：它们和细胞在同一个窗口里，只统计细胞会让人以为
            // 自己改的岩浆没生效。
            let changed = listed_types()
                .iter()
                .filter(|&&i| changed_values(&current_values(world, i), i))
                .count();
            ui.label(
                egui::RichText::new(if changed == 0 {
                    "全部是默认值".to_string()
                } else {
                    format!("{} 种被改过", changed)
                })
                .color(if changed == 0 { TEXT_DIM } else { ACCENT })
                .size(10.0),
            );
        });
    });
    ui.label(
        egui::RichText::new("改动立刻生效，影响后续演化（不会重置世界）")
            .color(TEXT_DIM)
            .size(10.0),
    );
    ui.label(
        egui::RichText::new("地形参数对画布上所有该地形的格子生效")
            .color(TEXT_DIM)
            .size(10.0),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use evosim_core::cell_type::ct;
    use evosim_core::config::{AttrField, TerrainField};

    #[test]
    fn factory_matches_cell_defs() {
        // 「重置」依赖这条：CELL_DEFS 是唯一真源，UI 里不写死数字
        for d in CELL_DEFS.iter().filter(|d| d.is_body_cell) {
            match factory_values(d.id) {
                Values::Cell(a) => {
                    assert_eq!(a.hp, d.hp, "{} 的血量默认值不一致", d.key);
                    assert_eq!(a.food_value, d.food_value, "{} 的食物值默认值不一致", d.key);
                }
                Values::Terrain(_) => panic!("{} 是身体细胞，出厂值不该是地形配置", d.key),
            }
        }
    }

    #[test]
    fn config_default_matches_factory() {
        // 右下角会显示"N 种被改过"，它靠 current == factory 判断。
        // 两者一旦不同源，用户一打开窗口就会看到莫须有的"改动"。
        let cfg = evosim_core::config::Config::default();
        for d in CELL_DEFS.iter().filter(|d| d.is_body_cell) {
            match factory_values(d.id) {
                Values::Cell(a) => assert_eq!(
                    cfg.cells[d.id as usize], a,
                    "{}（id {}）的 Config 默认值和 CELL_DEFS 不一致",
                    d.key, d.id
                ),
                Values::Terrain(_) => unreachable!(),
            }
        }
        for &id in PARAM_TERRAIN_TYPES.iter() {
            let d = &CELL_DEFS[id as usize];
            for f in d.params {
                if let ParamRef::Terrain(tf) = f {
                    assert_eq!(
                        cfg.terrain.get(*tf),
                        TerrainCfg::default().get(*tf),
                        "{}（id {}）的 {} 默认值和 TerrainCfg::default() 不一致",
                        d.key,
                        d.id,
                        tf.label()
                    );
                }
            }
        }
    }

    #[test]
    fn cell_attr_get_set_roundtrip() {
        // get/set 的 match 漏一个分支 → 那个滑块拖了没反应，而且不报错。
        for (k, f) in AttrField::ALL.iter().enumerate() {
            let mut a = CellAttrs::default();
            let v = 100.0 + k as f64; // 每个字段写不同的值，串了就能发现
            a.set(*f, v);
            assert_eq!(a.get(*f), v, "{:?} 的 get/set 不互通", f);
        }
    }

    #[test]
    fn terrain_get_set_roundtrip() {
        for (k, f) in TerrainField::ALL.iter().enumerate() {
            let mut t = TerrainCfg::default();
            let v = 100.0 + k as f64;
            t.set(*f, v);
            assert_eq!(t.get(*f), v, "{:?} 的 get/set 不互通", f);
        }
    }

    #[test]
    fn every_factory_value_fits_its_slider() {
        // 量程盖不住出厂值 → egui 在绘制时把值 clamp 掉，用户只是打开窗口、
        // 什么都没碰，配置就变了。这条和"步长把 0.006 吸附成 0"是同一类 bug，
        // 一旦有人调了某个类型的数值而忘了改量程，这里会先炸。
        for d in CELL_DEFS.iter() {
            let v = factory_values(d.id);
            for f in d.params {
                let (lo, hi, _, _) = f.range();
                let x = v.get(*f);
                assert!(
                    x >= lo && x <= hi,
                    "{}（id {}）的 {} 出厂值 {} 不在量程 {}..={} 内",
                    d.key,
                    d.id,
                    f.label(),
                    x,
                    lo,
                    hi
                );
            }
        }
    }

    /// 面板上出现的每一种类型都必须有参数；不出现的类型必须一个都没有。
    #[test]
    fn params_declared_exactly_for_listed_types() {
        let listed = listed_types();
        for d in CELL_DEFS.iter() {
            if listed.contains(&d.id) {
                assert!(!d.params.is_empty(), "{} 在面板上，却没有参数（空窗）", d.key);
            } else {
                assert!(
                    d.params.is_empty(),
                    "{} 不在面板上，却有参数——永远调不到",
                    d.key
                );
            }
        }
    }

    /// `PARAM_TERRAIN_TYPES` 必须恰好是"有地形参数的那些地形"。
    #[test]
    fn param_terrain_types_are_complete() {
        for &id in PARAM_TERRAIN_TYPES.iter() {
            let d = cell_type::def(id).expect("PARAM_TERRAIN_TYPES 里不该有未知 id");
            assert!(d.terrain, "{} 不是地形，不该出现在地形段", d.key);
            assert!(!d.params.is_empty(), "{} 没有参数，选中会是空窗", d.key);
            assert!(
                d.params.iter().all(|f| !f.is_cell()),
                "{} 是地形，参数不该落在 CellAttrs 上",
                d.key
            );
        }
        // 反过来：有地形参数的地形不能漏在列表外，否则永远调不到
        for d in CELL_DEFS.iter().filter(|d| d.terrain && !d.params.is_empty()) {
            assert!(
                PARAM_TERRAIN_TYPES.contains(&d.id),
                "{} 有地形参数却不在 PARAM_TERRAIN_TYPES 里",
                d.key
            );
        }
    }

    /// 参数种类必须和类型种类对上：身体细胞只能声明 `ParamRef::Cell`，地形只能声明
    /// `ParamRef::Terrain`。挂错了 `Values::get` 会静默返回 0（拖不动、也看不出错）。
    #[test]
    fn params_kind_matches_type_kind() {
        for d in CELL_DEFS.iter() {
            for f in d.params {
                if d.terrain {
                    assert!(
                        !f.is_cell(),
                        "{} 是地形，不该声明细胞属性「{}」",
                        d.key,
                        f.label()
                    );
                } else {
                    assert!(
                        f.is_cell(),
                        "{} 不是地形，不该声明地形参数「{}」",
                        d.key,
                        f.label()
                    );
                }
            }
        }
    }

    /// `TerrainField` 的每个变体都要被某种地形声明，且只能被一种声明。
    ///
    /// 漏声明 = 界面上根本没有那个旋钮（`TerrainCfg` 有字段但没人挂）；
    /// 重复声明 = 改岩浆会连带改沼泽，用户完全想不到。
    #[test]
    fn every_terrain_field_is_declared_once() {
        let mut declared: Vec<TerrainField> = Vec::new();
        for &id in PARAM_TERRAIN_TYPES.iter() {
            for f in CELL_DEFS[id as usize].params {
                if let ParamRef::Terrain(tf) = f {
                    assert!(!declared.contains(tf), "{:?} 被多种地形重复声明", tf);
                    declared.push(*tf);
                }
            }
        }
        for tf in TerrainField::ALL.iter() {
            assert!(
                declared.contains(tf),
                "{:?}（{}）没挂到任何地形上，界面上调不到",
                tf,
                tf.label()
            );
        }
    }

    /// 专属参数不能张冠李戴：`damage` 只给杀手/刺，`sense_range` 只给眼睛。
    /// 这些是"拖了没反应"的根源，让注册表自己守住。
    #[test]
    fn exclusive_params_only_on_types_that_use_them() {
        for d in CELL_DEFS.iter() {
            let has = |f: AttrField| d.params.contains(&ParamRef::Cell(f));
            if has(AttrField::Damage) {
                assert!(
                    d.id == ct::KILLER || d.id == ct::SPIKE,
                    "{} 不该有「伤害」——只有杀手和刺会读它",
                    d.key
                );
            }
            if has(AttrField::SenseRange) {
                assert_eq!(d.id, ct::EYE, "{} 不该有「感知距离」——只有眼睛会读它", d.key);
            }
            // 倍率各自只属于一种细胞
            for (f, owner) in [
                (AttrField::BiteMult, ct::MOUTH),
                (AttrField::ProdMult, ct::PRODUCER),
                (AttrField::RegenMult, ct::REGEN),
                (AttrField::StorageMult, ct::STORAGE),
                (AttrField::ArmorResist, ct::ARMOR),
            ] {
                if has(f) {
                    assert_eq!(d.id, owner, "{} 不该有「{}」", d.key, f.label());
                }
            }
        }
    }

    /// 三种通用属性每种身体细胞都要有，否则改不了一个该能改的数。
    #[test]
    fn universal_params_present_everywhere() {
        for d in CELL_DEFS.iter().filter(|d| d.is_body_cell) {
            for f in [AttrField::Hp, AttrField::Cost, AttrField::FoodValue] {
                assert!(
                    d.params.contains(&ParamRef::Cell(f)),
                    "{} 缺少通用参数「{}」",
                    d.key,
                    f.label()
                );
            }
        }
    }

    /// 地形共用一份 `TerrainCfg`，所以"重置这一项"必须只动自己那几个字段。
    /// 整体写回 `factory_values(id)` 会把别的地形一起清掉——这条守着它。
    #[test]
    fn resetting_one_terrain_keeps_the_others() {
        let mut v = Values::Terrain(TerrainCfg {
            lava_enter_damage: 99.0,
            swamp_move_cost: 42.0,
            ..TerrainCfg::default()
        });
        reset_into(&mut v, ct::LAVA);
        match v {
            Values::Terrain(t) => {
                assert_eq!(
                    t.lava_enter_damage,
                    TerrainCfg::default().lava_enter_damage,
                    "岩浆没被重置"
                );
                assert_eq!(t.swamp_move_cost, 42.0, "重置岩浆不该动沼泽");
            }
            Values::Cell(_) => panic!("地形不该变成细胞配置"),
        }
    }

    /// 同理，"N 种被改过"的计数也不能整体比 `TerrainCfg`，
    /// 否则改一个地形会让另外三个都显示成"被改过"。
    #[test]
    fn changing_one_terrain_does_not_mark_the_others() {
        let v = Values::Terrain(TerrainCfg {
            lava_enter_damage: 9.0,
            ..TerrainCfg::default()
        });
        assert!(changed_values(&v, ct::LAVA), "改了岩浆却没被算作改动");
        assert!(
            !changed_values(&v, ct::SWAMP),
            "改了岩浆不该让沼泽显示成被改过"
        );
        assert!(
            !changed_values(&v, ct::RAD),
            "改了岩浆不该让辐射带显示成被改过"
        );
        assert!(
            !changed_values(&v, ct::HUMUS),
            "改了岩浆不该让腐殖质显示成被改过"
        );
    }

    /// 没动过的类型不能被算成"改过"。
    #[test]
    fn untouched_values_are_not_changed() {
        for id in listed_types() {
            assert!(
                !changed_values(&factory_values(id), id),
                "{} 的出厂值被判成了「被改过」",
                CELL_DEFS[id as usize].key
            );
        }
    }
}
