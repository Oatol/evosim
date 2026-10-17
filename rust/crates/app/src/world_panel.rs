//! 「世界」窗口：新建 / 保存 / 读取。
//!
//! ## 为什么要二次确认
//!
//! 新建和读取都会**整体丢弃当前世界**——跑了一晚上的演化，点错一下就没了。
//! 所以两个动作都走「先变成危险态、3 秒内再点一次」的两段式：
//! 第一次点只是把按钮变成红字，手滑不会造成损失。
//!
//! ## 为什么不在这里直接换世界
//!
//! 换世界要重建 GPU 纹理、重置相机、清空检视器/曲线/物种树的状态。
//! 这些都属于主程序（`main.rs::install_world`），窗口只负责**收集意图**
//! （[`WorldActions`]），这样"谁拥有 World"始终只有一处。

use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use egui::{Color32, RichText};

use evosim_core::save;
use evosim_core::world::World;

use crate::ui::{ACCENT, TEXT, TEXT_DIM};

/// 危险态保持多久（秒）
const CONFIRM_WINDOW: f64 = 3.0;

/// 新建世界的参数
#[derive(Clone, Debug)]
pub struct NewWorld {
    pub cols: usize,
    pub rows: usize,
    pub seed: u32,
    pub demo_terrain: bool,
    pub pop: usize,
}

#[derive(Default)]
pub struct WorldActions {
    pub create: Option<NewWorld>,
    pub load: Option<PathBuf>,
}

/// 待确认的危险动作
#[derive(Clone, PartialEq, Eq)]
enum Pending {
    New,
    Load(PathBuf),
}

pub struct WorldPanel {
    pub open: bool,

    // ── 新建 ──
    cols: i32,
    rows: i32,
    seed_text: String,
    demo_terrain: bool,
    pop: i32,

    // ── 保存 ──
    save_name: String,

    // ── 读取 ──
    saves: Vec<save::SaveInfo>,
    scanned: bool,

    /// 待确认的动作 + 过期时刻（`ctx.input().time`，秒）
    pending: Option<(Pending, f64)>,
    /// 上一次操作的结果，显示在窗口底部
    status: String,
    status_err: bool,
}

impl Default for WorldPanel {
    fn default() -> Self {
        WorldPanel {
            open: false,
            cols: 200,
            rows: 150,
            seed_text: String::new(),
            demo_terrain: true,
            pop: 8,
            save_name: String::new(),
            saves: Vec::new(),
            scanned: false,
            pending: None,
            status: String::new(),
            status_err: false,
        }
    }
}

impl WorldPanel {
    pub fn refresh(&mut self) {
        self.saves = save::list_saves();
        self.scanned = true;
    }

    pub fn set_status(&mut self, msg: impl Into<String>, err: bool) {
        self.status = msg.into();
        self.status_err = err;
    }

    /// 当前世界是否"有东西可丢"——刚启动还没跑过 tick 时不必吓唬人
    fn has_progress(world: &World) -> bool {
        world.tick > 0 || world.organisms.len() > 0
    }

    /// 危险动作是否已处于确认态（且没过期）。
    ///
    /// 这里**只读不写**：过期的 `pending` 由 `draw` 开头统一清掉。
    /// 之所以不能顺手清，是因为绘制存档列表时正持着 `&self.saves` 的不可变借用。
    fn confirming(&self, now: f64, which: &Pending) -> bool {
        match &self.pending {
            Some((p, deadline)) => now <= *deadline && p == which,
            None => false,
        }
    }

    fn arm(&mut self, now: f64, which: Pending) {
        self.pending = Some((which, now + CONFIRM_WINDOW));
    }

    pub fn draw(&mut self, ctx: &egui::Context, world: &World) -> WorldActions {
        let mut act = WorldActions::default();
        if !self.open {
            return act;
        }

        // 第一次打开时扫一次存档目录（之后靠「刷新」和保存后自动刷新）
        if !self.scanned {
            self.refresh();
            if self.save_name.is_empty() {
                self.save_name = format!("world-{}", world.tick);
            }
        }

        let now = ctx.input(|i| i.time);
        // 危险态过期后要重绘一次，否则按钮停在红字上
        if let Some((_, deadline)) = &self.pending {
            if now > *deadline {
                self.pending = None;
                ctx.request_repaint();
            } else {
                ctx.request_repaint_after(Duration::from_millis(100));
            }
        }

        let mut open = self.open;
        egui::Window::new("世界")
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            // 必须同时钉住 min/max：`resizable(false)` 下 `default_width` 只是"建议"，
            // 窗口会缩到内容宽度，滑块和存档列表全被挤成一条
            .min_width(470.0)
            .max_width(470.0)
            .default_pos([260.0, 120.0])
            .show(ctx, |ui| {
                self.ui_current(ui, world);
                ui.add_space(6.0);

                self.ui_new(ui, world, now, &mut act);
                ui.separator();

                self.ui_save(ui, world);
                ui.separator();

                self.ui_load(ui, world, now, &mut act);

                if !self.status.is_empty() {
                    ui.add_space(4.0);
                    let color = if self.status_err {
                        Color32::from_rgb(248, 113, 113)
                    } else {
                        Color32::from_rgb(134, 239, 172)
                    };
                    ui.label(RichText::new(&self.status).color(color).size(11.0));
                }
            });
        self.open = open;

        act
    }

    // ────────────────────────── 当前世界 ──────────────────────────

    fn ui_current(&mut self, ui: &mut egui::Ui, world: &World) {
        let stats = world.snapshot_stats();
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("{} × {}", world.grid.cols, world.grid.rows))
                    .color(ACCENT)
                    .size(15.0)
                    .strong(),
            );
            ui.label(
                RichText::new(format!(
                    "tick {} · {} 只 · 食物 {} · 物种 {}",
                    world.tick,
                    stats.organisms,
                    stats.food,
                    world.species.size()
                ))
                .color(TEXT_DIM)
                .size(11.0),
            );
        });
    }

    // ────────────────────────── 新建 ──────────────────────────

    fn ui_new(&mut self, ui: &mut egui::Ui, world: &World, now: f64, act: &mut WorldActions) {
        ui.label(RichText::new("新建世界").color(TEXT).size(12.0).strong());
        ui.add_space(2.0);

        ui.spacing_mut().slider_width = 240.0;
        ui.add(egui::Slider::new(&mut self.cols, 20..=600).text("宽"));
        ui.add(egui::Slider::new(&mut self.rows, 20..=450).text("高"));

        ui.horizontal(|ui| {
            ui.label(RichText::new("预设").color(TEXT_DIM).size(11.0));
            for (label, c, r) in [
                ("小 120×90", 120, 90),
                ("标准 200×150", 200, 150),
                ("大 400×300", 400, 300),
            ] {
                if ui.small_button(label).clicked() {
                    self.cols = c;
                    self.rows = r;
                }
            }
        });

        ui.horizontal(|ui| {
            ui.label(RichText::new("种子").color(TEXT_DIM).size(11.0));
            ui.add(
                egui::TextEdit::singleline(&mut self.seed_text)
                    .desired_width(110.0)
                    .hint_text("留空 = 随机"),
            );
            ui.label(
                RichText::new("同一个种子会生成同一个世界")
                    .color(TEXT_DIM)
                    .size(10.0),
            );
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.demo_terrain, "铺演示地形");
            ui.add_space(10.0);
            ui.label(RichText::new("初始生命").color(TEXT_DIM).size(11.0));
            ui.add(egui::DragValue::new(&mut self.pop).range(0..=200).speed(1.0));
            ui.label(RichText::new("只").color(TEXT_DIM).size(11.0));
        });

        // ── 丢弃提醒：常驻红字，不是只弹一次 ──
        ui.add_space(2.0);
        ui.label(
            RichText::new("注意：新建会丢弃当前世界，未保存的进度无法恢复")
                .color(Color32::from_rgb(248, 113, 113))
                .size(11.0),
        );

        let armed = self.confirming(now, &Pending::New);
        let label = if armed {
            "确认丢弃当前世界"
        } else {
            "新建世界"
        };
        let btn = if armed {
            egui::Button::new(RichText::new(label).color(Color32::WHITE).strong())
                .fill(Color32::from_rgb(153, 27, 27))
        } else {
            egui::Button::new(label)
        };

        if ui.add_sized([150.0, 22.0], btn).clicked() {
            // 没有进度就直接执行，有进度才要二次确认
            if armed || !Self::has_progress(world) {
                self.pending = None;
                act.create = Some(NewWorld {
                    cols: self.cols.max(20) as usize,
                    rows: self.rows.max(20) as usize,
                    seed: parse_seed(&self.seed_text),
                    demo_terrain: self.demo_terrain,
                    pop: self.pop.max(0) as usize,
                });
                self.scanned = false; // 换了世界，下次打开重新扫描
            } else {
                self.arm(now, Pending::New);
            }
        }
    }

    // ────────────────────────── 保存 ──────────────────────────

    fn ui_save(&mut self, ui: &mut egui::Ui, world: &World) {
        ui.label(RichText::new("保存").color(TEXT).size(12.0).strong());
        ui.add_space(2.0);

        ui.horizontal(|ui| {
            ui.label(RichText::new("文件名").color(TEXT_DIM).size(11.0));
            ui.add(
                egui::TextEdit::singleline(&mut self.save_name)
                    .desired_width(200.0)
                    .hint_text("world-1234"),
            );
            ui.label(RichText::new(".evosim").color(TEXT_DIM).size(11.0));
            if ui.add_sized([54.0, 22.0], egui::Button::new("保存")).clicked() {
                let name = if self.save_name.trim().is_empty() {
                    format!("world-{}", world.tick)
                } else {
                    self.save_name.clone()
                };
                match save::save_to(world, &name) {
                    Ok(path) => {
                        let size = std::fs::metadata(&path)
                            .map(|m| m.len())
                            .unwrap_or(0);
                        self.set_status(
                            format!("已保存 {}（{:.1} MB）", short_path(&path), size as f64 / 1e6),
                            false,
                        );
                        self.refresh();
                    }
                    Err(e) => self.set_status(format!("保存失败：{}", e), true),
                }
            }
        });
        ui.label(
            RichText::new(format!("存档目录：{}", save::saves_dir().display()))
                .color(TEXT_DIM)
                .size(10.0),
        );
    }

    // ────────────────────────── 读取 ──────────────────────────

    fn ui_load(&mut self, ui: &mut egui::Ui, world: &World, now: f64, act: &mut WorldActions) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("读取").color(TEXT).size(12.0).strong());
            if ui.small_button("刷新").clicked() {
                self.refresh();
            }
            ui.label(
                RichText::new(format!("{} 个存档", self.saves.len()))
                    .color(TEXT_DIM)
                    .size(10.0),
            );
        });
        ui.add_space(2.0);

        if self.saves.is_empty() {
            ui.label(
                RichText::new("还没有存档——先跑一会儿，然后在上面保存")
                    .color(TEXT_DIM)
                    .size(11.0),
            );
            return;
        }

        // 列表：每行 = 名字 + 摘要 + 读取按钮
        let mut pick: Option<PathBuf> = None;
        egui::ScrollArea::vertical()
            .max_height(150.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for info in &self.saves {
                    ui.horizontal(|ui| {
                        let armed = self.confirming(now, &Pending::Load(info.path.clone()));
                        let btn_label = if armed { "确认丢弃" } else { "读取" };
                        let btn = if armed {
                            egui::Button::new(
                                RichText::new(btn_label).color(Color32::WHITE).strong(),
                            )
                            .fill(Color32::from_rgb(153, 27, 27))
                        } else {
                            egui::Button::new(btn_label)
                        };
                        if ui.add_sized([62.0, 20.0], btn).clicked() {
                            pick = Some(info.path.clone());
                        }
                        ui.vertical(|ui| {
                            ui.label(RichText::new(&info.name).color(TEXT).size(11.0));
                            ui.label(
                                RichText::new(format!(
                                    "{}×{} · tick {} · {} 只 · {:.1} MB · {}",
                                    info.cols,
                                    info.rows,
                                    info.tick,
                                    info.organisms,
                                    info.bytes as f64 / 1e6,
                                    ago(info.modified)
                                ))
                                .color(TEXT_DIM)
                                .size(10.0),
                            );
                        });
                    });
                }
            });

        if let Some(path) = pick {
            let armed = self.confirming(now, &Pending::Load(path.clone()));
            if armed || !Self::has_progress(world) {
                self.pending = None;
                act.load = Some(path);
            } else {
                self.arm(now, Pending::Load(path));
            }
        }
    }
}

// ────────────────────────── 小工具 ──────────────────────────

fn parse_seed(text: &str) -> u32 {
    let t = text.trim();
    if t.is_empty() {
        // 用时钟低位当随机种子（不引入 rand 依赖）
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(1);
        return nanos.max(1);
    }
    t.parse::<u32>().unwrap_or_else(|_| {
        // 允许随便敲字：按字节哈希成种子
        let mut h: u32 = 2166136261;
        for b in t.bytes() {
            h ^= b as u32;
            h = h.wrapping_mul(16777619);
        }
        h.max(1)
    })
}

fn short_path(p: &std::path::Path) -> String {
    p.file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("?")
        .to_string()
}

/// 「3 分钟前」这种相对时间，比时间戳好读
fn ago(t: Option<SystemTime>) -> String {
    let Some(t) = t else {
        return "?".into();
    };
    let Ok(d) = SystemTime::now().duration_since(t) else {
        return "刚刚".into();
    };
    let s = d.as_secs();
    if s < 60 {
        "刚刚".into()
    } else if s < 3600 {
        format!("{} 分钟前", s / 60)
    } else if s < 86400 {
        format!("{} 小时前", s / 3600)
    } else {
        format!("{} 天前", s / 86400)
    }
}
