//! 细胞参数窗口：**按细胞类型**暴露各自的参数。
//!
//! 以前是 16 种 × 5 个属性一视同仁，结果 80 个滑块里有 29 个是死旋钮——
//! `damage` 只有杀手/刺会读，`sense_range` 只有眼睛会读，拖别的类型完全没反应。
//! 现在每种细胞在 `CELL_DEFS[i].params` 里声明自己有哪些参数，面板照着渲染。
//!
//! 三件事需要小心：
//!
//! * **改的是活世界的配置**，立刻影响后续演化，但不是"重置世界"——已经存在的个体会
//!   按新参数结算，所以数值跨度别设太夸张，否则一次拖动就能让全种群暴毙。
//! * **默认值以 `CELL_DEFS` 为准**，不在 UI 里写死。以后加细胞或调平衡，
//!   这里的「重置」会自动跟着走。
//! * **只在真的变了才写回**。egui 的 Slider 会在绘制时按 `step_by` 吸附值，
//!   无条件写回等于让"打开窗口"变成一个会改配置的动作（0.006 被吸成 0 这个坑踩过）。

use egui::{Color32, CornerRadius, Sense, Stroke, Vec2};

use evosim_core::cell_type::{self, CELL_DEFS};
use evosim_core::config::CellAttrs;
use evosim_core::world::World;

use crate::ui::{ACCENT, BORDER, TEXT, TEXT_DIM};

fn fmt(v: f64, decimals: usize) -> String {
    format!("{:.*}", decimals, v)
}

/// `CELL_DEFS` 里的出厂值。倍率类一律走 `CellAttrs::default()`（1.0）。
fn defaults(id: u8) -> CellAttrs {
    match cell_type::def(id) {
        Some(d) => CellAttrs {
            hp: d.hp,
            cost: d.cost,
            damage: d.damage,
            sense_range: d.sense_range,
            food_value: d.food_value,
            ..CellAttrs::default()
        },
        None => CellAttrs::default(),
    }
}

pub fn draw(ui: &mut egui::Ui, world: &mut World, selected: &mut u8) {
    // ── 细胞选择：4×4 色块 ──
    ui.label(
        egui::RichText::new("选一种细胞")
            .color(TEXT_DIM)
            .size(11.0),
    );
    ui.add_space(2.0);
    let cols = 4;
    let btn_w = 104.0;
    for row in 0..4 {
        ui.horizontal(|ui| {
            for c in 0..cols {
                let id = (row * cols + c) as u8;
                if id as usize >= CELL_DEFS.len() {
                    continue;
                }
                let d = &CELL_DEFS[id as usize];
                let on = *selected == id;
                let (rect, resp) = ui.allocate_exact_size(
                    Vec2::new(btn_w, 22.0),
                    Sense::click(),
                );
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
                // 左侧色块 = 这个细胞在世界里的颜色
                let c = d.colors[0];
                let sw = egui::Rect::from_min_size(
                    rect.left_top() + egui::vec2(5.0, 5.0),
                    Vec2::splat(12.0),
                );
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
        });
    }

    ui.add_space(6.0);
    let d = &CELL_DEFS[*selected as usize];
    ui.separator();

    // ── 选中细胞的说明 ──
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(d.label)
                .color(TEXT)
                .size(13.0)
                .strong(),
        );
        ui.label(
            egui::RichText::new(if d.terrain {
                "地形"
            } else if d.is_body_cell {
                "身体细胞"
            } else {
                "世界物体"
            })
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
        ui.label(
            egui::RichText::new(d.desc)
                .color(TEXT_DIM)
                .size(10.0),
        );
    }

    ui.add_space(4.0);

    let id = *selected as usize;
    // 一次性读进局部副本再写回：直接对 `cfg.cells[id]` 的字段取 &mut
    // 会和循环里的 ui 借用打架，绕开它比跟借用检查器较劲划算。
    let before = world.config.cells[id];
    let factory = defaults(*selected);
    let mut cur = before;

    if d.params.is_empty() {
        // 地形 / 空 / 墙 / 食物：这些格子没有"每细胞"参数
        let why = if d.terrain {
            "地形效果在右侧「参数」面板里，每种地形一组（岩浆 / 沼泽 / 辐射 / 腐殖质）"
        } else if d.id == cell_type::ct::FOOD {
            "食物没有每格参数，腐败速率和能量换算在右侧「参数」面板里"
        } else {
            "这类格子没有可调参数"
        };
        ui.label(
            egui::RichText::new(why)
                .color(Color32::from_rgb(251, 191, 36))
                .size(10.0),
        );
    } else {
        // 用 Grid 而不是 Slider::text()：`text()` 把名字排在**数值框之后**，
        // 一行变成「滑块 1.0 血量」，名字在最右边、还每行不等宽，扫视起来很难受。
        // Grid 让名字固定在左列，滑块起点也对齐。
        ui.spacing_mut().slider_width = 200.0;
        egui::Grid::new("cell_attrs")
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
        if cur != before {
            world.config.cells[id] = cur;
        }
    }

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui
            .button("重置这一项")
            .on_hover_text("恢复成出厂默认值")
            .clicked()
        {
            world.config.cells[id] = defaults(*selected);
        }
        if ui
            .button("全部重置")
            .on_hover_text("16 种细胞全部恢复出厂默认值")
            .clicked()
        {
            for i in 0..CELL_DEFS.len() {
                world.config.cells[i] = defaults(i as u8);
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            // right_to_left 里先加的排在最右，所以先垫一格空白，
            // 否则计数会紧贴窗口右边框（看着像被裁了一半）。
            ui.add_space(6.0);
            // 显示"改没改过"，省得用户忘了自己动过手
            let changed = (0..CELL_DEFS.len())
                .filter(|&i| world.config.cells[i] != defaults(i as u8))
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use evosim_core::config::AttrField;

    #[test]
    fn defaults_match_cell_defs() {
        // 「重置」按钮依赖这条：CELL_DEFS 是唯一真源，UI 里不写死数字
        for d in CELL_DEFS.iter() {
            let a = defaults(d.id);
            assert_eq!(a.hp, d.hp, "{} 的血量默认值不一致", d.key);
            assert_eq!(a.food_value, d.food_value, "{} 的食物值默认值不一致", d.key);
        }
    }

    #[test]
    fn config_default_matches_cell_defs() {
        // 窗口右下角会显示"N 种被改过"，它靠比较 Config 与 CELL_DEFS 得出。
        // 两者一旦不同源，用户一打开窗口就会看到莫须有的"改动"。
        let cfg = evosim_core::config::Config::default();
        for d in CELL_DEFS.iter() {
            assert_eq!(
                cfg.cells[d.id as usize],
                defaults(d.id),
                "{}（id {}）的 Config 默认值和 CELL_DEFS 不一致",
                d.key,
                d.id
            );
        }
    }

    #[test]
    fn get_set_roundtrip_covers_every_field() {
        // get/set 的 match 漏一个分支 → 那个滑块拖了没反应，而且不报错。
        let all = [
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
        for (k, f) in all.iter().enumerate() {
            let mut a = CellAttrs::default();
            let v = 100.0 + k as f64; // 每个字段写不同的值，串了就能发现
            a.set(*f, v);
            assert_eq!(a.get(*f), v, "{:?} 的 get/set 不互通", f);
        }
    }

    #[test]
    fn every_factory_value_fits_its_slider() {
        // 量程盖不住出厂值 → egui 在绘制时把值 clamp 掉，用户只是打开窗口、
        // 什么都没碰，配置就变了。这条和"步长把 0.006 吸附成 0"是同一类 bug，
        // 一旦有人调了某个细胞的数值而忘了改量程，这里会先炸。
        for d in CELL_DEFS.iter() {
            let v = defaults(d.id);
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

    /// 每种身体细胞都必须有参数，否则面板上是个空窗；地形则必须**没有**
    /// 每细胞参数（它们走 `Config.terrain`，在这里给滑块是误导）。
    #[test]
    fn params_are_declared_for_body_cells_only() {
        for d in CELL_DEFS.iter() {
            if d.is_body_cell {
                assert!(
                    !d.params.is_empty(),
                    "{} 是身体细胞，应该有参数",
                    d.key
                );
            } else {
                assert!(
                    d.params.is_empty(),
                    "{} 不是身体细胞，不该有每细胞参数（地形走 Config.terrain）",
                    d.key
                );
            }
        }
    }

    /// 专属参数不能张冠李戴：`damage` 只给杀手/刺，`sense_range` 只给眼睛。
    /// 这些是"拖了没反应"的根源，让注册表自己守住。
    #[test]
    fn exclusive_params_only_on_types_that_use_them() {
        use evosim_core::cell_type::ct;
        for d in CELL_DEFS.iter() {
            let has = |f: AttrField| d.params.contains(&f);
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
                    d.params.contains(&f),
                    "{} 缺少通用参数「{}」",
                    d.key,
                    f.label()
                );
            }
        }
    }
}
