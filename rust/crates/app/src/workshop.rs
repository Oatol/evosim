//! 物种工坊：手绘蓝图 + 生物库。
//!
//! 左边是细胞调色板，中间是 15×15 的蓝图画布（点格子放细胞、右键擦除），
//! 右边是名字与三个动作（手动放置 / 放到视野中心 / 存入生物库），下面是生物库列表。
//!
//! 库存在 exe 同目录的 `creatures.json`（2.0 的存档格式会复用它）。

use std::collections::HashMap;
use std::path::PathBuf;

use egui::{Align2, Color32, CornerRadius, FontId, Sense, Stroke, StrokeKind};

use evosim_core::blueprint::Blueprint;
use evosim_core::cell_type::{self, ct, is_directional, LIVING_CELL_TYPES};
use evosim_core::world::World;
use evosim_render::colors::build_palette;

use crate::paint::ERASER;
use crate::ui::{ACCENT, BG, BORDER, TEXT, TEXT_DIM};

const RADIUS: i32 = 7; // 15 × 15
const CELL: f32 = 20.0;

pub struct Workshop {
    pub open: bool,
    pub cells: HashMap<(i32, i32), (u8, u8)>,
    pub brush: u8,
    pub dir: u8,
    pub name: String,
    pub library: Vec<Blueprint>,
    pub selected: Option<usize>,
    pub status: String,
    lib_path: PathBuf,
}

#[derive(Default)]
pub struct WorkshopActions {
    /// 手动放置：交给主程序装进"放置模式"
    pub place: Option<Blueprint>,
    /// 放到视野中心
    pub center: Option<Blueprint>,
    /// 从库里载入到画布
    pub loaded: bool,
}

impl Default for Workshop {
    fn default() -> Self {
        let mut w = Workshop {
            open: false,
            cells: HashMap::new(),
            brush: ct::MOUTH,
            dir: 0,
            name: String::new(),
            library: Vec::new(),
            selected: None,
            status: String::new(),
            lib_path: PathBuf::from("creatures.json"),
        };
        w.load_library();
        w
    }
}

impl Workshop {
    pub fn load_library(&mut self) {
        if let Ok(text) = std::fs::read_to_string(&self.lib_path) {
            if let Ok(list) = serde_json::from_str::<Vec<Blueprint>>(&text) {
                self.library = list;
                self.status = format!("已载入 {} 个生物", self.library.len());
            }
        }
    }

    pub fn save_library(&mut self) {
        match serde_json::to_string_pretty(&self.library) {
            Ok(text) => match std::fs::write(&self.lib_path, text) {
                Ok(_) => self.status = format!("已保存到 {}", self.lib_path.display()),
                Err(e) => self.status = format!("保存失败：{}", e),
            },
            Err(e) => self.status = format!("序列化失败：{}", e),
        }
    }

    pub fn clear(&mut self) {
        self.cells.clear();
    }

    pub fn load_blueprint(&mut self, bp: &Blueprint) {
        self.cells.clear();
        for c in &bp.cells {
            self.cells.insert((c.dx, c.dy), (c.t, c.dir));
        }
        self.name = bp.name.clone();
    }

    pub fn to_blueprint(&self) -> Blueprint {
        let mut cells: Vec<_> = self.cells.iter().collect();
        // 排序让蓝图稳定（指纹与导出结果可复现）
        cells.sort_by_key(|((dx, dy), _)| (*dy, *dx));
        Blueprint {
            name: if self.name.trim().is_empty() {
                "未命名".to_string()
            } else {
                self.name.trim().to_string()
            },
            cells: cells
                .into_iter()
                .map(|((dx, dy), (t, dir))| evosim_core::blueprint::CellSpec {
                    t: *t,
                    dx: *dx,
                    dy: *dy,
                    dir: *dir,
                })
                .collect(),
            ..Default::default()
        }
    }

    pub fn draw(&mut self, ctx: &egui::Context, _world: &World) -> WorkshopActions {
        let mut act = WorkshopActions::default();
        if !self.open {
            return act;
        }

        egui::Window::new("物种工坊")
            .resizable(false)
            .collapsible(false)
            .default_pos([300.0, 60.0])
            .default_size([640.0, 520.0])
            .show(ctx, |ui| {
                ui.horizontal_top(|ui| {
                    // ── 左：细胞调色板 ──
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("细胞类型").color(TEXT_DIM));
                        let palette = build_palette();
                        let mut ids: Vec<u8> = LIVING_CELL_TYPES.to_vec();
                        ids.push(ERASER);
                        for chunk in ids.chunks(2) {
                            ui.horizontal(|ui| {
                                for &id in chunk {
                                    self.cell_tile(ui, &palette, id);
                                }
                            });
                        }
                        if is_directional(self.brush) {
                            ui.add_space(2.0);
                            ui.label(egui::RichText::new("眼睛朝向").color(TEXT_DIM));
                            ui.horizontal(|ui| {
                                for (i, name) in ["上", "右", "下", "左"].iter().enumerate() {
                                    if ui
                                        .selectable_label(self.dir as usize == i, *name)
                                        .clicked()
                                    {
                                        self.dir = i as u8;
                                    }
                                }
                            });
                        }
                    });

                    ui.separator();

                    // ── 中：蓝图画布 ──
                    ui.vertical(|ui| {
                        let side = (RADIUS * 2 + 1) as f32 * CELL;
                        let (rect, resp) =
                            ui.allocate_exact_size(egui::vec2(side, side), Sense::click_and_drag());
                        self.paint_grid(ui, rect, &resp);

                        if resp.dragged() || resp.clicked() {
                            if let Some(pos) = resp.interact_pointer_pos() {
                                if let Some((dx, dy)) = self.cell_at(rect, pos) {
                                    if resp.secondary_clicked() || resp.dragged_by(egui::PointerButton::Secondary) {
                                        self.cells.remove(&(dx, dy));
                                    } else if self.brush == ERASER {
                                        self.cells.remove(&(dx, dy));
                                    } else if !self.cells.contains_key(&(dx, dy)) {
                                        let dir = if is_directional(self.brush) { self.dir } else { 0 };
                                        self.cells.insert((dx, dy), (self.brush, dir));
                                    }
                                }
                            }
                        }
                    });

                    ui.separator();

                    // ── 右：名字 + 动作 + 库 ──
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("名字").color(TEXT_DIM));
                        ui.add(egui::TextEdit::singleline(&mut self.name).desired_width(180.0));

                        ui.add_space(4.0);
                        let bp = self.to_blueprint();
                        ui.label(
                            egui::RichText::new(format!(
                                "{} 格 · {}",
                                bp.cell_count(),
                                bp.describe()
                            ))
                            .color(TEXT_DIM)
                            .size(11.0),
                        );

                        ui.add_space(4.0);
                        ui.horizontal(|ui| {
                            if ui
                                .button("手动放置")
                                .on_hover_text("收起工坊，在画布上点哪里就放在哪里（Esc 取消）")
                                .clicked()
                                && bp.cell_count() > 0
                            {
                                act.place = Some(bp.clone());
                                self.open = false;
                            }
                            if ui
                                .button("放到视野中心")
                                .on_hover_text("直接投放到当前视野中心")
                                .clicked()
                                && bp.cell_count() > 0
                            {
                                act.center = Some(bp.clone());
                            }
                        });
                        ui.horizontal(|ui| {
                            if ui.button("存入生物库").clicked() && bp.cell_count() > 0 {
                                self.library.push(bp.clone());
                                self.selected = Some(self.library.len() - 1);
                                self.save_library();
                            }
                            if ui.button("清空").clicked() {
                                self.clear();
                            }
                        });

                        ui.separator();
                        ui.label(egui::RichText::new("生物库").color(TEXT_DIM));

                        let mut to_remove: Option<usize> = None;
                        let mut to_load: Option<usize> = None;
                        let mut to_place: Option<usize> = None;
                        let mut to_center: Option<usize> = None;

                        egui::ScrollArea::vertical()
                            .max_height(220.0)
                            .auto_shrink([false, true])
                            .show(ui, |ui| {
                                if self.library.is_empty() {
                                    ui.label(
                                        egui::RichText::new("（空：用「存入生物库」添加）")
                                            .color(TEXT_DIM)
                                            .size(11.0),
                                    );
                                }
                                for (i, item) in self.library.iter().enumerate() {
                                    ui.horizontal(|ui| {
                                        let label = format!("{}（{} 格）", item.name, item.cell_count());
                                        if ui
                                            .selectable_label(self.selected == Some(i), label)
                                            .on_hover_text(item.describe())
                                            .clicked()
                                        {
                                            self.selected = Some(i);
                                        }
                                        if ui.small_button("载入").clicked() {
                                            to_load = Some(i);
                                        }
                                        if ui.small_button("放置").clicked() {
                                            to_place = Some(i);
                                        }
                                        if ui.small_button("居中").clicked() {
                                            to_center = Some(i);
                                        }
                                        if ui.small_button("删").clicked() {
                                            to_remove = Some(i);
                                        }
                                    });
                                }
                            });

                        if let Some(i) = to_load {
                            let bp = self.library[i].clone();
                            self.load_blueprint(&bp);
                            act.loaded = true;
                        }
                        if let Some(i) = to_place {
                            act.place = Some(self.library[i].clone());
                            self.open = false;
                        }
                        if let Some(i) = to_center {
                            act.center = Some(self.library[i].clone());
                        }
                        if let Some(i) = to_remove {
                            self.library.remove(i);
                            self.selected = None;
                            self.save_library();
                        }

                        if !self.status.is_empty() {
                            ui.add_space(2.0);
                            ui.label(
                                egui::RichText::new(&self.status).color(TEXT_DIM).size(10.0),
                            );
                        }
                    });
                });
            });

        act
    }

    /// 一个细胞类型方块（自绘）
    fn cell_tile(&mut self, ui: &mut egui::Ui, palette: &[[u8; 4]], id: u8) {
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(84.0, 22.0), Sense::click());
        let selected = self.brush == id;
        let color = if id == ERASER {
            Color32::from_rgb(203, 213, 225)
        } else {
            let p = palette[(id as usize).min(255)];
            Color32::from_rgb(p[0], p[1], p[2])
        };
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::ZERO, Color32::from_rgb(26, 33, 45));
        let swatch = egui::Rect::from_min_size(rect.min + egui::vec2(3.0, 3.0), egui::vec2(15.0, 15.0));
        painter.rect_filled(swatch, CornerRadius::ZERO, color);
        let name = if id == ERASER {
            "擦除"
        } else {
            cell_type::label(id)
        };
        painter.text(
            rect.min + egui::vec2(23.0, rect.height() * 0.5),
            Align2::LEFT_CENTER,
            name,
            FontId::proportional(11.0),
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
            "擦掉这一格的细胞".to_string()
        } else {
            cell_type::def(id).map(|d| d.desc.to_string()).unwrap_or_default()
        };
        if resp.on_hover_text(desc).clicked() {
            self.brush = id;
        }
    }

    /// 画 15×15 网格
    fn paint_grid(&self, ui: &egui::Ui, rect: egui::Rect, _resp: &egui::Response) {
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::ZERO, BG);
        let palette = build_palette();
        let n = RADIUS * 2 + 1;

        for gy in 0..n {
            for gx in 0..n {
                let dx = gx - RADIUS;
                let dy = gy - RADIUS;
                let cell_rect = egui::Rect::from_min_size(
                    rect.min + egui::vec2(gx as f32 * CELL, gy as f32 * CELL),
                    egui::vec2(CELL, CELL),
                );
                // 网格线
                painter.rect_stroke(
                    cell_rect,
                    CornerRadius::ZERO,
                    Stroke::new(1.0, Color32::from_rgb(30, 38, 50)),
                    StrokeKind::Inside,
                );
                if let Some((t, dir)) = self.cells.get(&(dx, dy)) {
                    let p = palette[(*t as usize).min(255)];
                    painter.rect_filled(
                        cell_rect.shrink(1.0),
                        CornerRadius::ZERO,
                        Color32::from_rgb(p[0], p[1], p[2]),
                    );
                    if is_directional(*t) {
                        // 眼睛：画一条朝向细缝
                        let c = cell_rect.center();
                        let v = match dir & 3 {
                            0 => egui::vec2(0.0, -1.0),
                            1 => egui::vec2(1.0, 0.0),
                            2 => egui::vec2(0.0, 1.0),
                            _ => egui::vec2(-1.0, 0.0),
                        };
                        painter.line_segment(
                            [c, c + v * (CELL * 0.4)],
                            Stroke::new(2.0, Color32::from_rgb(20, 20, 30)),
                        );
                    }
                }
            }
        }

        // 枢轴 (0,0)：画个十字，提醒"这里是旋转中心，不可删"
        let pivot = egui::Rect::from_min_size(
            rect.min + egui::vec2(RADIUS as f32 * CELL, RADIUS as f32 * CELL),
            egui::vec2(CELL, CELL),
        );
        painter.rect_stroke(
            pivot,
            CornerRadius::ZERO,
            Stroke::new(1.0, ACCENT),
            StrokeKind::Inside,
        );
    }

    fn cell_at(&self, rect: egui::Rect, pos: egui::Pos2) -> Option<(i32, i32)> {
        if !rect.contains(pos) {
            return None;
        }
        let gx = ((pos.x - rect.left()) / CELL).floor() as i32;
        let gy = ((pos.y - rect.top()) / CELL).floor() as i32;
        let dx = gx - RADIUS;
        let dy = gy - RADIUS;
        if dx.abs() > RADIUS || dy.abs() > RADIUS {
            return None;
        }
        Some((dx, dy))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use evosim_core::blueprint::CellSpec;

    #[test]
    fn to_blueprint_is_stable() {
        let mut w = Workshop::default();
        w.cells.clear();
        w.cells.insert((1, 1), (ct::PRODUCER, 0));
        w.cells.insert((0, 0), (ct::MOUTH, 0));
        let a = w.to_blueprint();
        let b = w.to_blueprint();
        assert_eq!(a.cell_count(), 2);
        assert_eq!(a.fingerprint(), b.fingerprint(), "两次导出应得到同一指纹");
    }

    #[test]
    fn load_blueprint_roundtrip_keeps_eye_dir() {
        let mut w = Workshop::default();
        let bp = Blueprint {
            name: "眼".into(),
            cells: vec![CellSpec {
                t: ct::EYE,
                dx: 0,
                dy: -1,
                dir: 2,
            }],
            ..Default::default()
        };
        w.load_blueprint(&bp);
        assert_eq!(w.cells.len(), 1);
        assert_eq!(w.cells.get(&(0, -1)), Some(&(ct::EYE, 2)), "眼睛朝向要保住");
        assert_eq!(w.to_blueprint().fingerprint(), bp.fingerprint());
    }

    #[test]
    fn workshop_draws_without_panic() {
        let ctx = egui::Context::default();
        crate::ui::configure(&ctx);

        let mut w = Workshop::default();
        w.open = true;
        w.cells.clear();
        w.cells.insert((0, 0), (ct::MOUTH, 0));
        w.cells.insert((1, 1), (ct::PRODUCER, 0));
        w.cells.insert((0, -1), (ct::EYE, 1));

        let world = World::new(
            evosim_core::config::Config::default(),
            evosim_core::rng::Rng::new(1),
        );

        let mut input = egui::RawInput::default();
        input.screen_rect = Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1600.0, 950.0),
        ));
        let out = ctx.run(input, |ctx| {
            let _ = w.draw(ctx, &world);
        });
        assert!(!out.shapes.is_empty(), "工坊没有产出任何图形");
    }

    #[test]
    fn library_persists_to_json() {
        let mut w = Workshop::default();
        w.lib_path = std::env::temp_dir().join(format!(
            "evosim-creatures-test-{}.json",
            std::process::id()
        ));
        w.library.clear();
        w.library.push(Blueprint {
            name: "持久".into(),
            cells: vec![CellSpec {
                t: ct::MOUTH,
                dx: 0,
                dy: 0,
                dir: 0,
            }],
            ..Default::default()
        });
        w.save_library();

        let mut w2 = Workshop::default();
        w2.lib_path = w.lib_path.clone();
        w2.library.clear();
        w2.load_library();
        assert_eq!(w2.library.len(), 1);
        assert_eq!(w2.library[0].name, "持久");
        let _ = std::fs::remove_file(&w.lib_path);
    }
}
