//! 检视器：缩略图 + 行为区 + 神经网络图。
//!
//! 三块内容都要"看一眼就懂"，所以全部自绘：
//!
//! * **缩略图** —— 按 `rotation` 把身体的局部坐标转成世界朝向再画，
//!   跟画布上看到的形状一致（不然转了身的个体会看起来是歪的）。
//! * **行为区** —— 能量条 + 还差多少能繁殖 + 代谢 + 饥饿计时 + 上一 tick 的收支/移动结果。
//! * **神经网络** —— 40 → 12 → 5 的固定拓扑。默认**只画强连接**（|w| ≥ 最大权重的 45%），
//!   否则 480 + 60 条线糊成一团。勾上「显示全部」才全画。
//!
//! 这里只读世界，不写任何东西。

use egui::{Align2, Color32, CornerRadius, FontId, Pos2, Sense, Stroke, Vec2};

use evosim_core::anatomy::{rotate_col, rotate_row, Flags};
use evosim_core::cell_type::{self, VISIBLE_CELL_TYPES};
use evosim_core::organism::Organism;
use evosim_core::sensors::{SENSE_DIM, STATE_BASE, VISUAL_SLOTS, VISION_DIRS};
use evosim_core::world::World;

use crate::panels::{PanelActions, Panels};
use crate::ui::{ACCENT, BORDER, TEXT, TEXT_DIM};

const DIR_NAMES: [&str; 4] = ["上", "右", "下", "左"];

/// 缩略图边长
const THUMB: f32 = 112.0;

pub fn draw_body(
    ui: &mut egui::Ui,
    world: &World,
    org: &Organism,
    panels: &mut Panels,
    act: &mut PanelActions,
) {
    let sp_name = org
        .species_fingerprint
        .as_deref()
        .and_then(|fp| world.species.by_fingerprint(fp))
        .map(|s| s.name.clone())
        .unwrap_or_else(|| "?".to_string());

    // ── 标题 ──
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("个体 #{}", org.id))
                .strong()
                .color(TEXT),
        );
        ui.label(
            egui::RichText::new(format!("物种 {}", sp_name))
                .color(TEXT_DIM)
                .size(11.0),
        );
    });

    // ── 上：缩略图 | 行为 ──
    ui.add_space(2.0);
    ui.horizontal_top(|ui| {
        draw_anatomy(ui, org);
        ui.add_space(10.0);
        ui.vertical(|ui| {
            behavior_block(ui, world, org);
        });
    });

    ui.add_space(4.0);

    // ── 能力徽章 ──
    ui.horizontal_wrapped(|ui| {
        let mut any = false;
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
                any = true;
                ui.label(
                    egui::RichText::new(name)
                        .color(ACCENT)
                        .size(11.0)
                        .background_color(Color32::from_rgb(24, 40, 56)),
                );
            }
        }
        if !any {
            ui.label(egui::RichText::new("无特殊能力").color(TEXT_DIM).size(11.0));
        }
    });

    ui.label(
        egui::RichText::new(org.anatomy.describe())
            .color(TEXT_DIM)
            .size(10.0),
    );

    // ── 神经网络 ──
    ui.add_space(3.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("神经网络 40 → 12 → 5")
                .color(TEXT_DIM)
                .size(11.0),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.checkbox(&mut panels.brain_show_all, "显示全部")
                .on_hover_text("不勾时只画强连接（|权重| ≥ 最大值的 45%），勾上画全部 540 条");
        });
    });
    let hint = draw_brain(ui, org, panels.brain_show_all);
    ui.label(
        egui::RichText::new(hint)
            .color(TEXT_DIM)
            .size(10.0),
    );

    // ── 操作 ──
    ui.add_space(2.0);
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
    ui.horizontal(|ui| {
        ui.checkbox(&mut panels.highlight_species, "高亮同物种")
            .on_hover_text("除了它自己，把它物种的其他个体也用该物种的颜色标出来");
        if panels.pinned {
            ui.label(
                egui::RichText::new("已锁定")
                    .color(ACCENT)
                    .size(11.0),
            );
        }
    });
}

// ────────────────────────────── 缩略图 ──────────────────────────────

fn draw_anatomy(ui: &mut egui::Ui, org: &Organism) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(THUMB), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, CornerRadius::same(3), Color32::from_rgb(12, 16, 24));
    p.rect_stroke(
        rect,
        CornerRadius::same(3),
        Stroke::new(1.0_f32, BORDER),
        egui::StrokeKind::Inside,
    );

    let cells = &org.anatomy.cells;
    if cells.is_empty() {
        return;
    }

    // 包围盒（**旋转之后**的坐标，这样画出来跟画布上的朝向一致）
    let mut min_c = i32::MAX;
    let mut max_c = i32::MIN;
    let mut min_r = i32::MAX;
    let mut max_r = i32::MIN;
    let mut pts: Vec<(i32, i32, u8, u8)> = Vec::with_capacity(cells.len());
    for c in cells {
        let x = rotate_col(c.dx, c.dy, org.rotation);
        let y = rotate_row(c.dx, c.dy, org.rotation);
        min_c = min_c.min(x);
        max_c = max_c.max(x);
        min_r = min_r.min(y);
        max_r = max_r.max(y);
        pts.push((x, y, c.type_id, c.dir));
    }
    let bw = (max_c - min_c + 1).max(1) as f32;
    let bh = (max_r - min_r + 1).max(1) as f32;

    let pad = 8.0;
    let cell = ((rect.width() - pad * 2.0) / bw)
        .min((rect.height() - pad * 2.0) / bh)
        .clamp(2.0, 13.0);

    // 居中
    let ox = rect.center().x - bw * cell * 0.5;
    let oy = rect.center().y - bh * cell * 0.5;

    for &(x, y, ty, dir) in &pts {
        let gx = ox + (x - min_c) as f32 * cell;
        let gy = oy + (y - min_r) as f32 * cell;
        let r = egui::Rect::from_min_size(
            egui::pos2(gx, gy),
            Vec2::splat((cell - 0.6).max(1.0)),
        );
        let c = cell_type::def(ty)
            .map(|d| d.colors[0])
            .unwrap_or(0x7d93a8);
        let col = Color32::from_rgb(
            ((c >> 16) & 0xff) as u8,
            ((c >> 8) & 0xff) as u8,
            (c & 0xff) as u8,
        );
        p.rect_filled(r, CornerRadius::same(1), col);
        if cell >= 5.0 {
            p.rect_stroke(
                r,
                CornerRadius::same(1),
                Stroke::new(1.0_f32, Color32::from_rgb(8, 11, 16)),
                egui::StrokeKind::Inside,
            );
        }

        // 眼睛：从格子中心朝它看的方向画一条短线（旋转后）
        if ty == cell_type::ct::EYE && cell >= 4.0 {
            let d = ((org.rotation as usize + dir as usize) & 3) as usize;
            let v = egui::vec2(
                [0.0, 1.0, 0.0, -1.0][d],
                [-1.0, 0.0, 1.0, 0.0][d],
            ) * (cell * 0.42);
            let c0 = r.center();
            p.line_segment(
                [c0, c0 + v],
                Stroke::new((cell * 0.16).max(1.0), Color32::from_rgb(10, 14, 20)),
            );
        }
    }

    // 枢轴格（0,0）：画个空心圈，好认出"锚点在哪"
    if 0 >= min_c && 0 <= max_c && 0 >= min_r && 0 <= max_r {
        let px = ox + (0 - min_c) as f32 * cell + cell * 0.5;
        let py = oy + (0 - min_r) as f32 * cell + cell * 0.5;
        p.circle_stroke(
            egui::pos2(px, py),
            (cell * 0.62).max(2.0),
            Stroke::new(1.0_f32, Color32::from_rgb(255, 255, 255)),
        );
    }

    // 底部：朝向提示
    p.text(
        egui::pos2(rect.center().x, rect.bottom() - 2.0),
        Align2::CENTER_BOTTOM,
        format!(
            "{} 格 · 朝{}",
            cells.len(),
            DIR_NAMES[org.rotation as usize & 3]
        ),
        FontId::proportional(9.0),
        TEXT_DIM,
    );
}

// ────────────────────────────── 行为区 ──────────────────────────────

fn behavior_block(ui: &mut egui::Ui, world: &World, org: &Organism) {
    let cfg = &world.config;
    let at = org.reproduce_at(cfg);
    let need = org.food_needed(cfg);
    let upkeep = org.upkeep_cost(cfg);

    // 能量条
    let frac = if at > 0.0 {
        (org.energy / at).clamp(0.0, 1.0) as f32
    } else {
        0.0
    };
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("能量").color(TEXT_DIM).size(11.0));
        ui.label(
            egui::RichText::new(format!("{:.1} / {:.0}", org.energy, at))
                .color(TEXT)
                .size(11.0),
        );
    });
    let (bar, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width().min(210.0), 8.0),
        Sense::hover(),
    );
    let p = ui.painter_at(bar);
    p.rect_filled(bar, CornerRadius::same(2), Color32::from_rgb(16, 22, 30));
    if frac > 0.0 {
        let mut fill = bar;
        fill.set_width(bar.width() * frac);
        // 快能繁殖时变绿，快饿死时变红
        let col = if frac >= 0.85 {
            Color32::from_rgb(74, 222, 128)
        } else if frac < 0.2 {
            Color32::from_rgb(248, 113, 113)
        } else {
            ACCENT
        };
        p.rect_filled(fill, CornerRadius::same(2), col);
    }

    // 繁殖门槛：再画一条竖线，标出"实扣成本"的位置
    if at > 0.0 && need < at {
        let x = bar.left() + bar.width() * (need / at).clamp(0.0, 1.0) as f32;
        p.line_segment(
            [
                egui::pos2(x, bar.top() - 1.0),
                egui::pos2(x, bar.bottom() + 1.0),
            ],
            Stroke::new(1.0_f32, Color32::from_rgb(251, 191, 36)),
        );
    }

    ui.add_space(3.0);
    egui::Grid::new("org-behave")
        .num_columns(2)
        .spacing([8.0, 1.0])
        .show(ui, |ui| {
            let dim = |ui: &mut egui::Ui, k: &str, v: String| {
                ui.label(egui::RichText::new(k).color(TEXT_DIM).size(10.0));
                ui.label(egui::RichText::new(v).color(TEXT).size(10.0));
                ui.end_row();
            };
            dim(
                ui,
                "还差",
                format!("{:.1} 可繁殖（实扣 {:.0}，储备 {:.0}）", (at - org.energy).max(0.0), need, (at - need).max(0.0)),
            );
            dim(ui, "代谢", format!("{:.3} / tick", upkeep));
            dim(
                ui,
                "收支",
                {
                    let d = org.energy_gained_last;
                    format!("{}{:.2} / tick", if d >= 0.0 { "+" } else { "" }, d)
                },
            );
            dim(
                ui,
                "饥饿",
                format!("{} / {} tick", org.starve_ticks, cfg.life.starvation_ticks),
            );
            dim(
                ui,
                "移动",
                if !org.anatomy.has(Flags::MOVER) {
                    "不会移动".to_string()
                } else if org.last_move_ok {
                    "上一 tick 挪动了".to_string()
                } else if org.plan_turn {
                    "上一 tick 在转身".to_string()
                } else {
                    "上一 tick 被挡住".to_string()
                },
            );
            dim(
                ui,
                "计划",
                format!(
                    "{}{}",
                    DIR_NAMES[org.plan_direction as usize & 3],
                    if org.plan_turn { "（转身）" } else { "" }
                ),
            );
            dim(
                ui,
                "转身冷却",
                if org.turn_cooldown > 0 {
                    format!("{} tick", org.turn_cooldown)
                } else {
                    "就绪".to_string()
                },
            );
            dim(
                ui,
                "生命 / 年龄",
                format!(
                    "{:.0} / {:.0}  ·  {} / {:.0}",
                    org.max_health(cfg) - org.damage,
                    org.max_health(cfg),
                    org.age,
                    org.lifespan(cfg)
                ),
            );
        });
}

// ────────────────────────────── 神经网络 ──────────────────────────────

/// 画图并返回一行提示文字（悬停时显示具体数值）
fn draw_brain(ui: &mut egui::Ui, org: &Organism, show_all: bool) -> String {
    let w = ui.available_width().min(420.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 196.0), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, CornerRadius::same(3), Color32::from_rgb(12, 16, 24));
    p.rect_stroke(
        rect,
        CornerRadius::same(3),
        Stroke::new(1.0_f32, BORDER),
        egui::StrokeKind::Inside,
    );

    let brain = &org.brain;
    let (in_n, hid_n, out_n) = (brain.in_size, brain.hidden_size, brain.out_size);

    // ── 几何：输入 5 块（4 个视觉方向 + 1 组状态）、隐藏 12、输出 5 ──
    let mut inputs: Vec<Pos2> = Vec::with_capacity(in_n);
    let left = rect.left() + 14.0;
    // 每块竖排：视觉块 3×3，状态块 2×2；块间留 8
    let step = 7.0;
    let mut y = rect.center().y - 46.0;
    for _ in 0..VISION_DIRS {
        for s in 0..VISUAL_SLOTS {
            let col = (s % 3) as f32;
            let row = (s / 3) as f32;
            inputs.push(egui::pos2(
                left + col * step,
                y + row * step,
            ));
        }
        // 组间留大一点：5 个输入块要能一眼看出是「4 个方向 + 1 组状态」
        y += 3.0 * step + 11.0;
    }
    // 状态组（36..39）2×2
    for s in 0..(SENSE_DIM - STATE_BASE) {
        let col = (s % 2) as f32;
        let row = (s / 2) as f32;
        inputs.push(egui::pos2(left + col * step, y + row * step));
    }
    while inputs.len() < in_n {
        inputs.push(egui::pos2(left, y));
    }

    let hid_x = rect.left() + 232.0;
    let mut hidden: Vec<Pos2> = Vec::with_capacity(hid_n);
    for j in 0..hid_n {
        let t = if hid_n > 1 {
            j as f32 / (hid_n - 1) as f32
        } else {
            0.5
        };
        hidden.push(egui::pos2(
            hid_x,
            rect.top() + 24.0 + t * (rect.height() - 38.0),
        ));
    }

    let out_x = rect.left() + 344.0;
    let mut outputs: Vec<Pos2> = Vec::with_capacity(out_n);
    for k in 0..out_n {
        let t = if out_n > 1 {
            k as f32 / (out_n - 1) as f32
        } else {
            0.5
        };
        outputs.push(egui::pos2(
            out_x,
            rect.top() + 26.0 + t * (rect.height() - 48.0),
        ));
    }

    // ── 连线 ──
    let max_w = brain.strongest_weight().max(1e-6);
    let thresh = if show_all { 0.0 } else { max_w * 0.45 };
    let pos_col = Color32::from_rgb(56, 189, 248);
    let neg_col = Color32::from_rgb(248, 113, 113);

    // 输入 → 隐藏。激活用 |输入| 加权：亮线 = 这一路真的在推
    for j in 0..hid_n {
        for i in 0..in_n.min(inputs.len()) {
            let wv = brain.w1[j * in_n + i];
            let a = wv.abs();
            if a < thresh {
                continue;
            }
            let act = org.sensors.get(i).copied().unwrap_or(0.0).abs();
            let strength = (a / max_w) * (0.35 + 0.65 * act.min(1.0));
            let alpha = (40.0 + 190.0 * strength).min(225.0) as u8;
            let col = if wv >= 0.0 {
                Color32::from_rgba_unmultiplied(pos_col.r(), pos_col.g(), pos_col.b(), alpha)
            } else {
                Color32::from_rgba_unmultiplied(neg_col.r(), neg_col.g(), neg_col.b(), alpha)
            };
            p.line_segment(
                [inputs[i], hidden[j]],
                Stroke::new(0.4 + 1.3 * strength, col),
            );
        }
    }
    // 隐藏 → 输出
    for j in 0..hid_n {
        for k in 0..out_n {
            let wv = brain.w2[j * out_n + k];
            let a = wv.abs();
            if a < thresh {
                continue;
            }
            let act = brain.hidden.get(j).copied().unwrap_or(0.0).abs();
            let strength = (a / max_w) * (0.35 + 0.65 * act.min(1.0));
            let alpha = (40.0 + 190.0 * strength).min(225.0) as u8;
            let col = if wv >= 0.0 {
                Color32::from_rgba_unmultiplied(pos_col.r(), pos_col.g(), pos_col.b(), alpha)
            } else {
                Color32::from_rgba_unmultiplied(neg_col.r(), neg_col.g(), neg_col.b(), alpha)
            };
            p.line_segment(
                [hidden[j], outputs[k]],
                Stroke::new(0.4 + 1.3 * strength, col),
            );
        }
    }

    // ── 节点 ──
    // 值 → 颜色：正值偏青、负值偏红，亮度随幅度。k 有下限，
    // 否则"接近 0"的节点在深底上完全看不见，会误以为没画。
    let val_col = |v: f32| {
        let t = v.abs().clamp(0.0, 1.0);
        let k = 0.25 + 0.75 * t;
        if v >= 0.0 {
            Color32::from_rgb(
                (40.0 + 150.0 * k) as u8,
                (120.0 + 135.0 * k) as u8,
                (170.0 + 85.0 * k) as u8,
            )
        } else {
            Color32::from_rgb(
                (150.0 + 105.0 * k) as u8,
                (60.0 + 50.0 * k) as u8,
                (60.0 + 50.0 * k) as u8,
            )
        }
    };

    for (i, pos) in inputs.iter().enumerate() {
        let v = org.sensors.get(i).copied().unwrap_or(0.0);
        p.circle_filled(*pos, 2.1, val_col(v));
    }
    for (j, pos) in hidden.iter().enumerate() {
        let v = brain.hidden.get(j).copied().unwrap_or(0.0);
        p.circle_filled(*pos, 4.0, val_col(v));
        p.circle_stroke(*pos, 4.0, Stroke::new(0.7_f32, Color32::from_rgb(64, 78, 96)));
    }
    for (k, pos) in outputs.iter().enumerate() {
        let v = brain.output.get(k).copied().unwrap_or(0.0);
        // 输出是线性的，用 tanh 压一下再映射颜色
        let t = (v * 0.5).tanh();
        p.circle_filled(*pos, 4.4, val_col(t));
        p.circle_stroke(*pos, 4.4, Stroke::new(0.8_f32, Color32::from_rgb(70, 84, 104)));
        let name = if k + 1 == out_n {
            "转身"
        } else {
            DIR_NAMES[k & 3]
        };
        // 决策就是取 5 个输出的 argmax（最后一个是转身），这里如实标出来
        let best = brain
            .output
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(m, _)| m)
            .unwrap_or(usize::MAX);
        let hot = k == best;
        p.text(
            *pos + egui::vec2(9.0, 0.0),
            Align2::LEFT_CENTER,
            format!("{} {:.2}", name, v),
            FontId::proportional(9.0),
            if hot { ACCENT } else { TEXT_DIM },
        );
    }

    // 层标题
    p.text(
        egui::pos2(left, rect.top() + 4.0),
        Align2::LEFT_TOP,
        format!("输入 {}", in_n),
        FontId::proportional(9.0),
        TEXT_DIM,
    );
    p.text(
        egui::pos2(hid_x, rect.top() + 4.0),
        Align2::CENTER_TOP,
        format!("隐藏 {}", hid_n),
        FontId::proportional(9.0),
        TEXT_DIM,
    );
    p.text(
        egui::pos2(out_x, rect.top() + 4.0),
        Align2::CENTER_TOP,
        format!("输出 {}", out_n),
        FontId::proportional(9.0),
        TEXT_DIM,
    );

    // ── 悬停：找最近的节点，把数值写在下面的提示行里 ──
    let mut hint = if show_all {
        "显示全部 540 条权重（青=正权重，红=负权重；越亮=这一路的输入越活跃）".to_string()
    } else {
        format!(
            "只画强连接（|权重| ≥ {:.2}）。青=正，红=负；越亮=输入越活跃。悬停节点看数值",
            thresh
        )
    };

    if let Some(mouse) = resp.hover_pos() {
        let mut best: Option<(f32, String)> = None;
        let mut consider = |pos: Pos2, label: String| {
            let d = pos.distance(mouse);
            if d <= 7.0 && best.as_ref().map(|b| d < b.0).unwrap_or(true) {
                best = Some((d, label));
            }
        };
        for (i, pos) in inputs.iter().enumerate() {
            let v = org.sensors.get(i).copied().unwrap_or(0.0);
            consider(*pos, format!("输入 {} = {:.2}", input_label(i, org.rotation), v));
        }
        for (j, pos) in hidden.iter().enumerate() {
            let v = brain.hidden.get(j).copied().unwrap_or(0.0);
            consider(*pos, format!("隐藏 h{} = {:.2}", j, v));
        }
        for (k, pos) in outputs.iter().enumerate() {
            let v = brain.output.get(k).copied().unwrap_or(0.0);
            let name = if k + 1 == out_n {
                "转身".to_string()
            } else {
                format!("往{}走", DIR_NAMES[k & 3])
            };
            consider(*pos, format!("输出 {} = {:.2}", name, v));
        }
        if let Some((_, text)) = best {
            hint = text;
        }
    }

    hint
}

/// 40 维输入的语义名字（悬停时用）
fn input_label(i: usize, rotation: u8) -> String {
    if i < VISION_DIRS * VISUAL_SLOTS {
        let local = i / VISUAL_SLOTS;
        let slot = i % VISUAL_SLOTS;
        // 传感器按**局部方向**取数，实际看的是 (rotation + local) 那一侧
        let world_dir = ((rotation as usize + local) & 3) as usize;
        if slot < VISIBLE_CELL_TYPES.len() {
            let ty = VISIBLE_CELL_TYPES[slot];
            let name = cell_type::def(ty).map(|d| d.name).unwrap_or("?");
            format!("看{}侧·{}", DIR_NAMES[world_dir], name)
        } else {
            format!("看{}侧·距离", DIR_NAMES[world_dir])
        }
    } else {
        match i - STATE_BASE {
            0 => "能量比例".to_string(),
            1 => "年龄比例".to_string(),
            2 => "会移动".to_string(),
            _ => "偏置".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_input_dim_has_a_label() {
        for rotation in 0..4u8 {
            for i in 0..SENSE_DIM {
                let s = input_label(i, rotation);
                assert!(!s.is_empty(), "第 {} 维没有名字", i);
                assert!(!s.contains('?'), "第 {} 维的名字里出现了占位符：{}", i, s);
            }
        }
    }

    #[test]
    fn vision_labels_follow_rotation() {
        // 局部方向 0 在 rotation=1 时应该报「右侧」
        assert!(input_label(0, 0).contains("上"));
        assert!(input_label(0, 1).contains("右"));
        assert!(input_label(0, 2).contains("下"));
        assert!(input_label(0, 3).contains("左"));
    }
}
