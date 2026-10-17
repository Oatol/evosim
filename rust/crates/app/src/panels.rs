//! 检视器 / 物种面板 / 曲线 —— M3 剩余部分的第一批。
//!
//! 全部用 egui 画（曲线与条形图用 `painter` 自绘）。高亮由主程序算成屏幕矩形，
//! 这里只负责描边——这样不用改着色器，也不会碰世界状态。

use std::collections::VecDeque;

use egui::{Align2, Color32, CornerRadius, FontId, Sense, Stroke, StrokeKind};

use evosim_core::world::World;
use evosim_render::colors::species_rgb;

use crate::ui::{ACCENT, BORDER, TEXT, TEXT_DIM};

/// 一条历史采样（曲线用）。
///
/// 字段都是 `f32`：曲线只画趋势，不需要 `f64` 的精度，而且几千个点
/// 每个省 4 字节是白捡的。
#[derive(Clone, Copy)]
pub struct Sample {
    pub tick: u64,
    pub pop: f32,
    pub food: f32,
    pub species: f32,
    /// 平均体型（格）
    pub body: f32,
    /// 平均变异率（%）
    pub mutability: f32,
    /// 平均年龄（tick）
    pub age: f32,
}

/// 历史点上限。1200 个点 × 每条 36 字节 ≈ 43 KB，可以忽略；
/// 再多就没意义了——屏幕上横着最多也就 700 个像素列。
pub const MAX_SAMPLES: usize = 1200;

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
    /// 神经网络图是否画全部权重（默认只画强连接）
    pub brain_show_all: bool,
    /// 曲线：每个指标是否显示（顺序见 `chart::METRICS`）
    pub chart_on: [bool; 6],
    /// 曲线：视图窗口，单位是**采样点下标**（不是 tick）。`None` = 全览
    pub chart_view: Option<(f32, f32)>,
    /// 曲线：新采样进来时视图自动跟到最右
    pub chart_follow: bool,
    /// 采样间隔（tick）。历史满了会翻倍并抽稀，见 `sample()`
    pub sample_interval: u64,
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
            brain_show_all: false,
            chart_on: [true, true, true, false, false, false],
            chart_view: None,
            chart_follow: true,
            sample_interval: 20,
        }
    }
}

#[derive(Default)]
pub struct PanelActions {
    pub center_on: Option<u32>,
    pub clear_selection: bool,
}

impl Panels {
    /// 世界被整体替换（新建 / 读档）时调用：清掉所有**属于旧世界**的状态。
    ///
    /// 窗口开关（`show_inspector` 等）**保留** —— 用户开着的面板不该因为
    /// 换了世界就自己关掉。这里只清"引用了旧世界数据"的东西：
    /// 选中的个体 id、跟随/图钉、选中的物种指纹、曲线历史。
    pub fn reset_world_state(&mut self) {
        self.selected_org = None;
        self.follow = false;
        self.pinned = false;
        self.selected_species = None;
        self.history.clear();
        self.last_sample_tick = 0;
        self.sample_interval = 20;
        self.chart_view = None;
    }

    /// 采样一个点。
    ///
    /// 间隔会**自适应**：历史满了就把间隔翻倍、隔一个丢一个，而不是丢掉最老的点。
    /// 这样跑十万 tick 也留着完整的演化史（只是越早越稀），比"只留最近一万 tick"
    /// 有用得多——灭绝事件往往在很后面才看出意义。
    pub fn sample(&mut self, world: &World) {
        if world.tick < self.last_sample_tick + self.sample_interval {
            return;
        }
        self.last_sample_tick = world.tick;
        let stats = world.snapshot_stats();
        let org = world.organism_stats();
        self.history.push_back(Sample {
            tick: world.tick,
            pop: stats.organisms as f32,
            food: stats.food as f32,
            species: world.species.size() as f32,
            body: org.avg_cells as f32,
            mutability: org.avg_mutability as f32,
            age: org.avg_age as f32,
        });

        if self.history.len() > MAX_SAMPLES {
            // 抽稀：保留下标为偶数的点（含最后一个），间隔翻倍
            let mut i = 0usize;
            self.history.retain(|_| {
                let keep = i % 2 == 0;
                i += 1;
                keep
            });
            self.sample_interval = self.sample_interval.saturating_mul(2).min(1 << 20);
            // 视图是按"采样点下标"记的，抽稀后下标含义变了，直接复位最安全
            self.chart_view = None;
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
                        // `resizable(false)` 下 default_width 只是建议，必须钉住 min/max
                        .min_width(452.0)
                        .max_width(452.0)
                        .open(&mut open)
                        // 放在工具栏右侧，别被工具窗挡住
                        .default_pos([230.0, 42.0])
                        .show(ctx, |ui| {
                            crate::inspector::draw_body(ui, world, org, self, &mut act);
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
                .collapsible(false)
                .min_width(560.0)
                .max_width(560.0)
                .open(&mut open)
                .default_pos([1000.0, 320.0])
                .show(ctx, |ui| {
                    crate::chart::draw(ui, self);
                });
            self.show_chart = open;
        }

        act
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
            Stroke::new(1.0_f32, BORDER),
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
                        Stroke::new(1.0_f32, ACCENT),
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
