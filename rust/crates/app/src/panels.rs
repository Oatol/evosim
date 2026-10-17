//! 检视器 / 物种面板 / 曲线 —— M3 剩余部分的第一批。
//!
//! 全部用 egui 画（曲线与条形图用 `painter` 自绘）。高亮由主程序算成屏幕矩形，
//! 这里只负责描边——这样不用改着色器，也不会碰世界状态。

use std::collections::VecDeque;

use egui::{Align2, Color32, CornerRadius, FontId, Sense, Stroke, StrokeKind};

use evosim_core::anatomy::Flags;
use evosim_core::cell_type::{self, ct};
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
                }
            }
        }
    }

    pub fn draw(
        &mut self,
        ctx: &egui::Context,
        world: &World,
        highlight: &mut Vec<egui::Rect>,
    ) -> PanelActions {
        let mut act = PanelActions::default();

        if self.show_inspector {
            if let Some(id) = self.selected_org {
                if let Some(org) = world.organism(id) {
                    egui::Window::new("检视器")
                        .resizable(false)
                        .collapsible(false)
                        .default_pos([10.0, 300.0])
                        .show(ctx, |ui| {
                            inspector_body(ui, world, org, self, &mut act);
                        });
                }
            }
        }

        if self.show_species {
            egui::Window::new("物种")
                .resizable(true)
                .default_size([300.0, 260.0])
                .default_pos([10.0, 470.0])
                .show(ctx, |ui| {
                    species_body(ui, world, self, highlight);
                });
        }

        if self.show_chart {
            egui::Window::new("曲线")
                .resizable(false)
                .collapsible(true)
                .default_pos([1100.0, 300.0])
                .show(ctx, |ui| {
                    chart_body(ui, self);
                });
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
}

fn species_body(
    ui: &mut egui::Ui,
    world: &World,
    panels: &mut Panels,
    highlight: &mut Vec<egui::Rect>,
) {
    let reg = &world.species;
    let mut alive: Vec<&evosim_core::species::Species> =
        reg.list.iter().filter(|s| s.population > 0).collect();
    alive.sort_by(|a, b| b.population.cmp(&a.population));

    ui.label(
        egui::RichText::new(format!(
            "存活 {} · 累计 {} · 峰值存活 {} · 已灭绝 {}",
            reg.alive_count, reg.total_created, reg.peak_alive_count, reg.total_extinct
        ))
        .color(TEXT_DIM)
        .size(11.0),
    );
    ui.separator();

    let max_pop = alive.first().map(|s| s.population).unwrap_or(1).max(1);
    let mut clicked: Option<String> = None;

    egui::ScrollArea::vertical()
        .max_height(220.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for sp in alive.iter().take(60) {
                let selected = panels
                    .selected_species
                    .as_deref()
                    .map(|fp| fp == sp.fingerprint.as_ref())
                    .unwrap_or(false);

                let (rect, resp) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width().min(280.0), 18.0),
                    Sense::click(),
                );
                let painter = ui.painter();

                // 条形：长度 = 当前个体数 / 最大个体数
                let frac = sp.population as f32 / max_pop as f32;
                let bar = egui::Rect::from_min_size(
                    rect.min,
                    egui::vec2(rect.width() * frac, rect.height()),
                );
                let rgb = species_rgb(sp.fingerprint.as_ref());
                painter.rect_filled(
                    bar,
                    CornerRadius::ZERO,
                    Color32::from_rgba_unmultiplied(rgb[0], rgb[1], rgb[2], 90),
                );
                if selected {
                    painter.rect_stroke(
                        rect,
                        CornerRadius::ZERO,
                        Stroke::new(1.0, ACCENT),
                        StrokeKind::Inside,
                    );
                }
                painter.text(
                    rect.min + egui::vec2(4.0, rect.height() * 0.5),
                    Align2::LEFT_CENTER,
                    format!("{}  {}", sp.name, sp.description),
                    FontId::proportional(11.0),
                    if selected { ACCENT } else { TEXT },
                );
                painter.text(
                    rect.min + egui::vec2(rect.width() - 4.0, rect.height() * 0.5),
                    Align2::RIGHT_CENTER,
                    format!("{}", sp.population),
                    FontId::proportional(11.0),
                    TEXT_DIM,
                );

                if resp.clicked() {
                    clicked = Some(sp.fingerprint.to_string());
                }
                resp.on_hover_text(format!(
                    "{} · 峰值 {} · 累计 {}\n出现于 tick {}，{}",
                    sp.description,
                    sp.peak_population,
                    sp.cumulative_pop,
                    sp.start_tick,
                    if sp.alive { "存活中" } else { "已灭绝" }
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
        let _ = color;
        for org in &world.organisms {
            if org.species_fingerprint.as_deref() != Some(fp.as_str()) {
                continue;
            }
            for c in &org.anatomy.cells {
                let col = org.real_col(c);
                let row = org.real_row(c);
                highlight.push(egui::Rect::from_min_size(
                    egui::pos2(col as f32, row as f32),
                    egui::vec2(1.0, 1.0),
                ));
            }
        }
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

    let plot = |vals: Vec<f32>, max: f32, color: Color32| -> Vec<egui::Pos2> {
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
