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
    /// 物种面板：谱系树模式（DFS 排序 + 父子连线）。关掉就是老的按种群数平铺
    pub species_tree: bool,
    /// 物种面板：只留活着的物种（以及它们的祖先）
    pub species_alive_only: bool,
    /// 物种面板：最少累计个体数，滤掉"一次性突变"噪声
    pub species_min_cum: u32,
    /// 物种面板：时间轴窗口（tick 范围）。`None` = 全览 0..now
    pub species_view: Option<(f64, f64)>,
    /// 拖拽选区时按下那一刻的视图快照（拖拽全程以它为基准，见 `species_body`）
    species_drag: Option<(f64, f64)>,
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
            show_inspector: false,
            show_species: false,
            show_chart: false,
            brain_show_all: false,
            chart_on: [true, true, true, false, false, false],
            chart_view: None,
            chart_follow: true,
            sample_interval: 20,
            species_tree: true,
            species_alive_only: false,
            // 默认 5 而不是 2：实测累计个体 < 5 的物种占全部登记物种的 ~69%，
            // 它们是"变异出来一代就没了"的中间态。这些留在树上只会把真正的
            // 分叉结构挤成一条竖链，树读不出谱系。
            species_min_cum: 5,
            species_view: None,
            species_drag: None,
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
        // 时间轴窗口是 tick 范围，换了世界就完全对不上了
        self.species_view = None;
        self.species_drag = None;
    }

    /// 检视工具点了某一格（`owner` 是那一格的生物 id，`-1` = 空地）。
    ///
    /// **点了生物必须顺带把检视器打开**。批次 17 把窗口默认值全改成了关，
    /// 于是这里只设 `selected_org` 的话，用户拿检视工具点半天什么都不会出现
    /// —— 面板是关的，窗口根本不画。
    ///
    /// 点空地只清选中，**不关面板**：用户可能只是想取消选中再点下一个，
    /// 每点一下空地就关一次窗口会很难用。
    pub fn on_inspect(&mut self, owner: i32) {
        // 图钉锁着的时候不换目标，但仍然保证面板是开着的。
        if self.pinned && self.selected_org.is_some() {
            if owner >= 0 {
                self.show_inspector = true;
            }
            return;
        }
        if owner >= 0 {
            self.selected_org = Some(owner as u32);
            self.show_inspector = true;
        } else {
            self.selected_org = None;
            self.follow = false;
        }
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
            // 存活物种，不是累计物种。
            //
            // `size()` 是**累计**创建过多少物种，单调爬升、撞 MAX_SPECIES 后变平，
            // 画在曲线上一路爬到顶就横着，跟种群曲线毫无关系，看着像坏了。
            // JS 版（main.js:431 / ui/Stats.js:100）一直用的是 aliveCount，这里是移植时写错。
            species: world.species.alive_count as f32,
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
            egui::Window::new("物种（谱系 · 时间轴）")
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

    // ── 控制行 ──
    //
    // 用 `selectable_label` 而不是 `checkbox`：egui 的 checkbox 点方框不生效、
    // 只有点到文字才算，很容易被当成坏了。
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        if ui
            .selectable_label(panels.species_tree, "谱系树")
            .on_hover_text("按祖先关系排序：同一支的后代紧挨在父物种下面，竖线是分叉点")
            .clicked()
        {
            panels.species_tree = !panels.species_tree;
        }
        if ui
            .selectable_label(panels.species_alive_only, "只留活着的血统")
            .on_hover_text("只显示现在还活着的物种，加上它们的祖先")
            .clicked()
        {
            panels.species_alive_only = !panels.species_alive_only;
        }
        ui.label(egui::RichText::new("最少累计").color(TEXT_DIM).size(10.0));
        for k in [1u32, 2, 5, 10] {
            if ui
                .selectable_label(panels.species_min_cum == k, k.to_string())
                .on_hover_text("累计出现过的个体数低于这个值的物种会被滤掉（一次性突变噪声）")
                .clicked()
            {
                panels.species_min_cum = k;
            }
        }
    });

    let tree = crate::lineage::build(
        reg,
        &crate::lineage::Opts {
            tree: panels.species_tree,
            alive_only: panels.species_alive_only,
            min_cum: panels.species_min_cum,
        },
    );

    // ── 时间轴窗口 ──
    //
    // 为什么需要它：谱系的分叉几乎全发生在前几千 tick 里（根物种先分裂出一串
    // 中间种，然后才稳定下来），而时间轴是 0..now 线性铺开。跑十万 tick 之后，
    // 所有分叉点挤在最左边十几个像素里，树看着就是一根竖线 —— 结构是对的，
    // 但读不出来。所以允许在时间轴上拖一段放大。
    let now_f = now as f64;
    let (mut t0, mut t1) = match panels.species_view {
        Some((a, b)) => (a.max(0.0), b.min(now_f)),
        None => (0.0, now_f),
    };
    // 读档 / 新建世界之后 now 可能变小，旧窗口整个跑到范围外 —— 直接回全览
    if !(t1 > t0) || t1 - t0 < 1.0 {
        t0 = 0.0;
        t1 = now_f;
        panels.species_view = None;
    }
    let zoomed = panels.species_view.is_some();

    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(
            egui::RichText::new(format!("显示 {} / {} 个物种", tree.rows.len(), reg.size()))
                .color(TEXT_DIM)
                .size(10.0),
        );
        if tree.hidden > 0 {
            ui.label(
                egui::RichText::new(format!("（滤掉 {}）", tree.hidden))
                    .color(Color32::from_rgb(120, 108, 70))
                    .size(10.0),
            );
        }
        if zoomed {
            ui.label(
                egui::RichText::new(format!("窗口 {:.0} → {:.0}", t0, t1))
                    .color(ACCENT)
                    .size(10.0),
            );
            if ui
                .selectable_label(false, "全览")
                .on_hover_text("回到 0 → 现在")
                .clicked()
            {
                panels.species_view = None;
                t0 = 0.0;
                t1 = now_f;
            }
        } else {
            ui.label(
                egui::RichText::new(if panels.species_tree {
                    "竖线 = 分叉 tick"
                } else {
                    "按个体数排序"
                })
                .color(TEXT_DIM)
                .size(10.0),
            );
        }
    });
    ui.separator();

    // 时间轴条带：拖一段放大、滚轮缩放、双击复位。
    //
    // 高度给到 22：12px 的细条既难点中，也完全看不出"这里可以拖"。
    let axis_w = (ui.available_width() - 6.0).max(180.0);
    let (axis_rect, axis_resp) =
        ui.allocate_exact_size(egui::vec2(axis_w, 22.0), Sense::click_and_drag());
    let axis_resp = axis_resp
        .on_hover_text("拖一段放大到那个时间范围 · 滚轮缩放 · 双击复位");
    {
        let p = ui.painter();
        // 底色：让它看起来是一条"可以操作的条带"，而不是一行字
        p.rect_filled(
            axis_rect,
            CornerRadius::ZERO,
            if axis_resp.hovered() {
                Color32::from_rgb(32, 40, 54)
            } else {
                Color32::from_rgb(24, 30, 42)
            },
        );
        p.text(
            axis_rect.left_top() + egui::vec2(2.0, 1.0),
            Align2::LEFT_TOP,
            format!("{:.0}", t0),
            FontId::proportional(9.0),
            TEXT_DIM,
        );
        p.text(
            axis_rect.right_top() + egui::vec2(-2.0, 1.0),
            Align2::RIGHT_TOP,
            format!("{:.0}", t1),
            FontId::proportional(9.0),
            TEXT_DIM,
        );
        // 四分之一刻度，给"拖多长"一个参照
        for k in 1..4 {
            let x = axis_rect.left() + axis_rect.width() * (k as f32 / 4.0);
            p.line_segment(
                [
                    egui::pos2(x, axis_rect.bottom() - 4.0),
                    egui::pos2(x, axis_rect.bottom()),
                ],
                Stroke::new(1.0_f32, Color32::from_rgb(44, 54, 70)),
            );
        }
        p.line_segment(
            [
                egui::pos2(axis_rect.left(), axis_rect.bottom()),
                egui::pos2(axis_rect.right(), axis_rect.bottom()),
            ],
            Stroke::new(1.0_f32, BORDER),
        );
    }

    // 把光标位置换算成 tick（0..1 的比例 → 当前窗口内）
    if axis_resp.drag_started() {
        // 拖拽期间每一帧都会触发 `dragged()`。如果每帧都拿**当前**视图去换算选区，
        // 就会在上一次的结果上继续缩，几帧之内窗口就缩到几个 tick（这个坑真踩过）。
        // 所以按下时先快照视图，整个拖拽过程都用这份快照做基准。
        panels.species_drag = Some((t0, t1));
    }
    if axis_resp.double_clicked() {
        panels.species_view = None;
        panels.species_drag = None;
        t0 = 0.0;
        t1 = now_f;
    } else if axis_resp.dragged() {
        // 拖动期间直接改视图：松手才生效会让人以为没反应
        if let (Some((d0, d1)), Some(a), Some(b)) = (
            panels.species_drag,
            ui.input(|i| i.pointer.press_origin()),
            ui.input(|i| i.pointer.interact_pos()),
        ) {
            let f = |x: f32| (((x - axis_rect.left()) / axis_rect.width()).clamp(0.0, 1.0)) as f64;
            let (fa, fb) = (f(a.x), f(b.x));
            let (lo, hi) = (fa.min(fb), fa.max(fb));
            // 选区太窄会缩进"看不见的刻度"里，留个下限
            if hi - lo > 0.004 {
                panels.species_view = Some((d0 + lo * (d1 - d0), d0 + hi * (d1 - d0)));
            }
        }
    }
    if axis_resp.drag_stopped() {
        panels.species_drag = None;
    }

    // 滚轮缩放（以光标为锚）。轴条不在滚动区里，所以不会和列表滚动抢事件。
    if axis_resp.hovered() {
        let scroll = ui.input(|i| i.raw_scroll_delta.y);
        if scroll != 0.0 {
            if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
                let f = (((pos.x - axis_rect.left()) / axis_rect.width()).clamp(0.0, 1.0)) as f64;
                // 和曲线窗口同样的处理：delta 单位随平台变，除了系数还要 clamp，
                // 不然一次滚轮就能把窗口缩没
                let factor = (-(scroll as f64) * 0.002).exp().clamp(0.4, 2.5);
                let span = ((t1 - t0) * factor).clamp(1.0, now_f);
                let anchor = t0 + f * (t1 - t0);
                let mut n0 = anchor - f * span;
                let mut n1 = n0 + span;
                if n0 < 0.0 {
                    n0 = 0.0;
                    n1 = span;
                }
                if n1 > now_f {
                    n1 = now_f;
                    n0 = (now_f - span).max(0.0);
                }
                if n1 - n0 > 1.0 {
                    panels.species_view = Some((n0, n1));
                }
            }
        }
    }

    let mut clicked: Option<String> = None;

    // 行高固定，用 `show_rows` 只铺可见的那几行：物种数能到四位数，
    // 全量 allocate 会让面板滚动卡成幻灯片。
    const ROW_H: f32 = 15.0;
    // 名字列宽度：要容得下 3px 内边距 + 最深缩进 18px + "S1234"
    const NAME_W: f32 = 64.0;
    // 右端留给当前个体数
    const POP_W: f32 = 30.0;

    if tree.rows.is_empty() {
        // 默认「最少累计」是 5，刚开的世界物种还很小，很容易一棵树都剩不下。
        // 不解释一句的话，用户会以为面板坏了。
        let mut why = format!("最少累计 {}", panels.species_min_cum);
        if panels.species_alive_only {
            why.push_str("、只留存活");
        }
        ui.label(
            egui::RichText::new(format!(
                "没有符合条件的物种（累计 {} 个，过滤：{}）—— 把「最少累计」调小试试",
                reg.size(),
                why
            ))
            .color(TEXT_DIM)
            .size(11.0),
        );
    } else {
        egui::ScrollArea::vertical()
            .max_height(300.0)
            .auto_shrink([false, true])
            .show_rows(ui, ROW_H, tree.rows.len(), |ui, range| {
                for i in range {
                    let row = &tree.rows[i];
                    let sp = &reg.list[row.idx];
                    let selected = panels
                        .selected_species
                        .as_deref()
                        .map(|fp| fp == sp.fingerprint.as_ref())
                        .unwrap_or(false);

                    let (rect, resp) =
                        ui.allocate_exact_size(egui::vec2(axis_w, ROW_H), Sense::click());
                    let painter = ui.painter();

                    // 行底色（悬停时提亮一点，能看出可点）
                    if resp.hovered() || selected {
                        painter.rect_filled(
                            rect,
                            CornerRadius::ZERO,
                            Color32::from_rgb(26, 33, 45),
                        );
                    }

                    let bar_x0 = rect.left() + NAME_W;
                    let bar_x1 = rect.right() - POP_W;
                    let span = (bar_x1 - bar_x0).max(10.0);
                    // tick → x，按当前时间窗口映射（全览时就是 0..now）
                    let x_of = |t: f64| {
                        let f = ((t - t0) / (t1 - t0)).clamp(0.0, 1.0);
                        bar_x0 + span * f as f32
                    };

                    let start = sp.start_tick as f64;
                    let end = if sp.end_tick >= 0 {
                        sp.end_tick as f64
                    } else {
                        now_f
                    };
                    // 整条落在窗口外就什么都不画（横条和连线都不画，
                    // 否则 x_of 会把它们夹到边缘，看起来像一堆假物种）
                    let on_screen = end >= t0 && start <= t1;

                    // ── 谱系连线 ──
                    //
                    // 一个非根物种只需要**一根竖线**：从父物种那一行、在"自己出现的
                    // 那个 tick"上竖下来，正好落到自己横条的左端。这是 DFS 排序换来的
                    // ——父子行挨得近，竖线只跨自己这一支，永远不会交叉。
                    //
                    // 父行可能在视口外，`dy` 会算出很大的负数；egui 会按滚动区裁剪，
                    // 于是看起来就是"线从上方延续下来"，正是想要的。
                    // 分叉点若在窗口左侧，`x_of` 会把它夹到左边缘：看起来是一条
                    // 沿着边界竖下来的"主干"，比整段连线消失要好——后者会让放大之后
                    // 的树看起来支离破碎。完全在窗口外的物种不画线（否则会在边界
                    // 堆出一列毫无意义的假分叉）。
                    if let (Some(pi), true) = (row.parent, on_screen) {
                        let x = x_of(start);
                        let y_me = rect.center().y;
                        let y_par = y_me + (pi as f32 - i as f32) * ROW_H;
                        painter.line_segment(
                            [egui::pos2(x, y_par), egui::pos2(x, y_me)],
                            Stroke::new(1.0_f32, Color32::from_rgb(56, 68, 86)),
                        );
                    }

                    // 左：名字（按代数缩进，封顶——深链能到上千层，
                    // 真按代数缩进会把名字挤出面板）
                    //
                    // 存续区间完全在当前时间窗口之外的物种仍然列出来（树的拓扑需要
                    // 它们当中间节点），但名字调暗，免得用户以为"这一行没画出来"。
                    let indent = (row.depth.min(6) as f32) * 3.0;
                    painter.text(
                        rect.left_center() + egui::vec2(3.0 + indent, 0.0),
                        Align2::LEFT_CENTER,
                        &sp.name,
                        FontId::proportional(10.0),
                        if selected {
                            ACCENT
                        } else if on_screen {
                            TEXT
                        } else {
                            Color32::from_rgb(78, 88, 104)
                        },
                    );

                    // 右：时间条（超出窗口的部分夹到边缘，视觉上表示"延续到窗口外"）
                    if on_screen {
                        let x0 = x_of(start.max(t0));
                        let x1 = x_of(end.min(t1)).max(x0 + 2.0);

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
                    }

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
                        "{}（{} 格）\n祖先 {}\n出现 tick {} → {}\n存续 {} tick · 当前 {} · 峰值 {} · 累计 {}",
                        sp.name,
                        sp.cell_count,
                        match sp.ancestor_id {
                            Some(a) => format!("S{}", a),
                            None => "—（根）".to_string(),
                        },
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
    }

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

    /// 跑两帧并把所有形状摊平（egui 的窗口第一帧是淡入，形状会被剔成 Noop）
    fn species_shapes(
        world: &World,
        panels: &mut Panels,
    ) -> (Vec<egui::epaint::RectShape>, Vec<egui::epaint::TextShape>) {
        let ctx = egui::Context::default();
        crate::ui::configure(&ctx);
        let mut hl: Vec<(egui::Rect, Color32)> = Vec::new();
        let mut input = egui::RawInput::default();
        input.screen_rect = Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1600.0, 950.0),
        ));
        let _ = ctx.run(input.clone(), |ctx| {
            let _ = panels.draw(ctx, world, &mut hl);
        });
        let out = ctx.run(input, |ctx| {
            let _ = panels.draw(ctx, world, &mut hl);
        });

        fn walk<'a>(
            shapes: &'a [egui::Shape],
            rects: &mut Vec<&'a egui::epaint::RectShape>,
            texts: &mut Vec<&'a egui::epaint::TextShape>,
        ) {
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
        (rects.into_iter().cloned().collect(), texts.into_iter().cloned().collect())
    }

    /// 拿检视工具点生物，**检视器必须自己打开**。
    ///
    /// 回归：批次 17 把窗口默认值全改成了关，于是 `inspect_at_cursor` 只设
    /// `selected_org` 的话，用户点半天什么都不会出现（面板关着，窗口不画）。
    #[test]
    fn inspecting_an_organism_opens_the_panel() {
        let mut p = Panels::default();
        assert!(!p.show_inspector, "默认应该是关的（批次 17）");

        p.on_inspect(7);
        assert_eq!(p.selected_org, Some(7), "应该选中那只生物");
        assert!(p.show_inspector, "点生物必须把检视器打开");

        // 点空地：清选中，但**不关面板**（否则连着点几下就得反复重开）
        p.on_inspect(-1);
        assert_eq!(p.selected_org, None, "点空地要清掉选中");
        assert!(p.show_inspector, "点空地不该把面板关掉");
    }

    /// 图钉锁着的时候不换目标，但仍然保证面板是开着的。
    #[test]
    fn pinned_inspector_keeps_its_target_but_stays_open() {
        let mut p = Panels::default();
        p.on_inspect(3);
        p.pinned = true;

        p.on_inspect(9);
        assert_eq!(p.selected_org, Some(3), "图钉锁着时不该换目标");
        assert!(p.show_inspector);

        // 手动关掉面板之后再点生物：仍然要打开
        p.show_inspector = false;
        p.on_inspect(9);
        assert!(p.show_inspector, "图钉状态下点生物也要把面板叫出来");
    }

    fn species_panels() -> Panels {
        let mut panels = Panels::default();
        panels.show_inspector = false;
        panels.show_chart = false;
        panels.show_species = true;
        // 这里测的是**绘制**逻辑，不是默认过滤强度。`live_world()` 只跑 150 tick，
        // 物种的累计个体数都还没到默认的 5，按默认过滤会一棵树都剩不下。
        panels.species_min_cum = 1;
        panels
    }

    /// 时间轴横条的判据：细长、有填充色（物种指纹色），且不是窗口底色
    fn count_bars(rects: &[egui::epaint::RectShape]) -> usize {
        rects
            .iter()
            .filter(|r| {
                r.rect.height() <= 10.0
                    && r.rect.width() >= 2.0
                    && r.fill.a() > 0
                    && r.fill != crate::ui::BG
            })
            .count()
    }

    /// 曲线采样的「物种」必须是**存活**物种，不是累计。
    ///
    /// 累计数是单调爬升的，撞 `MAX_SPECIES` 之后变平，画在曲线上就是一条爬到顶
    /// 就横着的线，跟种群曲线毫无关系——看着像"物种曲线坏了"。JS 版
    /// （`main.js:431` / `ui/Stats.js:100`）一直用的 `aliveCount`，2.0 移植时写成了
    /// `size()`，这条测试守着别再写回去。
    #[test]
    fn sample_records_alive_species_not_cumulative() {
        let mut world = live_world();
        // 跑到出现灭绝物种为止：只有那时两个数才分开，测试才有判别力
        for _ in 0..4000 {
            if world.species.size() > world.species.alive_count {
                break;
            }
            world.step();
        }
        assert!(
            world.species.size() > world.species.alive_count,
            "这个种子跑不出灭绝物种，测试失去判别力（累计 {} / 存活 {}）",
            world.species.size(),
            world.species.alive_count
        );

        let mut panels = Panels::default();
        panels.sample(&world);
        let s = panels.history.back().expect("应该采到一个点");
        assert_eq!(
            s.species,
            world.species.alive_count as f32,
            "采样要记存活物种（{}），不是累计（{}）",
            world.species.alive_count,
            world.species.size()
        );
    }

    /// 物种面板要真的画出时间轴（横条），不能只剩一行计数。
    #[test]
    fn species_timeline_draws_bars() {
        let world = live_world();
        assert!(world.species.size() > 0, "这个种子应该已经出现物种");

        let mut panels = species_panels();
        let (rects, texts) = species_shapes(&world, &mut panels);
        assert!(
            count_bars(&rects) > 0,
            "物种面板没有画出任何时间轴横条（rects={}）",
            rects.len()
        );

        // 刻度上要有当前 tick 的数字
        let now = world.tick.to_string();
        assert!(
            texts.iter().any(|t| t.galley.text() == now),
            "时间轴右端应标出当前 tick（{}）",
            now
        );
    }

    /// 放大时间窗口之后，横条必须按窗口裁剪。
    ///
    /// 这条守的是"假横条"：如果 `x_of` 把窗口外的 tick 直接夹到边界，
    /// 每个物种的条都会从左边拉到右边，整张图变成一片横贯全宽的色块，
    /// 看着像"所有物种同时存在"——比不画还误导。
    #[test]
    fn zoomed_timeline_clips_bars() {
        let world = live_world();
        let now = world.tick as f64;
        let mut panels = species_panels();
        // 只留最后 10% 的时间窗
        panels.species_view = Some((now * 0.9, now));

        let (rects, _) = species_shapes(&world, &mut panels);
        let bars: Vec<_> = rects
            .iter()
            .filter(|r| r.rect.height() <= 10.0 && r.rect.width() >= 2.0 && r.fill.a() > 0)
            .collect();
        assert!(!bars.is_empty(), "放大之后仍然要画得出横条");

        // 窗口是"最后 10%"，那么任何一条横条都不该宽到覆盖整条时间轴
        let widest = bars.iter().map(|r| r.rect.width()).fold(0.0_f32, f32::max);
        assert!(
            widest < 500.0,
            "放大到 10% 窗口后仍出现宽 {} px 的横条，说明没按窗口裁剪",
            widest
        );
    }
}
