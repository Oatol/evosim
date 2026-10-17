//! 检视器 / 物种面板 / 曲线 —— M3 剩余部分的第一批。
//!
//! 全部用 egui 画（曲线与条形图用 `painter` 自绘）。高亮由主程序算成屏幕矩形，
//! 这里只负责描边——这样不用改着色器，也不会碰世界状态。

use std::collections::VecDeque;

use egui::{Align2, Color32, CornerRadius, FontId, Sense, Stroke, StrokeKind};

use evosim_core::anatomy::Flags;
use evosim_core::world::World;
use evosim_render::colors::species_rgb;

use crate::ui::{ACCENT, BORDER, PANEL, TEXT, TEXT_DIM};

/// 一条历史采样（曲线用）
#[derive(Clone, Copy)]
pub struct Sample {
    pub tick: u64,
    pub pop: f32,
    pub food: f32,
    pub species: f32,
}

pub struct Panels {
    pub selected_org: Option<u32>,
    pub follow: bool,
    /// 图钉：锁定检视目标。锁上之后用检视工具点画布不会换目标、也不会清空。
    pub pinned: bool,
    /// 是否同时高亮"同物种的其他个体"（默认开）
    pub highlight_species: bool,
    pub selected_species: Option<String>,
    pub history: VecDeque<Sample>,
    last_sample_tick: u64,
    pub show_inspector: bool,
    pub show_species: bool,
    pub show_chart: bool,
}

impl Default for Panels {
    fn default() -> Self {
        Panels {
            selected_org: None,
            follow: false,
            pinned: false,
            highlight_species: true,
            selected_species: None,
            history: VecDeque::new(),
            last_sample_tick: 0,
            show_inspector: true,
            show_species: true,
            show_chart: true,
        }
    }
}

#[derive(Default)]
pub struct PanelActions {
    pub center_on: Option<u32>,
    pub clear_selection: bool,
}

impl Panels {
    /// 每 20 tick 采一个点，最多留 600 个（约 12000 tick 的视野）
    pub fn sample(&mut self, world: &World) {
        if world.tick < self.last_sample_tick + 20 {
            return;
        }
        self.last_sample_tick = world.tick;
        let stats = world.snapshot_stats();
        self.history.push_back(Sample {
            tick: world.tick,
            pop: stats.organisms as f32,
            food: stats.food as f32,
            species: world.species.size() as f32,
        });
        if self.history.len() > 600 {
            self.history.pop_front();
        }
    }

    /// 选中个体死了就取消跟随（避免相机追着一个不存在的目标）
    pub fn tick(&mut self, world: &World) {
        if let Some(id) = self.selected_org {
            match world.organism(id) {
                Some(o) if o.alive => {}
                _ => {
                    self.selected_org = None;
                    self.follow = false;
                    self.pinned = false;
                }
            }
        }
    }

    pub fn draw(
        &mut self,
        ctx: &egui::Context,
        world: &World,
        highlight: &mut Vec<(egui::Rect, Color32)>,
    ) -> PanelActions {
        let mut act = PanelActions::default();

        if self.show_inspector {
            if let Some(id) = self.selected_org {
                if let Some(org) = world.organism(id) {
                    let mut open = true;
                    egui::Window::new("检视器")
                        .resizable(false)
                        .collapsible(false)
                        .open(&mut open)
                        // 放在工具栏右侧，别被工具窗挡住
                        .default_pos([230.0, 42.0])
                        .show(ctx, |ui| {
                            inspector_body(ui, world, org, self, &mut act);
                        });
                    if !open {
                        act.clear_selection = true;
                    }
                }
            }
        }

        // ── 选中个体 / 同物种的高亮 ──
        //
        // 检视工具选中一个个体后，画布上必须能看出"选的是谁"：
        // 它自己用强调色描边，同物种的其他个体用该物种的指纹色（暗一档）。
        if let Some(id) = self.selected_org {
            if let Some(org) = world.organism(id) {
                push_cells(highlight, org, ACCENT);
                if self.highlight_species {
                    if let Some(fp) = org.species_fingerprint.as_deref() {
                        let rgb = species_rgb(fp);
                        let dim = Color32::from_rgb(
                            (rgb[0] as u16 * 6 / 10) as u8,
                            (rgb[1] as u16 * 6 / 10) as u8,
                            (rgb[2] as u16 * 6 / 10) as u8,
                        );
                        for other in &world.organisms {
                            if other.id == org.id {
                                continue;
                            }
                            if other.species_fingerprint.as_deref() == Some(fp) {
                                push_cells(highlight, other, dim);
                            }
                        }
                    }
                }
            }
        }

        if self.show_species {
            let mut open = self.show_species;
            egui::Window::new("物种（时间轴）")
                .resizable(true)
                .default_size([430.0, 400.0])
                .open(&mut open)
                .default_pos([10.0, 420.0])
                .show(ctx, |ui| {
                    species_body(ui, world, self, highlight);
                });
            self.show_species = open;
        }

        if self.show_chart {
            let mut open = self.show_chart;
            egui::Window::new("曲线")
                .resizable(false)
                .collapsible(true)
                .open(&mut open)
                .default_pos([1080.0, 330.0])
                .show(ctx, |ui| {
                    chart_body(ui, self);
                });
            self.show_chart = open;
        }

        act
    }
}

fn inspector_body(
    ui: &mut egui::Ui,
    world: &World,
    org: &evosim_core::organism::Organism,
    panels: &mut Panels,
    act: &mut PanelActions,
) {
    let cfg = &world.config;
    let sp_name = org
        .species_fingerprint
        .as_deref()
        .and_then(|fp| world.species.by_fingerprint(fp))
        .map(|s| s.name.clone())
        .unwrap_or_else(|| "?".to_string());

    ui.label(
        egui::RichText::new(format!("个体 #{} · 物种 {}", org.id, sp_name))
            .strong()
            .color(TEXT),
    );
    ui.label(
        egui::RichText::new(format!(
            "位置 ({}, {}) · 朝向 {} · 身体朝向 {}",
            org.col, org.row, org.direction, org.rotation
        ))
        .color(TEXT_DIM),
    );

    ui.add_space(3.0);
    egui::Grid::new("org-stats")
        .num_columns(2)
        .spacing([10.0, 2.0])
        .show(ui, |ui| {
            ui.label("能量");
            ui.label(format!(
                "{:.1} / {:.0}",
                org.energy,
                org.reproduce_at(cfg)
            ));
            ui.end_row();
            ui.label("年龄");
            ui.label(format!("{} / {:.0}", org.age, org.lifespan(cfg)));
            ui.end_row();
            ui.label("生命");
            ui.label(format!(
                "{:.1} / {:.0}",
                org.max_health(cfg) - org.damage,
                org.max_health(cfg)
            ));
            ui.end_row();
            ui.label("体型");
            ui.label(format!("{} 格", org.cell_count()));
            ui.end_row();
            ui.label("变异率");
            ui.label(format!("{:.1}%", org.mutability));
            ui.end_row();
        });

    ui.add_space(3.0);
    ui.label(egui::RichText::new("结构").color(TEXT_DIM));
    ui.label(org.anatomy.describe());

    // 能力徽章
    ui.horizontal_wrapped(|ui| {
        for (bit, name) in [
            (Flags::MOVER, "移动"),
            (Flags::EYE, "视觉"),
            (Flags::MOUTH, "进食"),
            (Flags::PRODUCER, "造食"),
            (Flags::KILLER, "攻击"),
            (Flags::ARMOR, "装甲"),
            (Flags::STORAGE, "储备"),
            (Flags::REGEN, "再生"),
            (Flags::SPIKE, "反伤"),
        ] {
            if org.anatomy.has(bit) {
                ui.label(egui::RichText::new(name).color(ACCENT).size(11.0));
            }
        }
    });

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui
            .selectable_label(panels.pinned, "图钉")
            .on_hover_text("锁定检视目标：锁上后用「检视」点画布不会切换、也不会清空当前个体")
            .clicked()
        {
            panels.pinned = !panels.pinned;
        }
        if ui
            .selectable_label(panels.follow, "跟随")
            .on_hover_text("相机每帧对准这个个体")
            .clicked()
        {
            panels.follow = !panels.follow;
        }
        if ui.button("居中").clicked() {
            act.center_on = Some(org.id);
        }
        if ui.button("关闭").clicked() {
            act.clear_selection = true;
        }
    });
    ui.checkbox(&mut panels.highlight_species, "高亮同物种")
        .on_hover_text("除了它自己，把它物种的其他个体也用该物种的颜色标出来");
    if panels.pinned {
        ui.label(
            egui::RichText::new("已锁定：点画布不会换目标")
                .color(ACCENT)
                .size(11.0),
        );
    }
}

fn species_body(
    ui: &mut egui::Ui,
    world: &World,
    panels: &mut Panels,
    highlight: &mut Vec<(egui::Rect, Color32)>,
) {
    let reg = &world.species;
    let now = world.tick.max(1);

    ui.label(
        egui::RichText::new(format!(
            "tick {} · 存活 {} · 累计 {} · 峰值存活 {} · 已灭绝 {}",
            now, reg.alive_count, reg.total_created, reg.peak_alive_count, reg.total_extinct
        ))
        .color(TEXT_DIM)
        .size(11.0),
    );
    ui.label(
        egui::RichText::new("每条横条是一个物种的存续区间：左端=出现，右端=灭绝（还活着的顶到最右）")
            .color(TEXT_DIM)
            .size(10.0),
    );
    ui.separator();

    // 排序：还活着的排前面（按当前个体数降序），灭绝的按出现时间排在后面
    let mut rows: Vec<&evosim_core::species::Species> = reg.list.iter().collect();
    rows.sort_by(|a, b| {
        (b.population > 0)
            .cmp(&(a.population > 0))
            .then(b.population.cmp(&a.population))
            .then(a.start_tick.cmp(&b.start_tick))
    });

    // 时间轴刻度
    let axis_w = (ui.available_width() - 6.0).max(180.0);
    let (axis_rect, _) = ui.allocate_exact_size(egui::vec2(axis_w, 12.0), Sense::hover());
    {
        let p = ui.painter();
        p.text(
            axis_rect.left_top(),
            Align2::LEFT_TOP,
            "0",
            FontId::proportional(9.0),
            TEXT_DIM,
        );
        p.text(
            axis_rect.right_top(),
            Align2::RIGHT_TOP,
            format!("{}", now),
            FontId::proportional(9.0),
            TEXT_DIM,
        );
        p.line_segment(
            [
                egui::pos2(axis_rect.left(), axis_rect.bottom()),
                egui::pos2(axis_rect.right(), axis_rect.bottom()),
            ],
            Stroke::new(1.0, BORDER),
        );
    }

    let mut clicked: Option<String> = None;

    egui::ScrollArea::vertical()
        .max_height(300.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for sp in rows.iter().take(150) {
                let selected = panels
                    .selected_species
                    .as_deref()
                    .map(|fp| fp == sp.fingerprint.as_ref())
                    .unwrap_or(false);

                let (rect, resp) =
                    ui.allocate_exact_size(egui::vec2(axis_w, 15.0), Sense::click());
                let painter = ui.painter();

                // 行底色（悬停时提亮一点，能看出可点）
                if resp.hovered() || selected {
                    painter.rect_filled(
                        rect,
                        CornerRadius::ZERO,
                        Color32::from_rgb(26, 33, 45),
                    );
                }

                // 左：名字
                painter.text(
                    rect.left_center() + egui::vec2(3.0, 0.0),
                    Align2::LEFT_CENTER,
                    &sp.name,
                    FontId::proportional(10.0),
                    if selected { ACCENT } else { TEXT },
                );

                // 右：时间条
                let bar_x0 = rect.left() + 46.0;
                let bar_x1 = rect.right() - 30.0;
                let span = (bar_x1 - bar_x0).max(10.0);
                let end = if sp.end_tick >= 0 {
                    sp.end_tick as u64
                } else {
                    now
                };
                let t0 = (sp.start_tick.min(now) as f32 / now as f32).clamp(0.0, 1.0);
                let t1 = (end.min(now) as f32 / now as f32).clamp(0.0, 1.0);
                let x0 = bar_x0 + span * t0;
                let x1 = (bar_x0 + span * t1).max(x0 + 2.0);

                let rgb = species_rgb(sp.fingerprint.as_ref());
                let alpha = if sp.alive { 210 } else { 90 };
                painter.rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(x0, rect.top() + 4.0),
                        egui::pos2(x1, rect.bottom() - 4.0),
                    ),
                    CornerRadius::ZERO,
                    Color32::from_rgba_unmultiplied(rgb[0], rgb[1], rgb[2], alpha),
                );

                // 右端：当前个体数（条宽表示存续时长，数字表示规模）
                if sp.population > 0 {
                    painter.text(
                        rect.right_center() - egui::vec2(3.0, 0.0),
                        Align2::RIGHT_CENTER,
                        format!("{}", sp.population),
                        FontId::proportional(9.0),
                        TEXT_DIM,
                    );
                }

                if selected {
                    painter.rect_stroke(
                        rect,
                        CornerRadius::ZERO,
                        Stroke::new(1.0, ACCENT),
                        StrokeKind::Inside,
                    );
                }

                if resp.clicked() {
                    clicked = Some(sp.fingerprint.to_string());
                }
                resp.on_hover_text(format!(
                    "{}（{} 格）\n出现 tick {} → {}\n存续 {} tick · 当前 {} · 峰值 {} · 累计 {}",
                    sp.name,
                    sp.cell_count,
                    sp.start_tick,
                    if sp.alive {
                        "存活中".to_string()
                    } else {
                        format!("灭绝 tick {}", sp.end_tick)
                    },
                    sp.lifespan(),
                    sp.population,
                    sp.peak_population,
                    sp.cumulative_pop
                ));
            }
        });

    if let Some(fp) = clicked {
        if panels.selected_species.as_deref() == Some(fp.as_str()) {
            panels.selected_species = None;
        } else {
            panels.selected_species = Some(fp);
        }
    }

    // 高亮：选中物种的所有存活个体
    if let Some(fp) = panels.selected_species.clone() {
        let rgb = species_rgb(&fp);
        let color = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
        for org in &world.organisms {
            if org.species_fingerprint.as_deref() != Some(fp.as_str()) {
                continue;
            }
            push_cells(highlight, org, color);
        }
    }
}

/// 把一个个体的所有身体格推进高亮列表
fn push_cells(
    out: &mut Vec<(egui::Rect, Color32)>,
    org: &evosim_core::organism::Organism,
    color: Color32,
) {
    for c in &org.anatomy.cells {
        let col = org.real_col(c);
        let row = org.real_row(c);
        out.push((
            egui::Rect::from_min_size(egui::pos2(col as f32, row as f32), egui::vec2(1.0, 1.0)),
            color,
        ));
    }
}

fn chart_body(ui: &mut egui::Ui, panels: &Panels) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(280.0, 96.0), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::ZERO, Color32::from_rgb(13, 17, 23));
    painter.rect_stroke(
        rect,
        CornerRadius::ZERO,
        Stroke::new(1.0, BORDER),
        StrokeKind::Inside,
    );

    let h = &panels.history;
    if h.len() < 2 {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "采样中…",
            FontId::proportional(11.0),
            TEXT_DIM,
        );
        return;
    }

    let max_pop = h.iter().map(|s| s.pop).fold(1.0f32, f32::max);
    let max_food = h.iter().map(|s| s.food).fold(1.0f32, f32::max);
    let max_species = h.iter().map(|s| s.species).fold(1.0f32, f32::max);
    let n = h.len();

    let plot = |vals: Vec<f32>, max: f32, _color: Color32| -> Vec<egui::Pos2> {
        vals.iter()
            .enumerate()
            .map(|(i, v)| {
                let x = rect.left() + rect.width() * (i as f32 / (n - 1) as f32);
                let y = rect.bottom() - rect.height() * (v / max).clamp(0.0, 1.0);
                egui::pos2(x, y)
            })
            .collect()
    };

    let pop_pts = plot(h.iter().map(|s| s.pop).collect(), max_pop, ACCENT);
    let food_pts = plot(h.iter().map(|s| s.food).collect(), max_food, Color32::from_rgb(125, 147, 168));
    let sp_pts = plot(
        h.iter().map(|s| s.species).collect(),
        max_species,
        Color32::from_rgb(216, 180, 254),
    );

    painter.add(egui::Shape::line(
        food_pts,
        Stroke::new(1.0, Color32::from_rgb(125, 147, 168)),
    ));
    painter.add(egui::Shape::line(
        sp_pts,
        Stroke::new(1.0, Color32::from_rgb(216, 180, 254)),
    ));
    painter.add(egui::Shape::line(pop_pts, Stroke::new(1.5, ACCENT)));

    painter.text(
        rect.left_top() + egui::vec2(4.0, 3.0),
        Align2::LEFT_TOP,
        format!("种群 {} · 食物 {} · 物种 {}", max_pop as i32, max_food as i32, max_species as i32),
        FontId::proportional(10.0),
        TEXT_DIM,
    );
    let _ = PANEL;
}

#[cfg(test)]
mod tests {
    use super::*;
    use evosim_core::config::Config;
    use evosim_core::factory::{seed_origin_of_life, spawn_population};
    use evosim_core::rng::Rng;
    use evosim_core::world::World;
    use evosim_core::worldgen::generate_demo_world;

    fn live_world() -> World {
        let mut w = World::new(Config::default(), Rng::new(1001));
        generate_demo_world(&mut w, 1.0, 1.0);
        seed_origin_of_life(&mut w);
        spawn_population(&mut w, 8, 3);
        for _ in 0..150 {
            w.step();
        }
        w
    }

    /// 物种面板要真的画出时间轴（横条），不能只剩一行计数。
    #[test]
    fn species_timeline_draws_bars() {
        let world = live_world();
        assert!(world.species.size() > 0, "这个种子应该已经出现物种");

        let ctx = egui::Context::default();
        crate::ui::configure(&ctx);
        let mut panels = Panels::default();
        panels.show_inspector = false;
        panels.show_chart = false;
        panels.show_species = true;

        let mut hl: Vec<(egui::Rect, Color32)> = Vec::new();
        let mut input = egui::RawInput::default();
        input.screen_rect = Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1600.0, 950.0),
        ));

        // 两帧：egui 的窗口第一帧是淡入，形状会被剔成 Noop
        let _ = ctx.run(input.clone(), |ctx| {
            let _ = panels.draw(ctx, &world, &mut hl);
        });
        let out = ctx.run(input, |ctx| {
            let _ = panels.draw(ctx, &world, &mut hl);
        });

        fn walk<'a>(shapes: &'a [egui::Shape], rects: &mut Vec<&'a egui::epaint::RectShape>, texts: &mut Vec<&'a egui::epaint::TextShape>) {
            for s in shapes {
                match s {
                    egui::Shape::Rect(r) => rects.push(r),
                    egui::Shape::Text(t) => texts.push(t),
                    egui::Shape::Vec(v) => walk(v, rects, texts),
                    _ => {}
                }
            }
        }
        let flat: Vec<egui::Shape> = out.shapes.iter().map(|c| c.shape.clone()).collect();
        let (mut rects, mut texts) = (Vec::new(), Vec::new());
        walk(&flat, &mut rects, &mut texts);

        // 时间轴横条：细长、有填充色（物种指纹色），且不是窗口底色
        let bars = rects
            .iter()
            .filter(|r| {
                r.rect.height() <= 10.0
                    && r.rect.width() >= 2.0
                    && r.fill.a() > 0
                    && r.fill != crate::ui::BG
            })
            .count();
        assert!(bars > 0, "物种面板没有画出任何时间轴横条（rects={}）", rects.len());

        // 刻度上要有当前 tick 的数字
        let now = world.tick.to_string();
        assert!(
            texts.iter().any(|t| t.galley.text() == now),
            "时间轴右端应标出当前 tick（{}）",
            now
        );
    }
}
