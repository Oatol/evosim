//! 细胞参数窗口：16 种细胞 × 5 个属性。
//!
//! 参数本身在 `Config.cells` 里（早就存在、也进存档），这个窗口只是把它露出来。
//! 两件事需要小心：
//!
//! * **改的是活世界的配置**，立刻影响后续演化，但不是"重置世界"——已经存在的个体会
//!   按新参数结算，所以数值跨度别设太夸张，否则一次拖动就能让全种群暴毙。
//! * **默认值以 `CELL_DEFS` 为准**，不在 UI 里写死。以后加细胞或调平衡，
//!   这里的「重置」会自动跟着走。

use egui::{Color32, CornerRadius, Sense, Stroke, Vec2};

use evosim_core::cell_type::{self, CELL_DEFS};
use evosim_core::config::{AttrField, CellAttrs};
use evosim_core::world::World;

use crate::ui::{ACCENT, BORDER, TEXT, TEXT_DIM};

/// 5 个属性的滑块配置。
///
/// 两条经验，都是踩出来的：
///
/// * `step` 必须按属性的**实际量级**给。消耗的默认值是 0.002~0.008 这个量级，
///   如果用 0.05 的步长，egui 会在绘制时把 0.006 吸附成 0——用户只是打开窗口、
///   什么都没碰，配置就被改了。
/// * `hi` 必须贴着**真实取值域**给，不能图省事给个大数。血量出厂值是 1.0，
///   量程要是 0~20，拖柄就永远趴在最左端（5% 处），既看不出当前值也调不动。
///   现在每种属性的出厂值大致落在量程的 10%~40%，拖动才是有意义的。
struct FieldDef {
    field: AttrField,
    name: &'static str,
    lo: f64,
    hi: f64,
    step: f64,
    decimals: usize,
    hint: &'static str,
}

static FIELDS: [FieldDef; 5] = [
    FieldDef {
        field: AttrField::Hp,
        name: "血量",
        lo: 0.0,
        hi: 5.0,
        step: 0.1,
        decimals: 1,
        hint: "被打掉多少伤害这个细胞才会死。0 = 一碰就碎。",
    },
    FieldDef {
        field: AttrField::Cost,
        name: "消耗",
        lo: 0.0,
        hi: 0.05,
        // 默认值就是 0.002 级别，步长必须比它细
        step: 0.001,
        decimals: 3,
        hint: "每 tick 的能量开销（会再乘上全局消耗量倍率）。",
    },
    FieldDef {
        field: AttrField::Damage,
        name: "伤害",
        lo: 0.0,
        hi: 5.0,
        step: 0.1,
        decimals: 1,
        hint: "攻击型细胞每次命中造成的伤害。",
    },
    FieldDef {
        field: AttrField::SenseRange,
        name: "感知距离",
        lo: 0.0,
        hi: 10.0,
        step: 1.0,
        decimals: 0,
        hint: "眼睛能看多远。0 = 用全局视野距离。",
    },
    FieldDef {
        field: AttrField::FoodValue,
        name: "食物值",
        lo: 0.0,
        hi: 10.0,
        step: 0.1,
        decimals: 1,
        hint: "被吃掉时提供多少能量（会再乘上食物量倍率）。",
    },
];

fn fmt(v: f64, decimals: usize) -> String {
    format!("{:.*}", decimals, v)
}

/// `CELL_DEFS` 里的出厂值
fn defaults(id: u8) -> CellAttrs {
    match cell_type::def(id) {
        Some(d) => CellAttrs {
            hp: d.hp,
            cost: d.cost,
            damage: d.damage,
            sense_range: d.sense_range,
            food_value: d.food_value,
        },
        None => CellAttrs::default(),
    }
}

fn get(a: &CellAttrs, f: AttrField) -> f64 {
    match f {
        AttrField::Hp => a.hp,
        AttrField::Cost => a.cost,
        AttrField::Damage => a.damage,
        AttrField::SenseRange => a.sense_range,
        AttrField::FoodValue => a.food_value,
    }
}

fn set(a: &mut CellAttrs, f: AttrField, v: f64) {
    match f {
        AttrField::Hp => a.hp = v,
        AttrField::Cost => a.cost = v,
        AttrField::Damage => a.damage = v,
        AttrField::SenseRange => a.sense_range = v,
        AttrField::FoodValue => a.food_value = v,
    }
}

/// 数组下标 → 属性，由 `FIELDS` 决定。
///
/// 不写死 `[a.hp, a.cost, ...]`：那样以后在 `FIELDS` 里插一行、忘了同步
/// 这里的顺序，滑块就会去改别的属性，而且不会报错。
fn read(a: &CellAttrs) -> [f64; 5] {
    std::array::from_fn(|k| get(a, FIELDS[k].field))
}

fn write(a: &mut CellAttrs, v: [f64; 5]) {
    for (k, f) in FIELDS.iter().enumerate() {
        set(a, f.field, v[k]);
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
    });
    if !d.desc.is_empty() {
        ui.label(
            egui::RichText::new(d.desc)
                .color(TEXT_DIM)
                .size(10.0),
        );
    }

    ui.add_space(4.0);

    // ── 5 个属性 ──
    //
    // 一次性读进数组再写回：直接对 `cfg.cells[id]` 的字段取 &mut
    // 会和循环里的 ui 借用打架，绕开它比跟借用检查器较劲划算。
    let id = *selected as usize;
    let before = read(&world.config.cells[id]);
    let factory = read(&defaults(*selected));
    let mut vals = before;

    // 用 Grid 而不是 Slider::text()：`text()` 把名字排在**数值框之后**，
    // 一行变成「滑块 1.0 血量」，名字在最右边、还每行不等宽，扫视起来很难受。
    // Grid 让名字固定在左列，滑块起点也对齐。
    ui.spacing_mut().slider_width = 200.0;
    egui::Grid::new("cell_attrs")
        .num_columns(2)
        .spacing([10.0, 7.0])
        .show(ui, |ui| {
            for (k, f) in FIELDS.iter().enumerate() {
                // 名字右边挂出厂值：不挂的话用户不知道"正常"是多少，
                // 而拖动过之后又看不出来回没回去。
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    // 改过的项名字变蓝，一眼看出动过哪个属性
                    let touched = vals[k] != factory[k];
                    ui.label(
                        egui::RichText::new(f.name)
                            .color(if touched { ACCENT } else { TEXT })
                            .size(12.0),
                    );
                    ui.label(
                        egui::RichText::new(fmt(factory[k], f.decimals))
                            .color(TEXT_DIM)
                            .size(10.0),
                    );
                });
                ui.add(
                    egui::Slider::new(&mut vals[k], f.lo..=f.hi)
                        .step_by(f.step)
                        .fixed_decimals(f.decimals),
                )
                .on_hover_text(f.hint);
                ui.end_row();
            }
        });
    // 只在真的变了才写回：egui 的 Slider 会在绘制时 clamp/吸附值，
    // 无条件写回等于让"打开窗口"变成一个会改配置的动作。
    if vals != before {
        write(&mut world.config.cells[id], vals);
    }

    // ── 哪些属性对这个细胞真的有用 ──
    ui.add_space(2.0);
    let mut notes: Vec<&str> = Vec::new();
    if d.terrain {
        notes.push("地形：只有「血量」和「食物值」有意义");
    }
    if d.behavior.is_none() && !d.terrain && !d.is_body_cell {
        notes.push("这一项没有行为逻辑，属性基本不影响模拟");
    }
    if !notes.is_empty() {
        ui.label(
            egui::RichText::new(notes.join("；"))
                .color(Color32::from_rgb(251, 191, 36))
                .size(10.0),
        );
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
                .filter(|&i| {
                    read(&world.config.cells[i]) != read(&defaults(i as u8))
                })
                .count();
            ui.label(
                egui::RichText::new(if changed == 0 {
                    "全部是默认值".to_string()
                } else {
                    format!("{} 项被改过", changed)
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
        // 窗口右下角会显示"N 项被改过"，它靠比较 Config 与 CELL_DEFS 得出。
        // 两者一旦不同源，用户一打开窗口就会看到莫须有的"改动"。
        let cfg = evosim_core::config::Config::default();
        for d in CELL_DEFS.iter() {
            let a = cfg.cells[d.id as usize];
            assert_eq!(
                read(&a),
                read(&defaults(d.id)),
                "{}（id {}）的 Config 默认值和 CELL_DEFS 不一致",
                d.key,
                d.id
            );
        }
    }

    #[test]
    fn read_write_roundtrip() {
        let mut a = CellAttrs::default();
        let v = [1.5, 2.5, 3.5, 4.5, 5.5];
        write(&mut a, v);
        assert_eq!(read(&a), v);
    }

    #[test]
    fn every_factory_value_fits_its_slider() {
        // 量程盖不住出厂值 → egui 在绘制时把值 clamp 掉，用户只是打开窗口、
        // 什么都没碰，配置就变了。这条和"步长把 0.006 吸附成 0"是同一类 bug，
        // 一旦有人调了某个细胞的数值而忘了改量程，这里会先炸。
        for d in CELL_DEFS.iter() {
            let v = read(&defaults(d.id));
            for (k, f) in FIELDS.iter().enumerate() {
                assert!(
                    v[k] >= f.lo && v[k] <= f.hi,
                    "{}（id {}）的 {} 出厂值 {} 不在量程 {}..={} 内",
                    d.key,
                    d.id,
                    f.name,
                    v[k],
                    f.lo,
                    f.hi
                );
            }
        }
    }

    #[test]
    fn field_table_covers_all_five_attrs() {
        // 少一项就意味着某个属性在 UI 里改不了
        assert_eq!(FIELDS.len(), 5);
        let names: Vec<&str> = FIELDS.iter().map(|f| f.name).collect();
        assert_eq!(names, vec!["血量", "消耗", "伤害", "感知距离", "食物值"]);
    }
}
