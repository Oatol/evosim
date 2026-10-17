//! egui 界面：顶部状态条、左侧工具/笔刷、右侧参数、右下悬停读数，外加三个面板。
//!
//! 风格刻意压紧（12px 字、行距 4、无圆角、1px 描边）——目标是 TPT 那种密度，
//! 而不是 egui 默认的"通用桌面软件"观感。笔刷方块是**自绘控件**（painter 直接画）。

use std::sync::Arc;

use egui::{Align2, Color32, CornerRadius, FontFamily, FontId, Sense, Stroke, StrokeKind};

use evosim_core::cell_type::{self, ct, TERRAIN_BRUSH_TYPES};
use evosim_core::world::World;
use evosim_render::colors::build_palette;

use crate::paint::{BrushShape, ERASER};
use crate::panels::Panels;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Paint,
    Pan,
    Inspect,
}

pub struct UiState {
    pub brush: u8,
    pub brush_size: i32,
    pub brush_shape: BrushShape,
    pub tool: Tool,
    pub hover: String,
    pub show_tools: bool,
    pub show_params: bool,
    /// 世界坐标下的高亮矩形（1 格大小）+ 各自的颜色。
    /// 由面板填充，这里换算成屏幕像素再描边。
    pub highlights: Vec<(egui::Rect, Color32)>,
}

impl Default for UiState {
    fn default() -> Self {
        UiState {
            brush: ct::WALL,
            brush_size: 1,
            brush_shape: BrushShape::Square,
            tool: Tool::Paint,
            hover: String::new(),
            show_tools: true,
            show_params: true,
            highlights: Vec::new(),
        }
    }
}

/// 相机快照（把世界坐标换算成屏幕像素用）
#[derive(Clone, Copy)]
pub struct ViewInfo {
    pub cam_x: f32,
    pub cam_y: f32,
    pub zoom: f32,
    pub width: f32,
    pub height: f32,
}

pub struct HudInfo {
    pub fps: f32,
    pub tps: f32,
    pub speed: f32,
    pub paused: bool,
    pub mode: u32,
    pub workshop_open: bool,
}

#[derive(Default)]
pub struct UiActions {
    pub toggle_pause: bool,
    pub step_once: bool,
    pub speed_delta: i32,
    pub toggle_mode: bool,
    pub fit: bool,
    pub wants_pointer: bool,
    pub center_on: Option<u32>,
    pub clear_selection: bool,
    pub toggle_workshop: bool,
}

pub const BG: Color32 = Color32::from_rgb(13, 17, 23);
pub const PANEL: Color32 = Color32::from_rgb(20, 26, 36);
pub const BORDER: Color32 = Color32::from_rgb(45, 55, 72);
pub const TEXT: Color32 = Color32::from_rgb(214, 222, 235);
pub const TEXT_DIM: Color32 = Color32::from_rgb(130, 143, 160);
pub const ACCENT: Color32 = Color32::from_rgb(56, 189, 248);

/// 世界高亮所在的图层。
///
/// **必须是 `Order::Background`** —— egui 对这个 Order 的定义原文是
/// "Painted behind all floating windows"。换成 `Order::Middle`（窗口所在层）
/// 就会盖在物种工坊 / 检视器上面：`GraphicLayers::drain()` 在同一 Order 内
/// 先画登记成 Area 的窗口，再画"不是 Area"的图层，而我们的高亮不是 Area。
pub fn highlight_layer() -> egui::LayerId {
    egui::LayerId::new(egui::Order::Background, egui::Id::new("world-highlight"))
}

/// 加载中文字体（优先微软雅黑，退回黑体）。找不到就用 egui 内置字体（中文会缺字）。
pub fn configure(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    let candidates = [
        "C:/Windows/Fonts/msyh.ttc",
        "C:/Windows/Fonts/simhei.ttf",
        "C:/Windows/Fonts/Deng.ttf",
    ];
    for path in candidates {
        if let Ok(bytes) = std::fs::read(path) {
            let bytes_len = bytes.len();
            let mut data = egui::FontData::from_owned(bytes);
            data.index = 0;
            fonts.font_data.insert("cjk".to_owned(), Arc::new(data));
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

pub fn draw(
    ctx: &egui::Context,
    st: &mut UiState,
    world: &mut World,
    hud: &HudInfo,
    view: &ViewInfo,
    panels: &mut Panels,
) -> UiActions {
    let mut act = UiActions::default();

    // ── 世界高亮画在 **Order::Background** 上 ──
    //
    // 为什么不是 Middle（窗口所在的那一层）：
    //   1. `Context::layer_painter()` 只是构造一个 Painter，**不会**创建图层；
    //      图层要等到真正 add 图形时才登记，所以"帧首先登记一次"是无效的。
    //   2. 更关键的是 `GraphicLayers::drain()` 的绘制顺序：同一 Order 内先画
    //      登记在 `area_order` 里的 Area（窗口），再画"没登记成 Area"的图层。
    //      我们的高亮不是 Area，所以它**永远排在所有窗口之后** —— 这就是
    //      "高亮浮在工坊/检视器之上"的根因。
    //
    // Order::Background 的定义就是 "Painted behind all floating windows"，
    // 换到这一层才是语义正确、且不依赖任何登记顺序的做法。
    let hl_layer = highlight_layer();

    // ── 顶部状态条 ──
    egui::TopBottomPanel::top("hud")
        .frame(
            egui::Frame::NONE
                .fill(BG)
                .stroke(Stroke::new(1.0, BORDER))
                .inner_margin(egui::Margin::symmetric(10, 5)),
        )
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
                    if ui
                        .selectable_label(hud.workshop_open, "物种工坊")
                        .on_hover_text("手绘蓝图 + 生物库（快捷键 G）")
                        .clicked()
                    {
                        act.toggle_workshop = true;
                    }
                    if ui
                        .selectable_label(st.show_tools, "工具")
                        .on_hover_text("左侧工具栏与笔刷")
                        .clicked()
                    {
                        st.show_tools = !st.show_tools;
                    }
                    if ui.button("适应窗口").on_hover_text("快捷键 F").clicked() {
                        act.fit = true;
                    }
                    let mode_label = match hud.mode {
                        1 => "着色：物种",
                        2 => "着色：温度",
                        3 => "着色：辐射",
                        _ => "着色：类型",
                    };
                    if ui
                        .button(mode_label)
                        .on_hover_text("Tab 依次切换：类型 → 物种 → 温度 → 辐射")
                        .clicked()
                    {
                        act.toggle_mode = true;
                    }
                    if ui.button(if hud.paused { "继续" } else { "暂停" }).clicked() {
                        act.toggle_pause = true;
                    }
                    if ui.button("单步").clicked() {
                        act.step_once = true;
                    }
                    if ui.button("◀").clicked() {
                        act.speed_delta = -1;
                    }
                    ui.label(format!("{:.0}×", hud.speed));
                    if ui.button("▶").clicked() {
                        act.speed_delta = 1;
                    }
                    ui.separator();
                    ui.checkbox(&mut panels.show_inspector, "检视器");
                    ui.checkbox(&mut panels.show_species, "物种");
                    ui.checkbox(&mut panels.show_chart, "曲线");
                });
            });
        });

    // ── 左侧：工具 + 笔刷 ──
    if st.show_tools {
        let mut tools_open = st.show_tools;
        egui::Window::new("工具")
            .resizable(false)
            .collapsible(false)
            .open(&mut tools_open)
            .default_pos([10.0, 42.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.selectable_label(st.tool == Tool::Paint, "画笔").clicked() {
                        st.tool = Tool::Paint;
                    }
                    if ui.selectable_label(st.tool == Tool::Pan, "平移").clicked() {
                        st.tool = Tool::Pan;
                    }
                    if ui.selectable_label(st.tool == Tool::Inspect, "检视").clicked() {
                        st.tool = Tool::Inspect;
                    }
                });
                ui.add_space(2.0);
                ui.label(egui::RichText::new("笔刷").color(TEXT_DIM));

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
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new("形状").color(TEXT_DIM));
                    for shape in BrushShape::all() {
                        if ui
                            .selectable_label(st.brush_shape == shape, shape.label())
                            .clicked()
                        {
                            st.brush_shape = shape;
                        }
                    }
                });
                let hint = if st.brush == ERASER {
                    "正在擦：地形（生物与食物不动）".to_string()
                } else {
                    format!("正在画：{}", cell_type::label(st.brush))
                };
                ui.label(egui::RichText::new(hint).color(TEXT_DIM).size(11.0));

                // 选中笔刷的简介（文案来自注册表 desc，笔刷/工坊/参数共用一份）
                let desc = if st.brush == ERASER {
                    "只擦地形（墙/岩浆/沼泽/辐射带/腐殖质），食物与生物一律不动".to_string()
                } else {
                    cell_type::def(st.brush)
                        .map(|d| d.desc.to_string())
                        .unwrap_or_default()
                };
                if !desc.is_empty() {
                    ui.add_space(2.0);
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                    ui.set_max_width(190.0);
                    ui.label(
                        egui::RichText::new(desc)
                            .color(Color32::from_rgb(150, 165, 185))
                            .size(11.0),
                    );
                }
            });
        st.show_tools = tools_open;
    }

    // ── 右侧：关键参数 ──
    if st.show_params {
        let mut params_open = st.show_params;
        egui::Window::new("参数")
            .resizable(false)
            .collapsible(true)
            .open(&mut params_open)
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
                );
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
                );
                ui.add(
                    egui::Slider::new(&mut cfg.life.lifespan_multiplier, 1.0..=1000.0)
                        .text("寿命倍率")
                        .step_by(1.0),
                );
            });
        st.show_params = params_open;
    }

    // ── 面板：检视器 / 物种 / 曲线 ──
    st.highlights.clear();
    let panel_act = panels.draw(ctx, world, &mut st.highlights);

    // ── 世界坐标 → 屏幕像素，画高亮 ──
    //
    // 用帧首登记好的那一层（`hl_layer`），所以永远画在窗口**下面**。
    // 再按"顶栏以下的区域"裁剪一次，避免溢到顶栏上。
    if !st.highlights.is_empty() {
        let canvas = ctx.available_rect();
        let painter = ctx
            .layer_painter(hl_layer)
            .with_clip_rect(canvas.intersect(ctx.screen_rect()));
        let z = view.zoom;
        for (r, color) in &st.highlights {
            let sx = (r.min.x - view.cam_x) * z;
            let sy = (r.min.y - view.cam_y) * z;
            let size = (r.width() * z).max(1.0);
            if sx + size < 0.0 || sy + size < 0.0 || sx > view.width || sy > view.height {
                continue;
            }
            let rect = egui::Rect::from_min_size(egui::pos2(sx, sy), egui::vec2(size, size));
            if z >= 3.0 {
                painter.rect_stroke(
                    rect,
                    CornerRadius::ZERO,
                    Stroke::new(1.0, *color),
                    StrokeKind::Inside,
                );
            } else {
                painter.rect_filled(
                    rect,
                    CornerRadius::ZERO,
                    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 200),
                );
            }
        }
    }

    // ── 左下：场滤镜图例 ──
    if hud.mode >= 2 {
        let lut = if hud.mode == 2 {
            evosim_render::colors::temp_colormap()
        } else {
            evosim_render::colors::rad_colormap()
        };
        egui::Area::new("field-legend".into())
            .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(10.0, -10.0))
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(Color32::from_rgba_unmultiplied(13, 17, 23, 230))
                    .stroke(Stroke::new(1.0, BORDER))
                    .inner_margin(egui::Margin::symmetric(8, 4))
                    .show(ui, |ui| {
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(if hud.mode == 2 { "温度" } else { "辐射" })
                                    .size(11.0)
                                    .color(TEXT),
                            );
                            let (rect, _) = ui.allocate_exact_size(
                                egui::vec2(120.0, 10.0),
                                Sense::hover(),
                            );
                            // 用 24 段近似渐变（每段 5 项色表），比逐像素画便宜得多
                            let painter = ui.painter();
                            for k in 0..24 {
                                let c = lut[(k * 255 / 23).min(255)];
                                let x = rect.left() + rect.width() * (k as f32 / 24.0);
                                let w = rect.width() / 24.0 + 1.0;
                                painter.rect_filled(
                                    egui::Rect::from_min_size(
                                        egui::pos2(x, rect.top()),
                                        egui::vec2(w, rect.height()),
                                    ),
                                    CornerRadius::ZERO,
                                    Color32::from_rgb(c[0], c[1], c[2]),
                                );
                            }
                            ui.label(egui::RichText::new("高").size(11.0).color(TEXT_DIM));
                        });
                    });
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
                        // 不让它按窄宽度竖排：强制单行 + 给足最大宽度
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                        ui.set_max_width(760.0);
                        ui.label(egui::RichText::new(&st.hover).size(11.0).color(TEXT));
                    });
            });
    }

    act.wants_pointer = ctx.wants_pointer_input();
    act.center_on = panel_act.center_on;
    act.clear_selection = panel_act.clear_selection;
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
    let swatch =
        egui::Rect::from_min_size(rect.min + egui::vec2(4.0, 4.0), egui::vec2(16.0, 16.0));
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
        Stroke::new(
            if selected { 2.0 } else { 1.0 },
            if selected { ACCENT } else { BORDER },
        ),
        StrokeKind::Inside,
    );

    let desc = if id == ERASER {
        "只擦地形（墙/岩浆/沼泽/辐射带/腐殖质），食物与生物一律不动".to_string()
    } else {
        cell_type::def(id)
            .map(|d| d.desc.to_string())
            .unwrap_or_default()
    };
    if resp.on_hover_text(desc).clicked() {
        st.brush = id;
        st.tool = Tool::Paint;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_world() -> World {
        World::new(
            evosim_core::config::Config::default(),
            evosim_core::rng::Rng::new(1),
        )
    }

    #[test]
    fn ui_builds_without_panic() {
        let ctx = egui::Context::default();
        configure(&ctx);

        let mut world = test_world();
        let mut st = UiState::default();
        st.hover = "(12, 34) 嘴 · 个体 #3 · S1".to_string();
        // 造一条高亮，顺便验证世界坐标 → 屏幕像素那段
        st.highlights.push((
            egui::Rect::from_min_size(egui::pos2(10.0, 10.0), egui::vec2(1.0, 1.0)),
            ACCENT,
        ));

        let hud = HudInfo {
            fps: 60.0,
            tps: 60.0,
            speed: 1.0,
            paused: false,
            mode: 0,
            workshop_open: false,
        };
        let view = ViewInfo {
            cam_x: 0.0,
            cam_y: 0.0,
            zoom: 4.0,
            width: 1600.0,
            height: 950.0,
        };
        let mut panels = Panels::default();

        let mut input = egui::RawInput::default();
        input.screen_rect = Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1600.0, 950.0),
        ));

        let out = ctx.run(input, |ctx| {
            let _ = draw(ctx, &mut st, &mut world, &hud, &view, &mut panels);
        });

        assert!(!out.shapes.is_empty(), "界面没有产出任何图形");
    }

    #[test]
    fn brush_list_covers_registry_and_eraser() {
        let mut ids: Vec<u8> = evosim_core::cell_type::TERRAIN_BRUSH_TYPES.to_vec();
        ids.push(crate::paint::ERASER);
        assert!(ids.contains(&evosim_core::cell_type::ct::WALL));
        assert!(ids.contains(&evosim_core::cell_type::ct::LAVA));
        assert!(ids.contains(&crate::paint::ERASER));
        assert_eq!(ids.len(), 8, "7 个地形笔刷 + 橡皮擦");
    }

    /// 回归：世界高亮必须画在**窗口下面**。
    ///
    /// 曾经的 bug：高亮画在 `Order::Middle`。`GraphicLayers::drain()` 在同一个
    /// Order 内先画登记成 Area 的窗口、再画"不是 Area"的图层，而高亮不是 Area，
    /// 所以它永远排在所有窗口之后 —— 一打开物种工坊 / 检视器，高亮就压在上面。
    ///
    /// 这里不依赖窗口渲染（headless 下 egui 的窗口帧有淡入，第一帧会被剔成 Noop），
    /// 而是直接验证"层的先后"这件事本身：往 Middle 层画一个窗口底色，再往高亮层
    /// 画描边，然后检查绘制顺序里描边排在底色之前。
    #[test]
    fn highlight_is_painted_below_windows() {
        assert_eq!(
            highlight_layer().order,
            egui::Order::Background,
            "高亮层必须是 Order::Background（egui 定义：Painted behind all floating windows）"
        );

        let ctx = egui::Context::default();
        configure(&ctx);

        let mut input = egui::RawInput::default();
        input.screen_rect = Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1600.0, 950.0),
        ));

        let out = ctx.run(input, |ctx| {
            // 先画"窗口"（Middle 层），再画高亮 —— 故意按最坏的顺序添加
            let win_layer = egui::LayerId::new(egui::Order::Middle, egui::Id::new("fake-window"));
            ctx.layer_painter(win_layer).rect_filled(
                egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(200.0, 200.0)),
                CornerRadius::ZERO,
                PANEL,
            );
            ctx.layer_painter(highlight_layer()).rect_stroke(
                egui::Rect::from_min_size(egui::pos2(120.0, 120.0), egui::vec2(8.0, 8.0)),
                CornerRadius::ZERO,
                Stroke::new(1.0, ACCENT),
                StrokeKind::Inside,
            );
        });

        fn walk<'a>(shapes: &'a [egui::Shape], out: &mut Vec<&'a egui::epaint::RectShape>) {
            for s in shapes {
                match s {
                    egui::Shape::Rect(r) => out.push(r),
                    egui::Shape::Vec(v) => walk(v, out),
                    _ => {}
                }
            }
        }
        let flat: Vec<egui::Shape> = out.shapes.iter().map(|c| c.shape.clone()).collect();
        let mut rects = Vec::new();
        walk(&flat, &mut rects);

        let hl = rects
            .iter()
            .position(|r| r.fill == Color32::TRANSPARENT && r.stroke.color == ACCENT)
            .expect("没有画出高亮描边");
        let win = rects
            .iter()
            .position(|r| r.fill == PANEL)
            .expect("没有画出窗口底色");

        assert!(
            hl < win,
            "高亮必须画在窗口之前（高亮 #{}, 窗口 #{}）——否则会盖在工坊/检视器上面",
            hl,
            win
        );
    }
}
