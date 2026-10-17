//! egui 界面：顶部状态条、左侧工具/笔刷、右侧参数、右下悬停读数。
//!
//! 风格刻意压紧（12px 字、行距 4、无圆角、1px 描边）——目标是 TPT 那种密度，
//! 而不是 egui 默认的"通用桌面软件"观感。笔刷方块是**自绘控件**（painter 直接画），
//! 这也是 M3 计划里"关键控件自绘"的落点。

use std::sync::Arc;

use egui::{Align2, Color32, CornerRadius, FontFamily, FontId, Sense, Stroke, StrokeKind};

use evosim_core::cell_type::{self, ct, TERRAIN_BRUSH_TYPES};
use evosim_core::world::World;
use evosim_render::colors::build_palette;

use crate::paint::ERASER;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Paint,
    Pan,
}

pub struct UiState {
    pub brush: u8,
    pub brush_size: i32,
    pub tool: Tool,
    pub hover: String,
    pub show_tools: bool,
    pub show_params: bool,
}

impl Default for UiState {
    fn default() -> Self {
        UiState {
            brush: ct::WALL,
            brush_size: 1,
            tool: Tool::Paint,
            hover: String::new(),
            show_tools: true,
            show_params: true,
        }
    }
}

pub struct HudInfo {
    pub fps: f32,
    pub tps: f32,
    pub speed: f32,
    pub paused: bool,
    pub mode: u32,
}

#[derive(Default)]
pub struct UiActions {
    pub toggle_pause: bool,
    pub step_once: bool,
    pub speed_delta: i32,
    pub toggle_mode: bool,
    pub fit: bool,
    pub wants_pointer: bool,
}

const BG: Color32 = Color32::from_rgb(13, 17, 23);
const PANEL: Color32 = Color32::from_rgb(20, 26, 36);
const BORDER: Color32 = Color32::from_rgb(45, 55, 72);
const TEXT: Color32 = Color32::from_rgb(214, 222, 235);
const TEXT_DIM: Color32 = Color32::from_rgb(130, 143, 160);
const ACCENT: Color32 = Color32::from_rgb(56, 189, 248);

/// 加载中文字体（优先微软雅黑，退回黑体）。找不到就用 egui 内置字体（中文会缺字）。
pub fn configure(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    let candidates = [
        ("C:/Windows/Fonts/msyh.ttc", 0u32),
        ("C:/Windows/Fonts/simhei.ttf", 0u32),
        ("C:/Windows/Fonts/Deng.ttf", 0u32),
    ];
    for (path, index) in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            let bytes_len = bytes.len();
            let mut data = egui::FontData::from_owned(bytes);
            data.index = index;
            fonts
                .font_data
                .insert("cjk".to_owned(), Arc::new(data));
            for family in [FontFamily::Proportional, FontFamily::Monospace] {
                fonts
                    .families
                    .entry(family)
                    .or_default()
                    .insert(0, "cjk".to_owned());
            }
            println!("界面字体：{}（{} KB）", path, bytes_len / 1024);
            break;
        }
    }
    ctx.set_fonts(fonts);

    let mut style = (*ctx.style()).clone();

    // ── 密度：比 egui 默认紧一档 ──
    style.spacing.item_spacing = egui::vec2(6.0, 4.0);
    style.spacing.button_padding = egui::vec2(7.0, 3.0);
    style.spacing.interact_size.y = 20.0;
    style.spacing.slider_width = 132.0;
    style.spacing.window_margin = egui::Margin::same(8);
    style.text_styles = [
        (egui::TextStyle::Heading, FontId::proportional(13.0)),
        (egui::TextStyle::Body, FontId::proportional(12.0)),
        (egui::TextStyle::Monospace, FontId::monospace(12.0)),
        (egui::TextStyle::Button, FontId::proportional(12.0)),
        (egui::TextStyle::Small, FontId::proportional(11.0)),
    ]
    .into();

    // ── 配色：深底 + 细边 ──
    let v = &mut style.visuals;
    v.dark_mode = true;
    v.panel_fill = PANEL;
    v.window_fill = PANEL;
    v.extreme_bg_color = BG;
    v.faint_bg_color = Color32::from_rgb(26, 33, 45);
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.selection.bg_fill = Color32::from_rgb(30, 64, 92);
    v.override_text_color = Some(TEXT);

    for w in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = CornerRadius::ZERO;
        w.bg_stroke = Stroke::new(1.0, BORDER);
    }
    v.widgets.inactive.weak_bg_fill = Color32::from_rgb(26, 33, 45);
    v.widgets.inactive.bg_fill = Color32::from_rgb(26, 33, 45);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(36, 46, 62);
    v.widgets.hovered.bg_fill = Color32::from_rgb(36, 46, 62);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, ACCENT);
    v.widgets.active.weak_bg_fill = Color32::from_rgb(30, 64, 92);
    v.widgets.active.bg_fill = Color32::from_rgb(30, 64, 92);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);

    ctx.set_style(style);
}

pub fn draw(ctx: &egui::Context, st: &mut UiState, world: &mut World, hud: &HudInfo) -> UiActions {
    let mut act = UiActions::default();

    // ── 顶部状态条 ──
    egui::TopBottomPanel::top("hud")
        .frame(egui::Frame::NONE.fill(BG).stroke(Stroke::new(1.0, BORDER)).inner_margin(egui::Margin::symmetric(10, 5)))
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                let stats = world.snapshot_stats();
                ui.label(format!("tick {}", world.tick));
                ui.separator();
                ui.label(format!("{} 只", stats.organisms));
                ui.separator();
                ui.label(format!("食物 {}", stats.food));
                ui.separator();
                ui.label(format!("物种 {}", world.species.size()));
                ui.separator();
                ui.label(if hud.paused { "已暂停" } else { "运行中" });
                ui.separator();
                ui.label(format!("{:.0} fps · {:.0} tps", hud.fps, hud.tps));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("适应窗口 F").clicked() {
                        act.fit = true;
                    }
                    if ui
                        .button(if hud.mode == 1 { "着色：物种 Tab" } else { "着色：类型 Tab" })
                        .clicked()
                    {
                        act.toggle_mode = true;
                    }
                    if ui.button(if hud.paused { "继续 空格" } else { "暂停 空格" }).clicked() {
                        act.toggle_pause = true;
                    }
                    if ui.button("单步 S").clicked() {
                        act.step_once = true;
                    }
                    if ui.button("◀").clicked() {
                        act.speed_delta = -1;
                    }
                    ui.label(format!("{:.0}×", hud.speed));
                    if ui.button("▶").clicked() {
                        act.speed_delta = 1;
                    }
                });
            });
        });

    // ── 左侧：工具 + 笔刷 ──
    if st.show_tools {
        egui::Window::new("工具")
            .resizable(false)
            .collapsible(false)
            .default_pos([10.0, 42.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.selectable_label(st.tool == Tool::Paint, "画笔").clicked() {
                        st.tool = Tool::Paint;
                    }
                    if ui.selectable_label(st.tool == Tool::Pan, "平移").clicked() {
                        st.tool = Tool::Pan;
                    }
                });
                ui.add_space(2.0);
                ui.label(egui::RichText::new("笔刷").color(TEXT_DIM));

                // 两列自绘方块
                let palette = build_palette();
                let mut ids: Vec<u8> = TERRAIN_BRUSH_TYPES.to_vec();
                ids.push(ERASER);

                for chunk in ids.chunks(2) {
                    ui.horizontal(|ui| {
                        for &id in chunk {
                            brush_tile(ui, st, &palette, id);
                        }
                    });
                }

                ui.add_space(2.0);
                ui.add(egui::Slider::new(&mut st.brush_size, 1..=12).text("大小"));
                if st.brush != ERASER {
                    let name = cell_type::label(st.brush);
                    ui.label(
                        egui::RichText::new(format!("正在画：{}", name))
                            .color(TEXT_DIM)
                            .size(11.0),
                    );
                } else {
                    ui.label(
                        egui::RichText::new("正在擦：地形（生物与食物不动）")
                            .color(TEXT_DIM)
                            .size(11.0),
                    );
                }
            });
    }

    // ── 右侧：关键参数 ──
    if st.show_params {
        egui::Window::new("参数")
            .resizable(false)
            .collapsible(false)
            .default_pos([1100.0, 42.0])
            .show(ctx, |ui| {
                let cfg = &mut world.config;
                ui.add(
                    egui::Slider::new(&mut cfg.life.metabolism_scale, 0.0..=5.0)
                        .text("全局消耗量倍率")
                        .step_by(0.05),
                )
                .on_hover_text("乘在所有细胞的每 tick 消耗上。0 = 关闭。");
                ui.add(
                    egui::Slider::new(&mut cfg.life.food_energy_scale, 0.1..=10.0)
                        .text("食物量倍率")
                        .step_by(0.1),
                )
                .on_hover_text("一份食物能提供多少能量（默认 1）。");
                ui.add(
                    egui::Slider::new(&mut cfg.life.food_prod_prob, 0.0..=100.0)
                        .text("生产者造食率")
                        .step_by(1.0),
                )
                .on_hover_text("生产者细胞每 tick 生成食物的概率（%）。");
                ui.separator();
                ui.checkbox(&mut cfg.mutation.use_global_mutability, "启用全局变异率");
                ui.add(
                    egui::Slider::new(&mut cfg.mutation.global_mutability, 1.0..=100.0)
                        .text("全局变异率")
                        .step_by(1.0),
                );
                ui.add(
                    egui::Slider::new(&mut cfg.life.max_organisms, -1..=5000)
                        .text("种群上限")
                        .step_by(50.0),
                )
                .on_hover_text("-1 = 无上限");
                ui.add(
                    egui::Slider::new(&mut cfg.life.lifespan_multiplier, 1.0..=1000.0)
                        .text("寿命倍率")
                        .step_by(1.0),
                );
                ui.separator();
                ui.label(
                    egui::RichText::new("改完立刻生效（每 tick 现读）")
                        .color(TEXT_DIM)
                        .size(11.0),
                );
            });
    }

    // ── 右下：悬停读数 ──
    if !st.hover.is_empty() {
        egui::Area::new("hover".into())
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-10.0, -10.0))
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(Color32::from_rgba_unmultiplied(13, 17, 23, 230))
                    .stroke(Stroke::new(1.0, BORDER))
                    .inner_margin(egui::Margin::symmetric(8, 4))
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new(&st.hover).size(11.0).color(TEXT));
                    });
            });
    }

    act.wants_pointer = ctx.wants_pointer_input();
    act
}

/// 一个笔刷方块：色块 + 名字 + 选中描边（自绘）
fn brush_tile(ui: &mut egui::Ui, st: &mut UiState, palette: &[[u8; 4]], id: u8) {
    let size = egui::vec2(84.0, 24.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    let selected = st.brush == id;

    let color = if id == ERASER {
        Color32::from_rgb(203, 213, 225)
    } else {
        let p = palette[(id as usize).min(255)];
        Color32::from_rgb(p[0], p[1], p[2])
    };

    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::ZERO, Color32::from_rgb(26, 33, 45));
    // 色块
    let swatch = egui::Rect::from_min_size(
        rect.min + egui::vec2(4.0, 4.0),
        egui::vec2(16.0, 16.0),
    );
    painter.rect_filled(swatch, CornerRadius::ZERO, color);
    painter.rect_stroke(
        swatch,
        CornerRadius::ZERO,
        Stroke::new(1.0, Color32::from_rgb(0, 0, 0)),
        StrokeKind::Inside,
    );

    let name = if id == ERASER {
        "橡皮擦"
    } else {
        cell_type::label(id)
    };
    painter.text(
        rect.min + egui::vec2(26.0, rect.height() * 0.5),
        Align2::LEFT_CENTER,
        name,
        FontId::proportional(12.0),
        if selected { ACCENT } else { TEXT },
    );

    painter.rect_stroke(
        rect,
        CornerRadius::ZERO,
        Stroke::new(if selected { 2.0 } else { 1.0 }, if selected { ACCENT } else { BORDER }),
        StrokeKind::Inside,
    );

    let desc = if id == ERASER {
        "只擦地形（墙/岩浆/沼泽/辐射带/腐殖质），食物与生物一律不动".to_string()
    } else {
        cell_type::def(id)
            .map(|d| d.desc.to_string())
            .unwrap_or_default()
    };
    resp.on_hover_text(desc).clicked().then(|| st.brush = id);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ui_builds_without_panic() {
        let ctx = egui::Context::default();
        configure(&ctx);

        let mut world = evosim_core::world::World::new(
            evosim_core::config::Config::default(),
            evosim_core::rng::Rng::new(1),
        );
        let mut st = UiState::default();
        st.hover = "(12, 34) 嘴 · 个体 #3 · S1".to_string();

        let hud = HudInfo {
            fps: 60.0,
            tps: 60.0,
            speed: 1.0,
            paused: false,
            mode: 0,
        };

        let mut input = egui::RawInput::default();
        input.screen_rect = Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1600.0, 950.0),
        ));

        let out = ctx.run(input, |ctx| {
            let _ = draw(ctx, &mut st, &mut world, &hud);
        });

        assert!(!out.shapes.is_empty(), "界面没有产出任何图形");
    }

    #[test]
    fn brush_list_covers_registry_and_eraser() {
        // 笔刷列表必须来自注册表 + 橡皮擦，不能手写漏项
        let mut ids: Vec<u8> = evosim_core::cell_type::TERRAIN_BRUSH_TYPES.to_vec();
        ids.push(crate::paint::ERASER);
        assert!(ids.contains(&evosim_core::cell_type::ct::WALL));
        assert!(ids.contains(&evosim_core::cell_type::ct::LAVA));
        assert!(ids.contains(&crate::paint::ERASER));
        assert_eq!(ids.len(), 8, "7 个地形笔刷 + 橡皮擦");
    }
}
