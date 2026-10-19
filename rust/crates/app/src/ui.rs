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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Paint,
    Pan,
    Inspect,
    /// 放置点光源。光源**独立于物质世界**：不占格子、不挡路、画笔擦不到。
    Light,
}

/// 左键按下时要做什么
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeftAction {
    Place,
    Paint,
    Inspect,
    Pan,
    /// 放置/移除点光源（点一下放、再点一下移除）
    Light,
}

/// **放置模式优先于一切工具**。
///
/// 曾经把投放藏在 `paint_at_cursor` 里，而那条路径要求当前工具是「画笔」，
/// 于是"手动放置后点画布放不上去"——放置是一次性的显式意图，
/// 不该被"你手上拿的是什么工具"卡住。
pub fn left_action(placing: bool, tool: Tool) -> LeftAction {
    if placing {
        return LeftAction::Place;
    }
    match tool {
        Tool::Paint => LeftAction::Paint,
        Tool::Inspect => LeftAction::Inspect,
        Tool::Pan => LeftAction::Pan,
        Tool::Light => LeftAction::Light,
    }
}

pub struct UiState {
    pub brush: u8,
    pub brush_size: i32,
    /// 「光源」工具放下时光源的强度（1–255，决定照射半径 = 强度/absorb 格）
    pub light_strength: u8,
    pub brush_shape: BrushShape,
    pub tool: Tool,
    pub hover: String,
    pub show_tools: bool,
    pub show_params: bool,
    /// 类型参数窗口（细胞 / 地形，选中哪个调哪个）
    pub type_params_open: bool,
    /// 类型参数里当前选中的类型 id
    pub type_selected: u8,
    /// 世界坐标下的高亮矩形（1 格大小）+ 各自的颜色。
    /// 由面板填充，这里换算成屏幕像素再描边。
    pub highlights: Vec<(egui::Rect, Color32)>,
    /// 放置模式的提示文字（None = 不在放置模式）
    pub place_hint: Option<String>,
    /// 放置预览：光标格 + 蓝图的相对格偏移（画成半透明轮廓，点之前能看出落点）
    pub place_preview: Option<(i32, i32, Vec<(i32, i32)>)>,
}

impl Default for UiState {
    fn default() -> Self {
        UiState {
            brush: ct::WALL,
            brush_size: 1,
            light_strength: 255,
            brush_shape: BrushShape::Square,
            tool: Tool::Paint,
            hover: String::new(),
            show_tools: true,
            show_params: true,
            type_params_open: false,
            type_selected: ct::MOUTH,
            highlights: Vec::new(),
            place_hint: None,
            place_preview: None,
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
    pub world_open: bool,
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
    pub toggle_world: bool,
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

/// 内嵌的中文子集 —— 由 `tools/subset-font.py` 从系统字体里裁出来，
/// 只含界面真正用到的那些字（当前 984 个，197 KB）。
///
/// 为什么要嵌：exe 要能"拷到任何一台机器上双击就用"，不能依赖对方装了微软雅黑。
/// 为什么不嵌整字体：微软雅黑 18.8 MB，黑体 9.7 MB —— 太肥了。
const EMBEDDED_CJK: &[u8] = include_bytes!("../assets/ui-cjk-subset.ttf");

/// 加载中文字体。
///
/// 两层：
///   1. **内嵌子集**（保证 exe 自带、任何机器都不是方块）
///   2. **系统字体**挂在它后面当后备 —— 子集里没有的字（比如玩家自己起的名字）交给它
pub fn configure(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    fonts.font_data.insert(
        "cjk".to_owned(),
        Arc::new(egui::FontData::from_static(EMBEDDED_CJK)),
    );
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, "cjk".to_owned());
    }

    let mut sys_note = "（无系统后备）".to_string();
    for path in [
        "C:/Windows/Fonts/msyh.ttc",
        "C:/Windows/Fonts/simhei.ttf",
        "C:/Windows/Fonts/Deng.ttf",
    ] {
        if let Ok(bytes) = std::fs::read(path) {
            let kb = bytes.len() / 1024;
            let mut data = egui::FontData::from_owned(bytes);
            data.index = 0;
            fonts.font_data.insert("cjk-sys".to_owned(), Arc::new(data));
            for family in [FontFamily::Proportional, FontFamily::Monospace] {
                fonts
                    .families
                    .entry(family)
                    .or_default()
                    .insert(1, "cjk-sys".to_owned());
            }
            sys_note = format!("+ 系统后备 {}（{} KB）", path, kb);
            break;
        }
    }
    println!(
        "界面字体：内嵌子集 {} KB {}",
        EMBEDDED_CJK.len() / 1024,
        sys_note
    );

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
    v.window_stroke = Stroke::new(1.0_f32, BORDER);
    v.selection.bg_fill = Color32::from_rgb(30, 64, 92);
    v.override_text_color = Some(TEXT);

    for w in [
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = CornerRadius::ZERO;
        w.bg_stroke = Stroke::new(1.0_f32, BORDER);
    }
    v.widgets.inactive.weak_bg_fill = Color32::from_rgb(26, 33, 45);
    // 滑块轨道在 egui 0.31 里取 `widgets.inactive.bg_fill`（全库只有滑块用它，
    // 按钮走的是 weak_bg_fill），所以这一行只影响轨道。原来它和面板底色
    // 只差 6 级灰度，深底上等于隐形——滑块看起来像坏了。
    v.widgets.inactive.bg_fill = Color32::from_rgb(40, 50, 66);
    // 已填充段画成蓝色：细胞参数里一半属性的出厂值都在量程 20% 以内，
    // 光看拖柄位置分不清"0"和"0.002"，有了填充段一眼就能读出来。
    v.slider_trailing_fill = true;
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(36, 46, 62);
    v.widgets.hovered.bg_fill = Color32::from_rgb(52, 64, 82);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    v.widgets.active.weak_bg_fill = Color32::from_rgb(30, 64, 92);
    v.widgets.active.bg_fill = Color32::from_rgb(30, 64, 92);
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, BORDER);

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
                .stroke(Stroke::new(1.0_f32, BORDER))
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
                // 存活物种（与 JS 版 aliveCount 对齐）。累计数在悬停里，
                // 两个数并排站着最容易让人以为"物种数≈个体数"是判定崩了。
                ui.label(format!("物种 {}", world.species.alive_count))
                    .on_hover_text(format!(
                        "存活物种 {} · 累计出现 {} · 峰值存活 {} · 已灭绝 {}",
                        world.species.alive_count,
                        world.species.size(),
                        world.species.peak_alive_count,
                        world.species.total_extinct
                    ));
                ui.separator();
                ui.label(if hud.paused { "已暂停" } else { "运行中" });
                ui.separator();
                ui.label(format!("{:.0} fps · {:.0} tps", hud.fps, hud.tps));

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .selectable_label(hud.world_open, "世界")
                        .on_hover_text("新建（可调尺寸）/ 保存 / 读取（快捷键 W）")
                        .clicked()
                    {
                        act.toggle_world = true;
                    }
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
                    if ui
                        .selectable_label(st.show_params, "全局参数")
                        .on_hover_text(
                            "整个模拟一份的设置：生命规则 / 变异 / 感知 / 体型\n\
                             改这里对所有个体所有格子一起生效；每种类型自己的参数在「类型参数」",
                        )
                        .clicked()
                    {
                        st.show_params = !st.show_params;
                    }
                    if ui
                        .selectable_label(st.type_params_open, "类型参数")
                        .on_hover_text(
                            "每种类型自己的参数：细胞 / 地形，选中哪个调哪个\n\
                             这里管「每种一份」，全局乘数在「全局参数」里",
                        )
                        .clicked()
                    {
                        st.type_params_open = !st.type_params_open;
                    }
                    if ui.button("适应窗口").on_hover_text("快捷键 F").clicked() {
                        act.fit = true;
                    }
                    let mode_label = match hud.mode {
                        1 => "着色：物种",
                        2 => "着色：温度",
                        3 => "着色：辐射",
                        4 => "着色：光照",
                        _ => "着色：类型",
                    };
                    if ui
                        .button(mode_label)
                        .on_hover_text("Tab 依次切换：类型 → 物种 → 温度 → 辐射 → 光照")
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
                    // 注意：这个 with_layout 是 right_to_left，**先加的在最右边**。
                    // 想让人看到「左降速 · 1× · 右提速」，代码里就得先写提速。
                    if ui.button(">").on_hover_text("提速").clicked() {
                        act.speed_delta = 1;
                    }
                    ui.label(format!("{:.0}×", hud.speed));
                    if ui.button("<").on_hover_text("降速").clicked() {
                        act.speed_delta = -1;
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
                    if ui
                        .selectable_label(st.tool == Tool::Light, "光源")
                        .on_hover_text(
                            "在画布上点一下放一个点光源，再点一下移除。\n\
                             光源独立于物质世界：不占格子、不挡路、画笔擦不到。",
                        )
                        .clicked()
                    {
                        st.tool = Tool::Light;
                    }
                });
                // 光源工具的两条提示：强度决定照射半径，且它不在物质层里
                if st.tool == Tool::Light {
                    ui.add_space(2.0);
                    ui.add(
                        egui::Slider::new(&mut st.light_strength, 8..=255)
                            .text("强度")
                            .step_by(1.0),
                    )
                    .on_hover_text(
                        "照射半径 ≈ 强度 / 衰减步长。满强度能照到世界大半，\
                         调低就是一个小范围的光斑。",
                    );
                    ui.label(
                        egui::RichText::new("左键放置 / 移除 · 不占格子，生物可以穿过")
                            .color(TEXT_DIM)
                            .size(10.0),
                    );
                }
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

    // ── 右侧：全局参数 ──
    //
    // 和「类型参数」的分工：这里的东西整个模拟只有一份（世界尺寸、生命规则、
    // 变异、感知），改了对所有个体所有格子一起生效；那边的值每种类型一份。
    //
    // 两边不是"粗调 / 细调"，而是：**实际值 = 这里的全局基值 × 那边的类型倍率**。
    // 所以地形效果**不在这里**——那是每种地形一份，归「类型参数」。
    if st.show_params {
        let mut params_open = st.show_params;
        // 能量账要在借 `&mut world.config` **之前**算好：
        // 窗口闭包里拿着配置的可变借用，那之后再读 world 就是借用冲突。
        let ledger = world.energy_audit();
        egui::Window::new("全局参数")
            .resizable(false)
            .collapsible(true)
            .open(&mut params_open)
            .default_pos([1100.0, 42.0])
            .show(ctx, |ui| {
                let cfg = &mut world.config;
                // 活着的价格。批次 10 起默认 1.0（不再是 0）：
                // 每个细胞的 `消耗` 已经是真实量级（生产者 0.010 对满照收入 0.12），
                // 所以 1.0 就是"照价付账"。0 = 关掉代谢，细胞不吃饭也不饿死。
                //
                // 量程 0~5：出厂 1.0 落在 20% 处。实测 2.0 世界开始稀疏、
                // 4.0 濒临灭绝 —— 上限留着，是给"想看生态系统怎么死"的人用的。
                let before_metab = cfg.life.metabolism_scale;
                let mut metab = before_metab;
                ui.add(
                    egui::Slider::new(&mut metab, 0.0..=5.0)
                        .text("全局消耗量倍率")
                        .step_by(0.05)
                        .fixed_decimals(2),
                )
                .on_hover_text(
                    "乘在所有细胞的每 tick 消耗上（默认 1.00）。\n\
                     0 = 关掉代谢：细胞不吃饭也不饿死，光合变成稳赚不赔。\n\
                     实测 2.0 世界开始稀疏、4.0 濒临灭绝。",
                );
                if (metab - before_metab).abs() > 1e-9 {
                    cfg.life.metabolism_scale = metab;
                }
                ui.add(
                    egui::Slider::new(&mut cfg.life.food_energy_scale, 0.1..=10.0)
                        .text("食物量倍率")
                        .step_by(0.1),
                )
                .on_hover_text("一份食物能提供多少能量（默认 1）。");
                // ── 食物腐败 ──
                //
                // 腐败是「相对 + 绝对」两条腿：相对项让大块尸体慢慢烂，绝对项
                // 让小块食物干脆消失。这里只把**绝对项**放出来，因为它是
                // 「尸体能躺多久」的直接旋钮——调大 = 残骸很快清空，
                // 调小 = 肥尸体变成一块能吃很久的资源（守尸 / 抢食才有戏）。
                //
                // 写法照 cells.rs 的老规矩：先读进局部变量、画完再比对，
                // 只在**真的变了**才写回。egui 的 Slider 绘制时会按 `step_by`
                // 吸附值，无条件写回等于让「打开参数面板」变成一个改配置的动作。
                let before_floor = cfg.world.food_decay_floor;
                let mut floor = before_floor;
                ui.add(
                    egui::Slider::new(&mut floor, 0.0..=0.1)
                        .text("食物腐败下限")
                        .step_by(0.005)
                        .fixed_decimals(3),
                )
                .on_hover_text(
                    "每 tick 至少流失这么多能量（默认 0.005）。\n\
                     调小 = 小块食物也能拖很久，肥尸体变成长期资源；\n\
                     调大 = 残骸很快清空。0 = 只按百分比衰减。",
                );
                if (floor - before_floor).abs() > 1e-9 {
                    cfg.world.food_decay_floor = floor;
                }
                // ── 光与光合 ──
                //
                // 批次 7 新增。生产者不再是永动机：它把**所在格的光**转成能量。
                // 批次 7.5 起天空光只是「世界顶部的一排光源」，可以整个关掉——
                // 关掉之后世界里只剩玩家用「光源」工具放的点光源，
                // 于是「光从哪来」完全由玩家决定，世界不再被强行分成上亮下黑。
                let before_sun = cfg.world.sun_intensity;
                let mut sun = before_sun;
                ui.add(
                    egui::Slider::new(&mut sun, 0.0..=1.0)
                        .text("天空光")
                        .step_by(0.05)
                        .fixed_decimals(2),
                )
                .on_hover_text(
                    "世界顶部那一排光源的强度。\n\
                     0 = 永夜：只剩你自己放的点光源（用左侧「光源」工具放）。",
                );
                if (sun - before_sun).abs() > 1e-9 {
                    cfg.world.sun_intensity = sun;
                }
                let before_photo = cfg.life.photosynthesis_rate;
                let mut photo = before_photo;
                ui.add(
                    egui::Slider::new(&mut photo, 0.0..=1.0)
                        .text("光合速率")
                        .step_by(0.01)
                        .fixed_decimals(2),
                )
                .on_hover_text(
                    "满照下一个生产者细胞每 tick 产出多少能量。\n\
                     产出正比于它所在格的光强——暗处的生产者是纯负担。",
                );
                if (photo - before_photo).abs() > 1e-9 {
                    cfg.life.photosynthesis_rate = photo;
                }

                // 遮挡：给「光合生物挤成一道墙」定价。
                //
                // 生产者正上方 8 行内每有一个身体格（**含自己的身体**），天空光就乘 (1-k)。
                // 墙内部的细胞照不到光、却照样付 upkeep → 墙从内部塌掉。
                // 只作用于天空光：身体挡得住头顶下来的光，挡不住侧面照进来的灯。
                let before_shade = cfg.life.shade_strength;
                let mut shade = before_shade;
                ui.add(
                    egui::Slider::new(&mut shade, 0.0..=1.0)
                        .text("遮挡强度")
                        .step_by(0.05)
                        .fixed_decimals(2),
                )
                .on_hover_text(
                    "生产者上方每压着一层身体，天空光就衰减这么多（默认 0.25）。
                     自己的身体也算——光不知道格子归谁。
                     0 = 关闭。实测 0.25 把有光区的「致密格」从 9.3% 压到 1.9%。",
                );
                if (shade - before_shade).abs() > 1e-9 {
                    cfg.life.shade_strength = shade;
                }

                // ── 捕食（批次 9.5：嘴咬活体）──
                //
                // 这一对旋钮决定「吃活的」值不值得。批次 9 的诊断结论是：
                // 杀手能杀人，但尸体撒在死者身上、凶手够不着，于是捕食的实际
                // 回报低于当生产者，杀手和移动器一起灭绝。把回报接到嘴上之后，
                // 捕食才第一次有正收益。
                //
                // 两个数要一起看：`咬合速率` 是"一口能吃多快"，`消化效率` 是
                // "吃到嘴的留下多少"。后者 < 1 是生态学里的营养级损耗，
                // 也是批次 10 守恒审计里唯一允许"离开生物圈"的常设项。
                let before_bite = cfg.life.bite_rate;
                let mut bite = before_bite;
                ui.add(
                    egui::Slider::new(&mut bite, 0.0..=2.0)
                        .text("咬合速率")
                        .step_by(0.05)
                        .fixed_decimals(2),
                )
                .on_hover_text(
                    "每个嘴细胞每 tick 能从相邻活体身上抽走多少能量（默认 0.30）。\n\
                     抽走量被对方当时剩的能量卡住——咬空壳什么也得不到。\n\
                     0 = 关闭「咬活体」，嘴只能吃食物和尸体。",
                );
                if (bite - before_bite).abs() > 1e-9 {
                    cfg.life.bite_rate = bite;
                }
                let before_eff = cfg.life.bite_efficiency;
                let mut eff = before_eff;
                ui.add(
                    egui::Slider::new(&mut eff, 0.0..=1.0)
                        .text("消化效率")
                        .step_by(0.05)
                        .fixed_decimals(2),
                )
                .on_hover_text(
                    "抽走的能量有多少变成自己的，其余散掉（默认 0.80）。\n\
                     小于 1 是捕食的固有代价：营养级每升一级都会损耗。",
                );
                if (eff - before_eff).abs() > 1e-9 {
                    cfg.life.bite_efficiency = eff;
                }
                // ── 食物算不算路障 ──
                //
                // 拆成两个开关是因为它们是两件事：穿过一片尸体堆去追猎物（移动），
                // 和把新生儿生在一格食物上（繁殖）。一个开关管两件事的话，
                // 想放开前者就必然放开后者。
                //
                // 打开「食物不挡移动」是有代价的：身体盖上去会把那格食物**压碎**
                // （`types` 是单层，能量没地方存），压碎的能量进热池。
                // 所以这个开关适合"让捕食者能穿过尸体堆"，不适合当日常配置。
                ui.checkbox(
                    &mut cfg.combat.food_blocks_movement,
                    "食物挡住移动",
                )
                .on_hover_text(
                    "关掉之后生物能穿过食物格。代价：盖上去会把那格食物压碎，\n\
                     碎掉的能量记进热池（不是吃掉，是毁掉）。默认开启。",
                );
                ui.checkbox(
                    &mut cfg.combat.food_blocks_reproduction,
                    "食物挡住繁殖",
                )
                .on_hover_text("关掉之后新生儿可以直接生在食物格上（会压掉那格食物）。默认开启。");
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
                ui.separator();

                // ── 体型 ──
                //
                // 这两项**会改变模拟**，而且默认值（1.0 / 0）正被黄金快照锁着，
                // 所以面板上改它们等于"有意让这次运行偏离 1.x 基线"，是给人做对照实验用的。
                //
                // 写法照 cells.rs 的老规矩：先读进局部变量、画完再比对，只在**真的变了**
                // 才写回。egui 的 Slider 会在绘制时按 `step_by` 吸附值，无条件写回等于
                // 让"打开参数面板"变成一个会改配置的动作——1.0 会被吸成 1.0000000000000002。
                let before_p = cfg.life.breed_cost_exponent;
                let mut p = before_p;
                ui.add(
                    egui::Slider::new(&mut p, 0.5..=2.0)
                        .text("繁殖成本指数 p")
                        .step_by(0.05)
                        .fixed_decimals(2),
                )
                .on_hover_text(
                    "繁殖成本 = 体型^p。1.00 = 线性（默认，与 1.x 一致）\n\
                     < 1 让大体型更便宜 → 体型往上漂；> 1 反过来把体型压小。",
                );
                if (p - before_p).abs() > 1e-9 {
                    cfg.life.breed_cost_exponent = p;
                }

                let before_cap = cfg.life.max_body_cells;
                let mut cap = before_cap;
                ui.add(
                    egui::Slider::new(&mut cap, 0..=40)
                        .text("体型上限")
                        .step_by(1.0),
                )
                .on_hover_text(
                    "细胞数上限，0 = 不限（默认）。\n\
                     只挡变异里的「加细胞」算子，不影响工坊里手画的个体。",
                );
                if cap != before_cap {
                    cfg.life.max_body_cells = cap;
                }

                // ── 能量账（批次 10）──
                //
                // 守恒律只对生命层成立：世界只有三个能量入口（光合 / 腐殖质 /
                // 手工放置），其余全是内部搬运。所以
                // `注入 == 生物 + 食物 + 热池 + 丢弃` 应该恒成立，
                // 残差不为 0 就说明有某条路径在凭空造能量或吞能量。
                ui.separator();
                ui.label(egui::RichText::new("能量账").strong());
                egui::Grid::new("energy-ledger")
                    .num_columns(2)
                    .spacing([8.0, 2.0])
                    .show(ui, |ui| {
                        let row = |ui: &mut egui::Ui, k: &str, v: f64| {
                            ui.label(egui::RichText::new(k).color(Color32::from_rgb(150, 165, 185)));
                            ui.label(format!("{:.1}", v));
                            ui.end_row();
                        };
                        row(ui, "注入", ledger.injected);
                        row(ui, "生物体内", ledger.organisms);
                        row(ui, "地上食物", ledger.food);
                        row(ui, "热池（离开生物圈）", ledger.heat);
                        row(ui, "丢弃（超容量 + 消化）", ledger.wasted);
                        let drift = ledger.drift();
                        let color = if drift.abs() <= 1e-6 * ledger.stored().max(1.0) {
                            Color32::from_rgb(120, 200, 140)
                        } else {
                            Color32::from_rgb(230, 120, 120)
                        };
                        ui.label(egui::RichText::new("残差").color(color));
                        ui.label(egui::RichText::new(format!("{:.3e}", drift)).color(color));
                        ui.end_row();
                    });
                ui.label(
                    egui::RichText::new("注入 = 生物 + 食物 + 热池 + 丢弃，残差应恒为 0")
                        .color(Color32::from_rgb(130, 140, 155))
                        .size(11.0),
                );
            });
        st.show_params = params_open;
    }

    if st.type_params_open {
        let mut open = st.type_params_open;
        egui::Window::new("类型参数")
            .resizable(false)
            .collapsible(false)
            .min_width(470.0)
            .max_width(470.0)
            .open(&mut open)
            .default_pos([500.0, 300.0])
            .show(ctx, |ui| {
                crate::cells::draw(ui, world, &mut st.type_selected);
            });
        st.type_params_open = open;
    }

    // ── 面板：检视器 / 物种 / 曲线 ──
    st.highlights.clear();
    let panel_act = panels.draw(ctx, world, &mut st.highlights);

    // 放置预览：把蓝图足迹按光标位置推进高亮列表（和别的高亮同一层，画在窗口下面）
    let preview = st.place_preview.take();
    if let Some((col, row, cells)) = preview {
        let tint = Color32::from_rgba_unmultiplied(ACCENT.r(), ACCENT.g(), ACCENT.b(), 170);
        for (dx, dy) in cells {
            st.highlights.push((
                egui::Rect::from_min_size(
                    egui::pos2((col + dx) as f32, (row + dy) as f32),
                    egui::vec2(1.0, 1.0),
                ),
                tint,
            ));
        }
    }

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
                    Stroke::new(1.0_f32, *color),
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

    // ── 点光源标记 ──
    //
    // 光源不占物质格子，所以**画面上没有任何格子能表示它**——不画的话用户放了灯
    // 却看不见，只能靠"这块地方莫名变亮"猜。这里画一个小圆点 + 光晕。
    //
    // 和网格对齐到格心（+0.5），这样缩小时它看起来就是"某一格上的一个亮点"，
    // 而不是漂在格子缝里。
    if !world.light.sources.is_empty() {
        let canvas = ctx.available_rect();
        let painter = ctx
            .layer_painter(hl_layer)
            .with_clip_rect(canvas.intersect(ctx.screen_rect()));
        let z = view.zoom;
        for s in &world.light.sources {
            let cx = (s.col as f32 + 0.5 - view.cam_x) * z;
            let cy = (s.row as f32 + 0.5 - view.cam_y) * z;
            if cx < -24.0 || cy < -24.0 || cx > view.width + 24.0 || cy > view.height + 24.0 {
                continue;
            }
            // 亮度越高点越大，一眼看出这个灯有多强
            let base = (z * 0.5).clamp(2.0, 12.0);
            let r = base * (0.55 + 0.45 * s.strength as f32 / 255.0);
            let c = egui::pos2(cx, cy);
            painter.circle_filled(
                c,
                r * 2.2,
                Color32::from_rgba_unmultiplied(255, 236, 140, 34),
            );
            painter.circle_filled(c, r, Color32::from_rgb(255, 238, 150));
            painter.circle_stroke(c, r, Stroke::new(1.0_f32, Color32::from_rgb(120, 88, 20)));
        }
    }

    // ── 左下：场滤镜图例 ──
    if hud.mode >= 2 {
        let (lut, name) = match hud.mode {
            2 => (evosim_render::colors::temp_colormap(), "温度"),
            3 => (evosim_render::colors::rad_colormap(), "辐射"),
            _ => (evosim_render::colors::light_colormap(), "光照"),
        };
        egui::Area::new("field-legend".into())
            .anchor(egui::Align2::LEFT_BOTTOM, egui::vec2(10.0, -10.0))
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(Color32::from_rgba_unmultiplied(13, 17, 23, 230))
                    .stroke(Stroke::new(1.0_f32, BORDER))
                    .inner_margin(egui::Margin::symmetric(8, 4))
                    .show(ui, |ui| {
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(name).size(11.0).color(TEXT),
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

    // ── 顶部中央：放置模式提示条 ──
    //
    // 放置模式以前没有任何可见提示，用户点了「手动放置」之后不知道自己在什么状态
    // （工坊关了、鼠标点了却没反应）。这条横幅把状态和退出方式都说清楚。
    if let Some(hint) = st.place_hint.clone() {
        egui::Area::new("place-hint".into())
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 44.0))
            .show(ctx, |ui| {
                egui::Frame::NONE
                    .fill(Color32::from_rgba_unmultiplied(13, 17, 23, 240))
                    .stroke(Stroke::new(1.0_f32, ACCENT))
                    .inner_margin(egui::Margin::symmetric(10, 5))
                    .show(ui, |ui| {
                        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                        ui.label(egui::RichText::new(hint).size(12.0).color(ACCENT));
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
                    .stroke(Stroke::new(1.0_f32, BORDER))
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
        Stroke::new(1.0_f32, Color32::from_rgb(0, 0, 0)),
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
            if selected { 2.0_f32 } else { 1.0_f32 },
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
            world_open: false,
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

    /// 类型参数窗口要能把**每一种**类型都画出来。
    ///
    /// `ui_builds_without_panic` 用的是 `UiState::default()`，那个窗口是关着的，
    /// 所以它一个字都没画到。这里逐个选中跑一帧。
    ///
    /// 地形那半边是这次新接进来的：参数种类和值的来源都和细胞不同
    /// （`Config.terrain` vs `Config.cells`），`Values::get` 挂错分支会**静默返回 0**
    /// ——滑块照样画得出来，只是永远是 0 且拖不动。只有真的每种都画一遍才发现得了。
    #[test]
    fn type_params_window_draws_every_type() {
        let ctx = egui::Context::default();
        configure(&ctx);
        let mut world = test_world();

        let mut ids: Vec<u8> = evosim_core::cell_type::LIVING_CELL_TYPES.to_vec();
        ids.extend_from_slice(&evosim_core::cell_type::PARAM_TERRAIN_TYPES);

        for id in ids {
            let mut sel = id;
            let mut input = egui::RawInput::default();
            input.screen_rect = Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1600.0, 950.0),
            ));
            let out = ctx.run(input, |ctx| {
                egui::Window::new("类型参数").show(ctx, |ui| {
                    crate::cells::draw(ui, &mut world, &mut sel);
                });
            });
            assert_eq!(
                sel,
                id,
                "选中「{}」之后被面板改掉了（多半是兜底分支误判成「没有参数」）",
                evosim_core::cell_type::label(id)
            );
            assert!(!out.shapes.is_empty(), "「{}」没画出任何图形", evosim_core::cell_type::label(id));
        }
    }

    /// 回归：放置模式必须压过当前工具。
    ///
    /// 曾经的 bug：投放逻辑藏在 `paint_at_cursor` 里，那条路径要求工具是「画笔」，
    /// 所以用户拿着「检视」或「平移」时，装了蓝图也点不出东西。
    #[test]
    fn placement_wins_over_every_tool() {
        for tool in [Tool::Paint, Tool::Pan, Tool::Inspect, Tool::Light] {
            assert_eq!(
                left_action(true, tool),
                LeftAction::Place,
                "放置模式下左键必须是投放（工具 = {:?}）",
                tool
            );
            // 不在放置模式时照旧按工具走
            assert_ne!(left_action(false, tool), LeftAction::Place);
        }
        assert_eq!(left_action(false, Tool::Paint), LeftAction::Paint);
        assert_eq!(left_action(false, Tool::Inspect), LeftAction::Inspect);
        assert_eq!(left_action(false, Tool::Pan), LeftAction::Pan);
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
                Stroke::new(1.0_f32, ACCENT),
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

    /// 内嵌字体必须覆盖界面上的每一个汉字。
    ///
    /// `tools/subset-font.py` 扫源码生成 `assets/ui-chars.txt`（它裁字体时的输入）。
    /// 这条测试守着"改了 UI 文案却忘了重跑脚本"——漏掉的字在界面上就是方块，
    /// 而方块在无头测试里看不出来，只有这里能抓住。
    #[test]
    fn embedded_font_covers_every_ui_char() {
        let face = ttf_parser::Face::parse(EMBEDDED_CJK, 0).expect("内嵌字体解析失败");
        let chars = include_str!("../assets/ui-chars.txt");

        let mut missing: Vec<char> = Vec::new();
        for ch in chars.chars() {
            if face.glyph_index(ch).is_none() {
                missing.push(ch);
            }
        }
        assert!(
            missing.is_empty(),
            "内嵌字体缺 {} 个字：{:?}（请重跑 python tools/subset-font.py）",
            missing.len(),
            missing
        );
        assert!(chars.chars().count() > 500, "字符清单太小，可能没生成好");
    }

    /// 子集必须"小"——它是嵌进 exe 的，膨胀了要有人管
    #[test]
    fn embedded_font_is_small() {
        let kb = EMBEDDED_CJK.len() / 1024;
        assert!(kb < 400, "内嵌字体 {} KB，太大了（关掉 hinting 能砍一半）", kb);
        assert!(kb > 50, "内嵌字体只有 {} KB，是不是裁错了", kb);
    }
}
