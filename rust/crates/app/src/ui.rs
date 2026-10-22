//! egui 界面：顶部状态条、左侧工具/笔刷、右侧参数、右下悬停读数，外加三个面板。
//!
//! 风格刻意压紧（12px 字、行距 4、无圆角、1px 描边）——目标是 TPT 那种密度，
//! 而不是 egui 默认的"通用桌面软件"观感。笔刷方块是**自绘控件**（painter 直接画）。

use std::sync::Arc;

use egui::{Align2, Color32, CornerRadius, FontFamily, FontId, Sense, Stroke, StrokeKind};

use evosim_core::cell_type::{self, ct, TERRAIN_BRUSH_TYPES};
use evosim_core::world::World;
use evosim_render::colors::build_palette;

use crate::paint::{BrushShape, CREATURE_ERASER, ERASER};
use crate::panels::Panels;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    Paint,
    Pan,
    Inspect,
    /// 放置点光源。光源**独立于物质世界**：不占格子、不挡路、画笔擦不到。
    Light,
    /// 移除点光源：删掉光标附近最近的那一个。
    ///
    /// 单独立一个工具而不是靠「再点一下切换」：光源在画面上是一团光晕，
    /// 视觉上的"这个光源"和它的格子中心差几格是常事；拖拽放置还会一次留下
    /// 一串。按"最近的"删，点哪儿都能删掉眼前那个。
    LightErase,
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
    /// 删掉光标附近最近的点光源
    LightErase,
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
        Tool::LightErase => LeftAction::LightErase,
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
            show_tools: false,
            show_params: false,
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
                        .on_hover_text(
                            "Tab 依次切换：类型 → 物种 → 温度 → 辐射 → 光照\n\
                             温度视图显示的是**绝对温度**（0–120℃ 映射到色表）：\n\
                             常温是暗紫，岩浆顶到最亮。调「环境温度」会看到整幅底色变亮。",
                        )
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
                } else if st.tool == Tool::LightErase {
                    ui.add_space(2.0);
                    ui.label(
                        egui::RichText::new("左键删掉附近最近的光源 · 可以拖拽连续删")
                            .color(TEXT_DIM)
                            .size(10.0),
                    );
                }
                ui.add_space(2.0);
                ui.label(egui::RichText::new("笔刷").color(TEXT_DIM));

                let palette = build_palette();
                let mut ids: Vec<u8> = TERRAIN_BRUSH_TYPES.to_vec();
                // 两个橡皮擦并排：擦地形 / 擦生物。分开是刻意的——
                // "把这块地方清空"和"把这只生物删掉"是两种不同的意图。
                ids.push(ERASER);
                ids.push(CREATURE_ERASER);

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
                } else if st.brush == CREATURE_ERASER {
                    "正在擦：生物（地形与食物不动）".to_string()
                } else {
                    format!("正在画：{}", cell_type::label(st.brush))
                };
                ui.label(egui::RichText::new(hint).color(TEXT_DIM).size(11.0));

                // 选中笔刷的简介（文案来自注册表 desc，笔刷/工坊/参数共用一份）
                let desc = if st.brush == ERASER {
                    "只擦地形（墙/岩浆/沼泽/辐射带/腐殖质），食物与生物一律不动".to_string()
                } else if st.brush == CREATURE_ERASER {
                    "只擦生物：整只移除，不留残骸（能量记进热池）。地形与食物不动".to_string()
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
            .default_pos([880.0, 42.0])
            .default_width(680.0)
            .show(ctx, |ui| {
                let cfg = &mut world.config;
                // ── 双列 ──
                //
                // 参数太多了，单列一路排下去有 2000 多像素高，一打开就铺满屏幕。
                // 分两列之后高度砍半，左右各自滚动（`horizontal_top` 保证两列从
                // 同一条基线开始，不会因为一边短就错位）。
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        // 每列固定宽度：不指定的话 egui 会按内容自然宽度排，
                        // 滑块宽度会忽大忽小，两列也参差不齐。
                        ui.set_width(316.0);
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

                // ── 云影（批次 10.5）──
                //
                // 把天空光乘一张平滑噪声图：亮的成"光岛"，暗的成"暗廊"。
                // 目的不是让世界变暗，而是把**明暗交界**从一条水平线拉成几千格的锯齿——
                // 批次 10 的结论是"光太均匀"才是专性异养的瓶颈。
                //
                // 平均透射率被归一化过（p=1 时约 0.5），所以这个旋钮只代表"碎片化"，
                // 不代表"变暗"。想单独测变暗就把上面的「天空光」往下拨。
                let before_patch = cfg.world.sky_patchiness;
                let mut patch = before_patch;
                ui.add(
                    egui::Slider::new(&mut patch, 0.0..=1.0)
                        .text("云影")
                        .step_by(0.05)
                        .fixed_decimals(2),
                )
                .on_hover_text(
                    "把天空光切成光岛 + 暗廊（默认 0 = 关闭，逐位等于旧行为）。
                     1.0 时约一半的格子透射率低于 15%、一半高于 85%。
                     只挡太阳，不挡你自己放的点光源。",
                );
                if (patch - before_patch).abs() > 1e-9 {
                    cfg.world.sky_patchiness = patch;
                }

                // ── 富斑块 / 贫斑块（批次 13）──
                //
                // 和「云影」共用同一张噪声图，但管的是**另一件事**：
                // 云影决定"明暗交界有多长"，这一项决定"暗处能有多穷"。
                //
                // 云影的暗廊是全黑的，生物干脆不住；把地板抬起来之后，
                // 暗廊变成**能住但很穷**的地方——光合只够维持、不够长身体。
                // 这才是"富斑块 / 贫斑块"该有的样子。
                let before_rich = cfg.world.patch_richness;
                let mut rich = before_rich;
                ui.add(
                    egui::Slider::new(&mut rich, 0.0..=1.0)
                        .text("富贫斑块")
                        .step_by(0.05)
                        .fixed_decimals(2),
                )
                .on_hover_text(
                    "把世界切成「光岛」和「暗廊」，暗廊里只剩 (1-值) 的天空光。
                     0（默认）= 关闭，逐位等于旧行为；0.7 = 暗廊只剩 30% 的光，
                     能活但长不动 —— 这是推荐值；1.0 = 暗廊全黑（等于云影的极端）。
                     食物密度也跟着这张图走：光足的地方草甸更密。
                     和「云影」共用一张噪声图，但云影的暗廊是全黑的，两者别一起开。",
                );
                if (rich - before_rich).abs() > 1e-9 {
                    cfg.world.patch_richness = rich;
                }

                // ── 昼夜（批次 22）──
                //
                // 光场在此之前完全是空间性的。加时间轴之后**储能第一次有独立价值**：
                // 白天攒、夜里烧；而容量正比于细胞数，所以大体型 = 大电池。
                // 这是给大体型找实质收益最自然的一条路。
                let before_day = cfg.world.day_length;
                let mut day = before_day as f64;
                ui.add(
                    egui::Slider::new(&mut day, 0.0..=2000.0)
                        .text("昼长")
                        .step_by(50.0)
                        .fixed_decimals(0),
                )
                .on_hover_text(
                    "一个昼夜周期多少 tick。0（默认）= 关闭，永远是正午。\n\
                     600 ≈ 10 秒（60 tick/s 下）；1200 更接近真实的一天。\n\
                     日照走余弦：正午满照、午夜降到「夜间日照」。\n\
                     点光源不受影响——夜里灯是唯一的收入来源。",
                );
                if (day - before_day as f64).abs() > 0.5 {
                    cfg.world.day_length = day.round().max(0.0) as u32;
                }
                let before_night = cfg.world.night_floor;
                let mut night = before_night;
                ui.add(
                    egui::Slider::new(&mut night, 0.0..=0.5)
                        .text("夜间日照")
                        .step_by(0.01)
                        .fixed_decimals(2),
                )
                .on_hover_text(
                    "午夜还剩多少比例的天空光（默认 0.085 = 收支平衡点）。\n\
                     低于它，夜里净亏钱，刚繁殖完（能量 0）的个体会被饿死，\n\
                     出生全挤到白天，人口掉一半以上（实测 0.05 时 904 → 148）。\n\
                     注意：平均日照 = (1 + 本值) / 2，所以开昼夜会让世界整体变穷；\n\
                     想保住人口就同时把「天空光」或「光合速率」提上去。",
                );
                if (night - before_night).abs() > 1e-9 {
                    cfg.world.night_floor = night;
                }

                // ── 洞穴（批次 25）──
                //
                // 云影已经证明「有暗区」不够——暗区还得有饭吃才可能住人。
                // 这里把现成的两样东西摆在一起：墙挡光做顶、腐殖质产食做底。
                let before_cave = cfg.world.cave_count;
                let mut cave = before_cave as f64;
                ui.add(
                    egui::Slider::new(&mut cave, 0.0..=12.0)
                        .text("洞穴")
                        .step_by(1.0)
                        .fixed_decimals(0),
                )
                .on_hover_text(
                    "世界生成时挖几条带顶的隧道（0 = 默认，关闭）。\n\
                     顶是墙（挡光）、底是腐殖质（每 tick 产食），每条挖一个竖井通地表。\n\
                     竖井会变成「光柱」：洞口亮、越往里越暗，正好是个梯度。\n\
                     只在新建 / 重置世界时生效，中途改不会重挖。",
                );
                if (cave - before_cave as f64).abs() > 0.5 {
                    cfg.world.cave_count = cave.round().max(0.0) as i32;
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
                // ── 地力耗竭（批次 35）──
                //
                // 批次 33 的结论：移动没有收益 → 移动细胞 0.1% → 大脑是装饰品。
                // 地力把"站着不动"从免费变成有代价：同一格连着光合会越来越瘦。
                // **必须配合「运动可光合」**：不然能动的个体根本不能光合，换地方也没意义。
                let before_soil = cfg.life.soil_deplete;
                let mut soil = before_soil as f64;
                ui.add(
                    egui::Slider::new(&mut soil, 0.0..=16.0)
                        .text("地力耗竭")
                        .step_by(1.0)
                        .fixed_decimals(0),
                )
                .on_hover_text(
                    "光合细胞每 tick 从脚下那一格抽走多少地力（0 = 关闭）。
                     255 ÷ 本值 = 一块生地能被吃多少 tick（8 ≈ 32 tick 见底）。
                     这是「移动」唯一的收益来源——不走，产量就掉到「地力下限」。
                     注意：调到 8 以上容易把生态打死（实测 8 直接灭绝）。",
                );
                if (soil - before_soil as f64).abs() > 0.5 {
                    cfg.life.soil_deplete = soil.round().max(0.0) as u8;
                }
                if cfg.life.soil_deplete > 0 {
                    let before_sr = cfg.life.soil_recover;
                    let mut sr = before_sr as f64;
                    ui.add(
                        egui::Slider::new(&mut sr, 0.0..=8.0)
                            .text("地力恢复")
                            .step_by(1.0)
                            .fixed_decimals(0),
                    )
                    .on_hover_text("每 tick 全图每格回多少地力。恢复远慢于消耗，换地方才有价值。");
                    if (sr - before_sr as f64).abs() > 0.5 {
                        cfg.life.soil_recover = sr.round().max(0.0) as u8;
                    }
                    let before_sf = cfg.life.soil_floor;
                    let mut sf = before_sf;
                    ui.add(
                        egui::Slider::new(&mut sf, 0.0..=1.0)
                            .text("地力下限")
                            .step_by(0.05)
                            .fixed_decimals(2),
                    )
                    .on_hover_text(
                        "地力耗尽时产量还剩几成（默认 0.25）。
                         不能是 0——全枯会让「停在原地」等于必死，生态直接崩。",
                    );
                    if (sf - before_sf).abs() > 1e-9 {
                        cfg.life.soil_floor = sf;
                    }
                }
                let mut mcp = cfg.life.movers_can_produce;
                if ui
                    .checkbox(&mut mcp, "运动可光合")
                    .on_hover_text(
                        "默认关：有运动细胞就完全不能光合，世界被切成「光合不能动 / 能动不能光合」两半。
                         批次 33 量到移动细胞只占 0.1%——因为最划算的「移动 + 光合」被禁掉了。
                         打开后配合「地力耗竭」，移动第一次能靠换地方挣钱（实测 0.1% → 17%）。",
                    )
                    .changed()
                {
                    cfg.life.movers_can_produce = mcp;
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
                        .step_by(0.01)
                        .fixed_decimals(2),
                )
                .on_hover_text(
                    "每个嘴细胞每 tick 能从相邻活体身上抽走多少能量（默认 0.06，是满照光合 0.12 的一半）。\n\
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
                // ── 一张嘴一口（批次 21）──
                //
                // 批次 20 的扫描挖出的根因：`Behavior::Mouth` 遍历四邻域，每个邻居
                // 各抽一次 `bite_rate`，于是单嘴上限是 `4 × bite_rate`——0.30 的时候
                // 是 1.2/tick，满照光合只有 0.12，嘴成了 10 倍收益的机器。
                ui.checkbox(&mut cfg.life.mouth_one_bite, "一张嘴一口")
                    .on_hover_text(
                        "开（默认）：一个嘴细胞每 tick 只能咬住一个活体目标，\
                         收益上限就是「咬合速率」本身。\n\
                         关：退回旧行为——四个邻居各抽一次，上限是咬合速率的 4 倍。\n\
                         食物不受这条限制：能舔到几格食物由几何决定。",
                    );
                // ── 捕食端体型差（批次 23）──
                let before_adv = cfg.life.size_advantage;
                let mut adv = before_adv;
                ui.add(
                    egui::Slider::new(&mut adv, 0.0..=1.0)
                        .text("体型压制")
                        .step_by(0.05)
                        .fixed_decimals(2),
                )
                .on_hover_text(
                    "咬的收益 × (捕食者格数 / 猎物格数)^指数，夹在 0.25~4.0。\n\
                     0（默认）= 关闭，体型对捕食完全中性。\n\
                     0.5 是推荐起点：10 格咬 1 格 = 3.2 倍，1 格咬 10 格 = 0.32 倍。\n\
                     这是给大体型的第一条「进攻性」收益——之前它的好处全是防御性的。",
                );
                if (adv - before_adv).abs() > 1e-9 {
                    cfg.life.size_advantage = adv;
                }
                // ── 同种相残惩罚（批次 24）──
                let before_kin = cfg.life.kin_bite_penalty;
                let mut kin = before_kin;
                ui.add(
                    egui::Slider::new(&mut kin, 0.0..=1.0)
                        .text("同种相残")
                        .step_by(0.05)
                        .fixed_decimals(2),
                )
                .on_hover_text(
                    "同种之间咬的收益乘 (1 - 本值)。0（默认）= 关闭。\n\
                     打开之后捕食压力才会落到**别的物种**身上，\n\
                     红皇后 / 军备竞赛才有舞台。\n\
                     注意：判据是身体蓝图的指纹，变异一次就换种，\n\
                     所以母子可能被判成不同种——判据见下面的「同种按物种判」。",
                );
                if (kin - before_kin).abs() > 1e-9 {
                    cfg.life.kin_bite_penalty = kin;
                }
                // ── 同种判据（批次 32）──
                //
                // 指纹判据「变异一格就换种」，母子经常被判成异种——那正好是
                // 最该保护的一层。切成构成聚类之后母子/近亲都算得上，
                // 但口径**粗得多**（同一构成可能有几十个互不相干的个体），
                // 所以默认关：这是行为改动，不是口径改动。
                let mut kinspec = cfg.life.kin_by_species;
                if ui
                    .checkbox(&mut kinspec, "同种按物种判")
                    .on_hover_text(
                        "勾上：同种相残按**身体构成聚类**判（差得不多就算同种），母子、近亲都保护得到。
                         不勾（默认）：按身体蓝图指纹判——变异一格就换种，母子经常被判成异种。
                         注意：聚类口径粗得多，切过去会让「同种相残」从少数情况变成多数情况，
                         捕食可能整体塌掉。建议先看曲线再决定。",
                    )
                    .changed()
                {
                    cfg.life.kin_by_species = kinspec;
                }
                if cfg.life.kin_by_species {
                    let before_sd = cfg.life.species_distance;
                    let mut sd = before_sd;
                    ui.add(
                        egui::Slider::new(&mut sd, 0.02..=0.60)
                            .text("物种阈值")
                            .step_by(0.01)
                            .fixed_decimals(2),
                    )
                    .on_hover_text(
                        "两个身体的构成距离 ≤ 本值就算同一物种（默认 0.15）。
                         距离是 Jaccard 比例，和体型无关：14 格多一格 ≈ 0.07（同种）、
                         1 格嘴 vs 1 格生产者 = 1.0（完全不同的东西）。
                         这个阈值同时也是曲线里「物种(聚类)」那条线的判据。",
                    );
                    if (sd - before_sd).abs() > 1e-9 {
                        cfg.life.species_distance = sd;
                    }
                }
                // ── 捕食专精奖励（批次 31）──
                //
                // 混养体（生产者 + 嘴）永远赢：有底薪又吃红利。禁止混养是灾难
                // （99.7% 的成体都是混养体），所以改成**给放弃底薪的个体一个补偿**。
                let before_spec = cfg.life.predator_specialization;
                let mut spec = before_spec;
                ui.add(
                    egui::Slider::new(&mut spec, 1.0..=3.0)
                        .text("专精奖励")
                        .step_by(0.1)
                        .fixed_decimals(1),
                )
                .on_hover_text(
                    "身体里**没有生产者细胞**时，咬合力乘这个数（1.0 = 关闭）。

                     混养体完全不受影响——分化来自收益差，不是禁令。

                     纯捕食者放弃了底薪，所以给它一个补偿。",
                );
                if (spec - before_spec).abs() > 1e-9 {
                    cfg.life.predator_specialization = spec;
                }
                // ── 捕食者抗饿（批次 34）──
                //
                // 批次 31 把纯异养推到 6.9% 后饱和：瓶颈是"找不到猎物的 tick 会饿死"，
                // 不是"咬得不狠"。这一项给的是**生理差异**（捕食者本来就抗饿）。
                let before_starve = cfg.life.predator_starve_mult;
                let mut starve = before_starve;
                ui.add(
                    egui::Slider::new(&mut starve, 1.0..=6.0)
                        .text("捕食者抗饿")
                        .step_by(0.5)
                        .fixed_decimals(1),
                )
                .on_hover_text(
                    "身体里**没有生产者细胞**时，饥饿致死的宽限期乘这个数（1.0 = 关闭）。
                     默认 60 tick，×3 就是 180 tick。
                     这是生理差异：捕食是「饱一顿饿一顿」，光合是「每天都有底薪」。",
                );
                if (starve - before_starve).abs() > 1e-9 {
                    cfg.life.predator_starve_mult = starve;
                }
                // ── 大脑消融（批次 33）──
                //
                // 40→12→5 的网络从没做过对照。0.1% 的个体有运动细胞，
                // 所以这一项的差别本来就很微弱——那正是结论本身。
                ui.horizontal(|ui| {
                    ui.label("大脑");
                    for (v, name, tip) in [
                        (0u8, "正常", "演化出的大脑决定方向"),
                        (1u8, "恒直走", "输出清零：一直朝当前朝向走"),
                        (2u8, "随机走", "方向由 id+age 的哈希决定，大脑不参与"),
                    ] {
                        if ui
                            .selectable_label(cfg.life.brain_ablation == v, name)
                            .on_hover_text(tip)
                            .clicked()
                        {
                            cfg.life.brain_ablation = v;
                        }
                    }
                });
                // ── 循环连接（批次 37）──
                //
                // 批次 36 的结论：感知补上了，大脑还是打不过「恒直走」——
                // 因为直线走把"别回头"编码在运动里，而没有记忆的网络
                // 分不清"我在新地方"和"我在原地打转"。
                let mut rec = cfg.brain.recurrent;
                if ui
                    .checkbox(&mut rec, "循环连接（记忆）")
                    .on_hover_text(
                        "勾上：隐藏层多一项「上一 tick 的隐藏层」→ 网络有记忆。\n\
                         不勾（默认）：每 tick 独立看当前格，逐位等于旧行为。\n\
                         判据：配合「地力耗竭 + 运动可光合」，「正常」能不能打赢「恒直走」。",
                    )
                    .changed()
                {
                    cfg.brain.recurrent = rec;
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

                // ── 整口咬合（批次 16）──
                //
                // 血量是**体积**（每格 1 HP），而旧逻辑的伤害是**接触面积**——
                // 20 格的猎物会同时被 3~5 个杀手格咬到，每 tick 挨 3~5 点，
                // 20 点血 4~5 tick 就没了。击杀时间 ∝ √n，体型几乎买不到生存时间。
                ui.checkbox(&mut cfg.combat.killer_bite_capped, "整口咬合")
                    .on_hover_text(
                        "开（默认）：一个个体每 tick 对同一个目标只咬一口，
                         伤害 = 单格伤害 × 自己的杀手细胞数，**不按接触面积叠加**。
                         于是击杀时间 = 目标血量 ÷ 咬合力：1 格 1 tick、5 格 5 tick、
                         20 格 20 tick —— 体型第一次成为真实的防御。
                         关：退回旧行为（每个接触格各算一次），击杀时间只 ∝ √体型。",
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

                // ── 体型与繁殖（批次 11）──
                //
                // 这一组是**耦合的实验旋钮**：单独调一个往往看不到效果。
                // 原因都写在 `LifeCfg` 对应字段的文档里，这里只说操作后果。
                ui.separator();
                ui.label(egui::RichText::new("体型与繁殖").strong());

                let mut free = !cfg.life.repro_fail_costs_energy;
                if ui
                    .checkbox(&mut free, "繁殖失败不扣能量")
                    .on_hover_text(
                        "勾上（默认）：先检查空间，放得下才收费——\"找不到地方就继续攒着\"。
                         取消：先收费再检查，没地方生也白付一次成本（旧行为）。
                         实测旧行为下 86% 的尝试失败，繁殖成本实际是名义值的 7 倍，
                         而且失败会把能量清到 0，几次就饿死——繁殖失败本身成了主要死因。",
                    )
                    .changed()
                {
                    cfg.life.repro_fail_costs_energy = !free;
                }

                // ── 繁殖失败冷却（批次 26）──
                let before_cd = cfg.life.repro_cooldown;
                let mut cd = before_cd as f64;
                ui.add(
                    egui::Slider::new(&mut cd, 0.0..=60.0)
                        .text("繁殖冷却")
                        .step_by(1.0)
                        .fixed_decimals(0),
                )
                .on_hover_text(
                    "繁殖失败后歇多少 tick 才能再试（0 = 默认，不歇）。\n\
                     不歇的话「找不到空地」的个体会每 tick 白跑一遍搜索：\n\
                     实测尝试次数涨 20 倍、tick 耗时 ×1.67。\n\
                     副作用可能比性能收益更有意思：\n\
                     「什么时候繁殖」第一次变成一件要等的事。",
                );
                if (cd - before_cd as f64).abs() > 0.5 {
                    cfg.life.repro_cooldown = cd.round().max(0.0) as u32;
                }

                let before_tries = cfg.life.repro_search_tries;
                let mut tries = before_tries;
                ui.add(
                    egui::Slider::new(&mut tries, 1..=16)
                        .text("繁殖搜索候选数")
                        .step_by(1.0),
                )
                .on_hover_text(
                    "每次繁殖最多试几个落点（1 = 旧行为，随机挑一个，不行就放弃）。
                     调大之后个体在放弃前会多试几个位置。
                     注意：它只提高成功率，**不会**让体型变大——
                     小身体需要的空地少，搜索越充分小体型越占优。",
                );
                if tries != before_tries {
                    cfg.life.repro_search_tries = tries;
                }

                let before_min = cfg.life.min_breed_cells;
                let mut min_cells = before_min;
                ui.add(
                    egui::Slider::new(&mut min_cells, 1..=8)
                        .text("最小可繁殖体型")
                        .step_by(1.0),
                )
                .on_hover_text(
                    "细胞数低于它的个体不能繁殖（1 = 旧行为，任何体型都能生）。
                     这是唯一实测能把体型稳定推上去的旋钮，
                     但生物学对应物牵强，而且会产生\"攒满能量却不繁殖\"的僵尸个体。",
                );
                if min_cells != before_min {
                    cfg.life.min_breed_cells = min_cells;
                }

                let before_up = cfg.life.upkeep_exponent;
                let mut up = before_up;
                ui.add(
                    egui::Slider::new(&mut up, 0.5..=1.5)
                        .text("维持成本指数")
                        .step_by(0.05)
                        .fixed_decimals(2),
                )
                .on_hover_text(
                    "总开销 = 各细胞成本之和 × 体型^指数。1.00 = 线性（默认）。
                     < 1 = Kleiber：单位体重的代谢率随体型下降（大动物吃得少、活得久）。
                     实测 0.75 能让世界多养活 43% 的个体，但推不动体型——
                     因为成本只占满照收入的 8%，折扣太小。",
                );
                if (up - before_up).abs() > 1e-9 {
                    cfg.life.upkeep_exponent = up;
                }

                // ── 成长过程（批次 12）──
                    });
                    ui.add_space(12.0);
                    ui.separator();
                    ui.vertical(|ui| {
                        ui.set_width(316.0);
                ui.label(egui::RichText::new("成长").strong());

                let mut grow = cfg.life.growth_enabled;
                if ui
                    .checkbox(&mut grow, "启用成长过程")
                    .on_hover_text(
                        "勾上：后代以 1 格的芽出生，花能量一格格长到母体体型，
                         长成之前不能繁殖。取消（默认）：出生即成年。
                         实测效果很大：平均体型 2.25 → 3.32、存活种 34 → 94、
                         异养占比 0% → 9.9%（第一次稳定出现）。",
                    )
                    .changed()
                {
                    cfg.life.growth_enabled = grow;
                }

                if cfg.life.growth_enabled {
                    let before_gc = cfg.life.growth_cost;
                    let mut gc = before_gc;
                    ui.add(
                        egui::Slider::new(&mut gc, 0.0..=3.0)
                            .text("每格生长成本")
                            .step_by(0.05)
                            .fixed_decimals(2),
                    )
                    .on_hover_text(
                        "长一格要花的能量（默认 1.0）。
                         调大 = 生长更慢更贵；0 = 只花时间不花能量。",
                    );
                    if (gc - before_gc).abs() > 1e-9 {
                        cfg.life.growth_cost = gc;
                    }

                    let before_sf = cfg.life.growth_seed_fraction;
                    let mut sf = before_sf;
                    ui.add(
                        egui::Slider::new(&mut sf, 0.0..=1.0)
                            .text("亲代投资")
                            .step_by(0.05)
                            .fixed_decimals(2),
                    )
                    .on_hover_text(
                        "母体付的繁殖成本里，有多大比例转给芽当启动能量（默认 0.5）。
                         调 0 时芽出生就是 1 格 1 能量，实测杀手占比会飙到 50%+、
                         两个种子直接灭绝——幼体太脆，捕食者会在繁殖环节大爆发。",
                    );
                    if (sf - before_sf).abs() > 1e-9 {
                        cfg.life.growth_seed_fraction = sf;
                    }

                    // 环境可塑：芽长不动够久就提前成熟（体型小但能繁殖）。
                    // 0 = 关闭（必须长满，默认）。
                    let before_st = cfg.life.growth_stall_mature;
                    let mut st_v = before_st.min(255) as u8;
                    ui.add(
                        egui::Slider::new(&mut st_v, 0..=120)
                            .text("提前成熟阈值")
                            .step_by(1.0),
                    )
                    .on_hover_text(
                        "芽连续多少个 tick 长不动就「提前成熟」，0 = 关闭（必须长满）。
                         长不动 = 能量不够或目标格被占，两者都算。
                         建议 30~60（饥饿致死是 60 tick，太大等不到成熟就饿死）。
                         实测：关闭时体型 3.01 / 人口 1710；调到 30 时
                         体型 1.69 / 人口 2638 / 存活种 112 —— 这就是 r/K 的两端。",
                    );
                    if st_v as u32 != before_st {
                        cfg.life.growth_stall_mature = st_v as u32;
                    }

                    // 最短成长期：芽必须活够 N tick 才能成年。
                    //
                    // 这一项是「1 格目标 = 出生即成年 = 立刻繁殖」这条捷径的补丁。
                    // 没有它，整个成长机制会塌成"1 格最优"（实测平均体型 1.69）；
                    // 20 tick 能到 4~8 格，40 tick 能到 11 格。
                    let before_mg = cfg.life.growth_min_ticks;
                    let mut mg = before_mg.min(255) as u8;
                    ui.add(
                        egui::Slider::new(&mut mg, 0..=120)
                            .text("最短成长期")
                            .step_by(1.0),
                    )
                    .on_hover_text(
                        "芽必须活够这么多 tick 才能成年，0 = 关闭。
                         为什么需要它：`growth_plan` 是母体成熟时的体型，所以 1 格母体的
                         后代目标就是 1 格——它出生那一 tick 就「长满了」，立刻成年、立刻繁殖。
                         于是「小」变成免费策略，体型在任何环境里都涨不上去。
                         实测（6000 tick）：0 → 体型 1.69；15 → 3.35；20 → 4 左右；
                         25 → 7.33；40 → 11.58（人口从 2682 降到 519）。
                         它是**下限**不是加时：3 格身体本来就要长 20~40 tick，
                         所以这一项只惩罚「什么都不用长」的个体。建议 15~25。",
                    );
                    if mg as u32 != before_mg {
                        cfg.life.growth_min_ticks = mg as u32;
                    }

                    // ── 卵生（批次 27）──
                    //
                    // 和出芽是两条并行的路，生态学上正好是 r / K 两端：
                    // 出芽（K）芽贴母体、母体供养；卵生（r）产在空地上、母体不管。
                    // 卵**占一格、挡路、能被嘴吃** —— 捕食压力第一次进到繁殖环节。
                    let mut egging = cfg.life.egging;
                    if ui
                        .checkbox(&mut egging, "卵生")
                        .on_hover_text(
                            "勾上：普通繁殖改成产卵。卵占一格、挡路、可以被嘴吃掉，
                             到期才孵出一格芽。

                             和「出芽」是两条路：出芽是 K（母体养），卵生是 r（自己扛）。
                             建议和「同种相残」一起用 —— 否则捕食者会先把自己的卵吃光。",
                        )
                        .changed()
                    {
                        cfg.life.egging = egging;
                    }
                    if cfg.life.egging {
                        let before_eh = cfg.life.egg_hatch_ticks;
                        let mut eh = before_eh as f64;
                        ui.add(
                            egui::Slider::new(&mut eh, 20.0..=600.0)
                                .text("孵化时间")
                                .step_by(10.0)
                                .fixed_decimals(0),
                        )
                        .on_hover_text(
                            "卵在世界上暴露多少 tick 才孵化（默认 120 ≈ 2 秒）。
                             太长 = 全被吃光；太短 = 等于直接生。",
                        );
                        if (eh - before_eh as f64).abs() > 0.5 {
                            cfg.life.egg_hatch_ticks = eh.round().max(1.0) as u32;
                        }
                    }

                    // ── 出芽（批次 15）──
                    //
                    // 芽不生在 4~6 格外的空地，而是**紧贴母体**，由母体每 tick 供养。
                    // 关键不是"省了找地方"（那件事偏向小体型），而是
                    // **芽位随体型涨**：只有够大的身体才拿得到芽位。
                    let mut bud = cfg.life.budding_enabled;
                    if ui
                        .checkbox(&mut bud, "出芽")
                        .on_hover_text(
                            "勾上：芽紧贴母体出生、由母体每 tick 供养，芽位随体型涨。
                             不到芽位门槛的身体退回普通繁殖（不会绝育）。
                             实测（6000 tick、最短成长期 20）：体型 4.76 → 5.57、
                             有光占用 35.9% → 52.6%、异养 2.4% → 26.0%。",
                        )
                        .changed()
                    {
                        cfg.life.budding_enabled = bud;
                    }

                    if cfg.life.budding_enabled {
                        let before_bp = cfg.life.bud_cells_per_slot;
                        let mut bp = before_bp;
                        ui.add(
                            egui::Slider::new(&mut bp, 1.0..=20.0)
                                .text("每芽位格数")
                                .step_by(1.0)
                                .fixed_decimals(0),
                        )
                        .on_hover_text(
                            "多少格身体换一个芽位（同时能挂几个未成熟的芽）。
                             这个数**必须够大**：出芽最直接的效果是「出生不用再找空地」，
                             而让出生变容易永远是偏向小体型的。实测每芽位 2 格时体型
                             1.80、4 格 2.59、6 格 3.15、8 格 3.70 —— 越小越塌。
                             默认 8，也就是只有 8 格以上的身体才能出芽。",
                        );
                        if (bp - before_bp).abs() > 1e-9 {
                            cfg.life.bud_cells_per_slot = bp;
                        }

                        let before_bf = cfg.life.bud_feed;
                        let mut bf = before_bf;
                        ui.add(
                            egui::Slider::new(&mut bf, 0.0..=2.0)
                                .text("母体供养")
                                .step_by(0.05)
                                .fixed_decimals(2),
                        )
                        .on_hover_text(
                            "母体每 tick 给每个未成熟芽的能量（满照一个生产者自己挣 0.12）。
                             喂得少（0.03）时芽主要靠自己攒，而「自己攒多久」正比于目标体型，
                             大体型的芽长得慢、芽位长期占满 —— 实测体型只有 3.70。
                             喂得足（0.5）时芽的成长期被压到「最短成长期」那个固定值上，
                             芽位周转快，大体型才真正赢（体型 5.57）。",
                        );
                        if (bf - before_bf).abs() > 1e-9 {
                            cfg.life.bud_feed = bf;
                        }
                    }
                }

                // ── 环境温度（批次 11）──
                //
                // 温度场一直存在（岩浆驱动），但以前**对生物毫无作用**，只是着色。
                // 这一组把它接进模拟：热区变成一道"体型筛选"的墙。
                ui.separator();
                ui.label(egui::RichText::new("环境温度").strong());

                let before_td = cfg.life.temp_damage;
                let mut td = before_td;
                ui.add(
                    egui::Slider::new(&mut td, 0.0..=3.0)
                        .text("温度伤害")
                        .step_by(0.05)
                        .fixed_decimals(2),
                )
                .on_hover_text(
                    "0 = 关闭（默认，与旧行为逐位相同）。
                     每 tick 按所在格温度扣血：超过「舒适阈值」的部分平方放大。
                     个体身上最热的一格说了算——把一根手指伸进岩浆也要付代价。",
                );
                if (td - before_td).abs() > 1e-9 {
                    cfg.life.temp_damage = td;
                }

                // 环境温度：整个世界的基础温度。
                // 温度场存的是"高于它的温差"，所以调高这一项 = 全球变暖。
                let before_amb = cfg.world.ambient_temp;
                let mut amb = before_amb;
                ui.add(
                    egui::Slider::new(&mut amb, 0.0..=60.0)
                        .text("环境温度 ℃")
                        .step_by(1.0)
                        .fixed_decimals(0),
                )
                .on_hover_text(
                    "整个世界的基础温度（默认 22℃）。\n\
                     温度场存的是「高于环境的温差」，岩浆钉在满值，\n\
                     所以岩浆的温度 = 环境温度 + 量程（默认 +100℃）。\n\
                     调到 40℃ 以上，连常温区都会开始扣血——全球变暖旋钮。",
                );
                if (amb - before_amb).abs() > 1e-9 {
                    cfg.world.ambient_temp = amb;
                }

                let before_tc = cfg.life.comfort_temp;
                let mut tc = before_tc;
                ui.add(
                    egui::Slider::new(&mut tc, 0.0..=120.0)
                        .text("舒适温度 ℃")
                        .step_by(1.0)
                        .fixed_decimals(0),
                )
                .on_hover_text(
                    "局部温度低于它就不受伤（默认 35℃）。\n\
                     环境 22℃ + 量程 100℃ → 岩浆约 122℃，\n\
                     所以贴着岩浆必然受伤，离得远才安全。",
                );
                if (tc - before_tc).abs() > 1e-9 {
                    cfg.life.comfort_temp = tc;
                }

                let before_te = cfg.life.temp_size_exponent;
                let mut te = before_te;
                ui.add(
                    egui::Slider::new(&mut te, -0.75..=1.0)
                        .text("温度伤害指数")
                        .step_by(0.05)
                        .fixed_decimals(2),
                )
                .on_hover_text(
                    "伤害 ∝ 体型^指数。-0.25（默认）= 热容/散热比：
                     小个体升温快，10 格能撑 17.8 倍于 1 格的时间。
                     0 = 与体型无关（大个体只靠血厚）；+0.75 = 伤害按接触面积算（优势只剩 1.78 倍）。",
                );
                if (te - before_te).abs() > 1e-9 {
                    cfg.life.temp_size_exponent = te;
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
                });
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
    } else if id == CREATURE_ERASER {
        Color32::from_rgb(148, 163, 184)
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
        "擦地形"
    } else if id == CREATURE_ERASER {
        "擦生物"
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
    } else if id == CREATURE_ERASER {
        "只擦生物：整只移除，不留残骸。地形与食物不动".to_string()
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
        for tool in [
            Tool::Paint,
            Tool::Pan,
            Tool::Inspect,
            Tool::Light,
            Tool::LightErase,
        ] {
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
    fn brush_list_covers_registry_and_both_erasers() {
        let mut ids: Vec<u8> = evosim_core::cell_type::TERRAIN_BRUSH_TYPES.to_vec();
        ids.push(crate::paint::ERASER);
        ids.push(crate::paint::CREATURE_ERASER);
        assert!(ids.contains(&evosim_core::cell_type::ct::WALL));
        assert!(ids.contains(&evosim_core::cell_type::ct::LAVA));
        assert!(ids.contains(&crate::paint::ERASER));
        assert!(ids.contains(&crate::paint::CREATURE_ERASER));
        // 「空」不再是笔刷：擦除拆成擦地形 / 擦生物两个明确的意图。
        assert!(
            !ids.contains(&evosim_core::cell_type::ct::EMPTY),
            "「空」不该出现在笔刷里"
        );
        assert_eq!(ids.len(), 8, "6 个地形笔刷 + 两个橡皮擦");
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
