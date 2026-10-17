//! 曲线窗口：多指标折线 + 缩放平移 + 悬停读数。
//!
//! 设计上两个决定值得说明：
//!
//! 1. **每条曲线各自归一化到自己的峰值**（不是共用一根 Y 轴）。种群 1500 和物种 87
//!    放同一根轴上，后者会贴成一条直线。代价是 Y 轴刻度只能标百分比，绝对量写在图例里。
//! 2. **视图窗口以"采样点下标"为单位**，不是 tick。采样间隔会自适应翻倍
//!    （见 `Panels::sample`），用 tick 记窗口在抽稀后会错位。

use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Sense, Stroke, StrokeKind, Vec2};

use crate::panels::{Panels, Sample};
use crate::ui::{ACCENT, BORDER, TEXT, TEXT_DIM};

pub struct MetricDef {
    pub name: &'static str,
    pub color: Color32,
    pub get: fn(&Sample) -> f32,
    /// 小数位
    pub digits: usize,
    /// 单位后缀
    pub unit: &'static str,
}

pub static METRICS: [MetricDef; 6] = [
    MetricDef {
        name: "种群",
        color: ACCENT,
        get: |s| s.pop,
        digits: 0,
        unit: "",
    },
    MetricDef {
        name: "食物",
        color: Color32::from_rgb(125, 147, 168),
        get: |s| s.food,
        digits: 0,
        unit: "",
    },
    MetricDef {
        name: "物种",
        color: Color32::from_rgb(216, 180, 254),
        get: |s| s.species,
        digits: 0,
        unit: "",
    },
    MetricDef {
        name: "体型",
        color: Color32::from_rgb(251, 191, 36),
        get: |s| s.body,
        digits: 2,
        unit: " 格",
    },
    MetricDef {
        name: "变异率",
        color: Color32::from_rgb(94, 234, 212),
        get: |s| s.mutability,
        digits: 1,
        unit: "%",
    },
    MetricDef {
        name: "年龄",
        color: Color32::from_rgb(248, 113, 113),
        get: |s| s.age,
        digits: 0,
        unit: " tick",
    },
];

const PLOT_H: f32 = 210.0;
/// 窗口最少显示多少个采样点（再少就没法看了）
const MIN_SPAN: f32 = 4.0;

pub fn draw(ui: &mut egui::Ui, panels: &mut Panels) {
    // ── 指标开关 ──
    ui.horizontal_wrapped(|ui| {
        for (i, m) in METRICS.iter().enumerate() {
            let mut on = panels.chart_on[i];
            // 用带色的小方块代替普通 checkbox，一眼能对上曲线颜色
            let label = egui::RichText::new(m.name)
                .color(if on { m.color } else { TEXT_DIM })
                .size(11.0);
            if ui.selectable_label(on, label).clicked() {
                on = !on;
                panels.chart_on[i] = on;
            }
        }
        ui.separator();
        if ui
            .selectable_label(panels.chart_follow, "跟随最新")
            .on_hover_text("缩放/平移之后打开它，视图会一直贴住最新的数据")
            .clicked()
        {
            panels.chart_follow = !panels.chart_follow;
        }
        if ui.button("全览").on_hover_text("显示全部历史（双击图也能这样）").clicked() {
            panels.chart_view = None;
        }
    });

    ui.add_space(2.0);

    let w = ui.available_width().min(544.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, PLOT_H), Sense::click_and_drag());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, CornerRadius::ZERO, Color32::from_rgb(13, 17, 23));
    p.rect_stroke(
        rect,
        CornerRadius::ZERO,
        Stroke::new(1.0, BORDER),
        StrokeKind::Inside,
    );

    // 绘图区：左边留 32px 给 Y 轴百分比，顶部留 13px 给时间刻度。
    // 不留出来的话，0% 那个刻度和左下角的 tick 数字会叠在一起。
    let plot = egui::Rect::from_min_max(
        egui::pos2(rect.left() + 32.0, rect.top() + 17.0),
        egui::pos2(rect.right() - 4.0, rect.bottom() - 4.0),
    );

    let n = panels.history.len();
    if n < 2 {
        p.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "采样中…（每 20 tick 一个点）",
            FontId::proportional(11.0),
            TEXT_DIM,
        );
        return;
    }

    let last = (n - 1) as f32;

    // ── 视图窗口（单位：采样点下标）──
    let mut span = MIN_SPAN;
    let mut start = 0.0f32;
    if let Some((s, l)) = panels.chart_view {
        span = l.clamp(MIN_SPAN, last.max(MIN_SPAN));
        start = if panels.chart_follow { last - span } else { s };
        start = start.clamp(0.0, (last - span).max(0.0));
    } else {
        span = last;
    }

    // ── 交互：滚轮缩放（以光标为锚）+ 拖动平移 ──
    if resp.hovered() {
        let scroll = ui.input(|i| i.raw_scroll_delta.y);
        if scroll.abs() > 0.5 {
            if let Some(mouse) = resp.hover_pos() {
                let t = ((mouse.x - plot.left()) / plot.width()).clamp(0.0, 1.0);
                // 向上滚放大（窗口变短）。滚轮 delta 的单位随平台/后端变
                // （Windows 实测一次约 90 点），所以除了系数还要 clamp：
                // 不管 delta 报多大，一帧最多缩放 2 倍，手感才不会失控。
                let factor = (-scroll * 0.001).exp().clamp(0.5, 2.0);
                let new_span = (span * factor).clamp(MIN_SPAN, last.max(MIN_SPAN));
                let anchor = start + span * t;
                let new_start = (anchor - new_span * t).clamp(0.0, (last - new_span).max(0.0));
                panels.chart_view = Some((new_start, new_span));
                panels.chart_follow = false;
            }
        }
    }
    if resp.dragged() {
        let dx = resp.drag_delta().x;
        if dx.abs() > 0.0 {
            let shift = -dx / plot.width() * span;
            let new_start = (start + shift).clamp(0.0, (last - span).max(0.0));
            panels.chart_view = Some((new_start, span));
            panels.chart_follow = false;
        }
    }
    if resp.double_clicked() {
        panels.chart_view = None;
        panels.chart_follow = true;
    }

    // 拖动会改 panels.chart_view，但这一帧仍按旧值画 —— 差一帧无所谓，
    // 反过来在这里重新读一遍会让"拖动中"的画面对不上鼠标。

    // ── 网格 + Y 轴百分比刻度 ──
    for k in 0..=4 {
        let f = k as f32 / 4.0;
        let y = plot.bottom() - plot.height() * f;
        p.line_segment(
            [egui::pos2(plot.left(), y), egui::pos2(plot.right(), y)],
            Stroke::new(1.0, Color32::from_rgb(22, 28, 38)),
        );
        p.text(
            egui::pos2(plot.left() - 4.0, y),
            Align2::RIGHT_CENTER,
            format!("{:.0}%", f * 100.0),
            FontId::proportional(8.0),
            Color32::from_rgb(82, 95, 114),
        );
    }

    let to_x = |t: f32| plot.left() + plot.width() * ((t - start) / span);
    let to_y = |v: f32, max: f32| plot.bottom() - plot.height() * (v / max.max(1e-6)).clamp(0.0, 1.0);

    // 视图覆盖的下标区间（含两端各多取一个，避免边缘断线）
    let i0 = (start.floor().max(0.0)) as usize;
    let i1 = ((start + span).ceil() as usize).min(n - 1);

    let mut any = false;
    let mut peaks: [f32; 6] = [0.0; 6];
    let mut cur: [f32; 6] = [0.0; 6];

    for (mi, m) in METRICS.iter().enumerate() {
        if !panels.chart_on[mi] {
            continue;
        }
        any = true;
        let mut max = 0.0f32;
        for i in i0..=i1 {
            let v = (m.get)(&panels.history[i]);
            if v > max {
                max = v;
            }
        }
        peaks[mi] = max;
        cur[mi] = (m.get)(&panels.history[n - 1]);

        let pts: Vec<Pos2> = (i0..=i1)
            .map(|i| {
                let s = &panels.history[i];
                egui::pos2(to_x(i as f32), to_y((m.get)(s), max))
            })
            .collect();
        if pts.len() >= 2 {
            p.add(egui::Shape::line(pts, Stroke::new(1.5, m.color)));
        }
    }

    if !any {
        p.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "上面一个指标都没选",
            FontId::proportional(11.0),
            TEXT_DIM,
        );
    }

    // ── 悬停：竖线 + 交点 + 读数 ──
    let mut hover_i: Option<usize> = None;
    if let Some(mouse) = resp.hover_pos() {
        if plot.contains(mouse) {
            let t = start + (mouse.x - plot.left()) / plot.width() * span;
            let i = t.round().clamp(0.0, last) as usize;
            hover_i = Some(i);
            let x = to_x(i as f32);
            p.line_segment(
                [egui::pos2(x, plot.top()), egui::pos2(x, plot.bottom())],
                Stroke::new(1.0, Color32::from_rgb(90, 105, 125)),
            );
            for (mi, m) in METRICS.iter().enumerate() {
                if !panels.chart_on[mi] || peaks[mi] <= 0.0 {
                    continue;
                }
                let v = (m.get)(&panels.history[i]);
                p.circle_filled(
                    egui::pos2(x, to_y(v, peaks[mi])),
                    2.5,
                    m.color,
                );
            }
        }
    }

    // ── 时间轴刻度（顶部，避开左下角的 0%）──
    let tick_at = |i: usize| panels.history[i].tick;
    p.text(
        egui::pos2(plot.left(), rect.top() + 1.0),
        Align2::LEFT_TOP,
        format!("tick {}", tick_at(i0)),
        FontId::proportional(9.0),
        TEXT_DIM,
    );
    p.text(
        egui::pos2(plot.right(), rect.top() + 1.0),
        Align2::RIGHT_TOP,
        format!("{}", tick_at(i1)),
        FontId::proportional(9.0),
        TEXT_DIM,
    );

    // ── 图例 / 读数 ──
    ui.add_space(3.0);
    match hover_i {
        Some(i) => {
            let s = &panels.history[i];
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new(format!("tick {}", s.tick))
                        .color(TEXT)
                        .size(11.0)
                        .strong(),
                );
                for (mi, m) in METRICS.iter().enumerate() {
                    if !panels.chart_on[mi] {
                        continue;
                    }
                    ui.label(
                        egui::RichText::new(format!(
                            "{} {:.digits$}{}",
                            m.name,
                            (m.get)(s),
                            m.unit,
                            digits = m.digits
                        ))
                        .color(m.color)
                        .size(11.0),
                    );
                }
            });
        }
        None => {
            ui.horizontal_wrapped(|ui| {
                for (mi, m) in METRICS.iter().enumerate() {
                    if !panels.chart_on[mi] {
                        continue;
                    }
                    ui.label(
                        egui::RichText::new(format!(
                            "{} {:.digits$}{}",
                            m.name,
                            cur[mi],
                            m.unit,
                            digits = m.digits
                        ))
                        .color(m.color)
                        .size(11.0),
                    );
                    ui.label(
                        egui::RichText::new(format!(
                            "峰 {:.digits$}",
                            peaks[mi],
                            digits = m.digits
                        ))
                        .color(Color32::from_rgb(116, 128, 148))
                        .size(10.0),
                    );
                    ui.add_space(4.0);
                }
            });
        }
    }

    // ── 底部状态：范围 / 采样密度 / 操作提示 ──
    let view_ticks = tick_at(i1).saturating_sub(tick_at(i0));
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!(
                "跨度 {} tick · {} 点 · 每 {} tick 一点",
                view_ticks,
                i1 - i0 + 1,
                panels.sample_interval
            ))
            .color(TEXT_DIM)
            .size(10.0),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new("滚轮缩放 · 拖动平移 · 双击全览")
                    .color(Color32::from_rgb(90, 102, 120))
                    .size(10.0),
            );
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_table_covers_every_slot() {
        // `Panels::chart_on` 是 [bool; 6]，指标表少一项就会越界/漏画
        assert_eq!(METRICS.len(), 6);
        for m in METRICS.iter() {
            assert!(!m.name.is_empty());
        }
    }

    #[test]
    fn metrics_read_distinct_fields() {
        // 同一个样本上，各指标不该全都返回同一个数（说明 get 没接错字段）
        let s = Sample {
            tick: 100,
            pop: 11.0,
            food: 22.0,
            species: 33.0,
            body: 44.0,
            mutability: 55.0,
            age: 66.0,
            movers: 77.0,
            killers: 88.0,
        };
        let vals: Vec<f32> = METRICS.iter().map(|m| (m.get)(&s)).collect();
        let mut uniq = vals.clone();
        uniq.sort_by(|a, b| a.partial_cmp(b).unwrap());
        uniq.dedup();
        assert_eq!(uniq.len(), vals.len(), "有两个指标读到了同一个字段：{:?}", vals);
    }
}
