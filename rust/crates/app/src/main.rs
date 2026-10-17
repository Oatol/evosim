//! evosim 2.0 —— 原生窗口（M2 渲染 + M3 界面）。
//!
//! 用法：
//!   evosim-app                                   打开窗口
//!   evosim-app --screenshot out.png --ticks 500  离屏渲染一张 PNG（不需要窗口）
//!   evosim-app --bench --seconds 4 --speed 50    无窗口稳态压测
//!   evosim-app --load saves/x.evosim             直接打开一份存档
//!
//! 快捷键：空格 暂停/继续 · S 单步 · [ ] 调速 · Tab 类型/物种着色 · F 适应窗口 · Esc 退出
//! 鼠标：左键落笔（画笔工具）· 滚轮以光标为锚缩放 · 右键/中键拖动平移

mod paint;
mod panels;
mod chart;
mod inspector;
mod ui;
mod workshop;
mod world_panel;

use std::sync::Arc;
use std::time::Instant;

use evosim_core::cell_type::{self, ct};
use evosim_core::config::Config;
use evosim_core::factory::{seed_origin_of_life, spawn_population};
use evosim_core::rng::Rng;
use evosim_core::world::World;
use evosim_core::worldgen::generate_demo_world;
use evosim_render::{Camera, GridRenderer};

use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

const SPEEDS: [f32; 4] = [1.0, 5.0, 10.0, 50.0];
const MIN_ZOOM: f32 = 0.5;
const MAX_ZOOM: f32 = 64.0;

fn arg_value(key: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == key)
        .and_then(|i| args.get(i + 1).cloned())
}

fn args_have(key: &str) -> bool {
    std::env::args().any(|a| a == key)
}

fn build_world(seed: u32, pop: usize) -> World {
    let mut world = World::new(Config::default(), Rng::new(seed));
    generate_demo_world(&mut world, 1.0, 1.0);
    seed_origin_of_life(&mut world);
    spawn_population(&mut world, pop, 3);
    world
}

fn main() {
    evosim_core::init_threads();

    let seed: u32 = arg_value("--seed")
        .and_then(|v| v.parse().ok())
        .unwrap_or(20_261_015);
    let ticks: u64 = arg_value("--ticks")
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    let pop: usize = arg_value("--pop").and_then(|v| v.parse().ok()).unwrap_or(8);

    if args_have("--bench") {
        let seconds: f64 = arg_value("--seconds")
            .and_then(|v| v.parse().ok())
            .unwrap_or(3.0);
        let speed: f64 = arg_value("--speed")
            .and_then(|v| v.parse().ok())
            .unwrap_or(1.0);
        bench(seed, pop, seconds, speed);
        return;
    }

    if let Some(path) = arg_value("--screenshot") {
        let w: u32 = arg_value("--width")
            .and_then(|v| v.parse().ok())
            .unwrap_or(1280);
        let h: u32 = arg_value("--height")
            .and_then(|v| v.parse().ok())
            .unwrap_or(800);
        screenshot(&path, seed, pop, ticks, w, h);
        return;
    }

    let event_loop = EventLoop::new().expect("创建事件循环失败");
    event_loop.set_control_flow(ControlFlow::Poll);
    let mut app = App::new(seed, pop, ticks);
    event_loop.run_app(&mut app).expect("事件循环异常退出");
}

fn screenshot(path: &str, seed: u32, pop: usize, ticks: u64, width: u32, height: u32) {
    let mut world = match arg_value("--load") {
        Some(p) => match evosim_core::save::load_from(std::path::Path::new(&p)) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("读档失败：{}", e);
                std::process::exit(1);
            }
        },
        None => build_world(seed, pop),
    };
    let t0 = Instant::now();
    for _ in 0..ticks {
        world.step();
    }
    let sim_ms = t0.elapsed().as_secs_f64() * 1000.0;

    let (device, queue) = match evosim_render::create_headless_device() {
        Some(v) => v,
        None => {
            eprintln!("没有可用的 GPU 适配器");
            std::process::exit(1);
        }
    };

    let (cols, rows) = (world.grid.cols as u32, world.grid.rows as u32);
    let mut renderer = GridRenderer::new(device, queue, wgpu::TextureFormat::Rgba8Unorm, cols, rows);
    renderer.upload_types(&world);
    renderer.upload_species(&world);

    let mode = match arg_value("--mode").as_deref() {
        Some("species") => 1,
        Some("temp") => 2,
        Some("rad") => 3,
        _ => 0,
    };

    // 场滤镜截图：铺几摊岩浆 / 辐射带，跑一会儿场再拍，否则拍到的是全黑
    if mode >= 2 {
        for (c, r, w, h) in [(30, 30, 8, 5), (120, 90, 10, 6), (60, 120, 6, 4)] {
            for rr in r..r + h {
                for cc in c..c + w {
                    world.set_cell(cc, rr, ct::LAVA, -1);
                }
            }
        }
        for (c, r, w, h) in [(150, 40, 7, 5), (90, 130, 6, 4)] {
            for rr in r..r + h {
                for cc in c..c + w {
                    world.set_cell(cc, rr, ct::RAD, -1);
                }
            }
        }
        for _ in 0..120 {
            world.update_fields();
        }
        let data = if mode == 2 {
            world.fields.temp.clone()
        } else {
            world.fields.rad.clone()
        };
        renderer.upload_field(&data);
    }
    renderer.set_mode(mode);

    let mut cam = Camera::new(4.0);
    cam.fit_grid(
        world.grid.cols,
        world.grid.rows,
        width as f32,
        height as f32,
        10.0,
    );

    let rgba = evosim_render::render_to_rgba(&mut renderer, width, height, &cam);
    evosim_render::save_png(path, width, height, &rgba).expect("写 PNG 失败");

    let stats = world.snapshot_stats();
    println!(
        "已写出 {}（{}×{}）· tick {} · 个体 {} · 食物 {} · 物种 {} · 模拟 {:.0} ms",
        path,
        width,
        height,
        world.tick,
        stats.organisms,
        stats.food,
        world.species.size(),
        sim_ms
    );
}

/// 无窗口压测：给定倍速跑 N 秒，报告实际 tps 与单 tick 耗时
fn bench(seed: u32, pop: usize, seconds: f64, speed: f64) {
    let mut world = build_world(seed, pop);
    let target = 60.0 * speed;

    for _ in 0..600 {
        world.step();
    }

    let t0 = Instant::now();
    let mut ticks: u64 = 0;
    while t0.elapsed().as_secs_f64() < seconds {
        world.step();
        ticks += 1;
    }
    let elapsed = t0.elapsed().as_secs_f64();
    let tps = ticks as f64 / elapsed;
    let stats = world.snapshot_stats();

    println!(
        "压测 {:.1} 秒（目标 {:.0} tps）· 实际 {:.0} tps（{:.0}%）· {:.3} ms/tick · 种群 {} · 食物 {} · 物种 {}",
        elapsed,
        target,
        tps,
        tps / target * 100.0,
        elapsed * 1000.0 / ticks as f64,
        stats.organisms,
        stats.food,
        world.species.size()
    );
}

struct App {
    seed: u32,
    pop: usize,
    pre_ticks: u64,

    world: Option<World>,
    renderer: Option<GridRenderer>,
    surface: Option<wgpu::Surface<'static>>,
    config: Option<wgpu::SurfaceConfiguration>,
    window: Option<Arc<Window>>,

    camera: Camera,
    paused: bool,
    speed_idx: usize,
    acc: f64,
    /// 环境场的独立累加器（墙钟 60 Hz，与 tick 解耦）
    field_acc: f64,
    last: Instant,

    frames: u32,
    fps_timer: Instant,
    fps: f32,
    tps: f32,
    ticks_this_second: u64,

    dragging: bool,
    last_cursor: (f64, f64),
    painting: bool,
    hover_cell: Option<(i32, i32)>,

    egui_ctx: egui::Context,
    egui_state: Option<egui_winit::State>,
    egui_renderer: Option<egui_wgpu::Renderer>,
    ui: ui::UiState,
    panels: panels::Panels,
    workshop: workshop::Workshop,
    world_panel: world_panel::WorldPanel,
    /// 「手动放置」装进来的蓝图：下一次左键就把它放在那里
    pending_place: Option<evosim_core::blueprint::Blueprint>,
    /// 放置模式的一次性反馈（成功 / 放不下），显示在提示条上
    place_msg: String,
    /// egui 是否吃掉了指针（吃掉就不落笔）
    ui_wants_pointer: bool,
}

impl App {
    fn new(seed: u32, pop: usize, pre_ticks: u64) -> Self {
        let egui_ctx = egui::Context::default();
        ui::configure(&egui_ctx);
        App {
            seed,
            pop,
            pre_ticks,
            world: None,
            renderer: None,
            surface: None,
            config: None,
            window: None,
            camera: Camera::new(4.0),
            paused: false,
            speed_idx: 0,
            acc: 0.0,
            field_acc: 0.0,
            last: Instant::now(),
            frames: 0,
            fps_timer: Instant::now(),
            fps: 0.0,
            tps: 0.0,
            ticks_this_second: 0,
            dragging: false,
            last_cursor: (0.0, 0.0),
            painting: false,
            hover_cell: None,
            egui_ctx,
            egui_state: None,
            egui_renderer: None,
            ui: ui::UiState::default(),
            panels: panels::Panels::default(),
            workshop: workshop::Workshop::default(),
            world_panel: world_panel::WorldPanel::default(),
            place_msg: String::new(),
            pending_place: None,
            ui_wants_pointer: false,
        }
    }

    fn tps_target(&self) -> f64 {
        60.0 * SPEEDS[self.speed_idx] as f64
    }

    fn init_gpu(&mut self, event_loop: &ActiveEventLoop) {
        let window = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("evosim 2.0")
                        .with_inner_size(winit::dpi::LogicalSize::new(1600.0, 950.0)),
                )
                .expect("创建窗口失败"),
        );

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::from_env().unwrap_or(wgpu::Backends::all()),
            ..Default::default()
        });
        let surface = instance
            .create_surface(window.clone())
            .expect("创建 surface 失败");

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .expect("没有可用的 GPU 适配器");

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("evosim"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults(),
                memory_hints: Default::default(),
            },
            None,
        ))
        .expect("创建设备失败");

        let caps = surface.get_capabilities(&adapter);
        // 优先非 sRGB：调色板里存的是 sRGB 字节，写进非 sRGB 目标才能原样呈现
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(caps.formats[0]);

        let size = window.inner_size();
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let mut world = match arg_value("--load") {
            Some(p) => match evosim_core::save::load_from(std::path::Path::new(&p)) {
                Ok(w) => {
                    println!("已读取存档 {}", p);
                    w
                }
                Err(e) => {
                    eprintln!("读档失败：{}（改用默认世界）", e);
                    build_world(self.seed, self.pop)
                }
            },
            None => build_world(self.seed, self.pop),
        };
        for _ in 0..self.pre_ticks {
            world.step();
        }
        let (cols, rows) = (world.grid.cols as u32, world.grid.rows as u32);

        let egui_state = egui_winit::State::new(
            self.egui_ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            Some(device.limits().max_texture_dimension_2d as usize),
        );
        let egui_renderer = egui_wgpu::Renderer::new(&device, format, None, 1, false);

        let mut renderer = GridRenderer::new(device, queue, format, cols, rows);
        renderer.upload_types(&world);
        renderer.upload_species(&world);

        self.camera.fit_grid(
            world.grid.cols,
            world.grid.rows,
            config.width as f32,
            config.height as f32,
            20.0,
        );

        println!(
            "GPU: {} · 格式 {:?} · 世界 {}×{}",
            adapter.get_info().name,
            format,
            cols,
            rows
        );
        println!(
            "空格 暂停 · S 单步 · [ ] 调速 · Tab 着色（类型/物种/温度/辐射）· F 适应 · G 工坊 · Esc 退出"
        );
        println!("左键：画笔落笔 / 平移拖动 / 检视选中 · 滚轮缩放 · 右键或中键随时平移");

        self.window = Some(window);
        self.surface = Some(surface);
        self.config = Some(config);
        self.renderer = Some(renderer);
        self.world = Some(world);
        self.egui_state = Some(egui_state);
        self.egui_renderer = Some(egui_renderer);
        self.last = Instant::now();
    }

    fn step_simulation(&mut self) {
        let target = self.tps_target();
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f64().min(0.25);
        self.last = now;

        // ── 环境场：按墙钟 60 Hz 推进，**与 tick 解耦** ──
        //
        // 50× 倍速下 tick 是 3000/s，如果场也跟着跑 3000 次扩散就是白烧 CPU；
        // 而且"岩浆烤热周围"是物理时间，1× 和 50× 下应该一样快。
        // 暂停时也继续跑——它目前只影响显示，不影响模拟。
        self.field_acc += dt;
        let step = 1.0 / 60.0;
        let mut n = 0;
        if let Some(world) = self.world.as_mut() {
            while self.field_acc >= step && n < 4 {
                world.update_fields();
                self.field_acc -= step;
                n += 1;
            }
        }
        if n > 0 {
            self.upload_field();
        }

        if self.paused {
            return;
        }

        self.acc += dt * target;
        let mut steps = 0u32;
        if let Some(world) = self.world.as_mut() {
            while self.acc >= 1.0 && steps < 400 {
                world.step();
                self.acc -= 1.0;
                steps += 1;
                self.ticks_this_second += 1;
            }
        }
        if steps == 400 {
            self.acc = 0.0;
        }
        if steps > 0 {
            self.upload_world();
        }
    }

    fn upload_world(&mut self) {
        if let (Some(renderer), Some(world)) = (self.renderer.as_mut(), self.world.as_ref()) {
            renderer.upload_types(world);
            if renderer.mode == 1 {
                renderer.upload_species(world);
            }
        }
    }

    /// 只在场滤镜模式下上传场（类型/物种着色时不需要）
    fn upload_field(&mut self) {
        if let (Some(renderer), Some(world)) = (self.renderer.as_mut(), self.world.as_ref()) {
            match renderer.mode {
                2 => renderer.upload_field(&world.fields.temp),
                3 => renderer.upload_field(&world.fields.rad),
                _ => {}
            }
        }
    }

    /// 悬停读数：光标所在格子的类型 / 归属个体 / 能量
    fn update_hover(&mut self) {
        let Some(world) = self.world.as_ref() else {
            return;
        };
        let Some((col, row)) = self.hover_cell else {
            self.ui.hover.clear();
            return;
        };
        let Some(idx) = world.grid.index(col, row) else {
            self.ui.hover.clear();
            return;
        };

        let ty = world.grid.types[idx];
        let owner = world.grid.owners[idx];
        let floor = world.grid.floor[idx];

        let mut text = format!("({}, {}) {}", col, row, cell_type::label(ty));
        if floor != ty && floor != ct::EMPTY {
            text.push_str(&format!("（地形：{}）", cell_type::label(floor)));
        }
        // 每格环境属性（M4）：温度 / 辐射，0–255 归一化后按百分比显示
        let temp = world.fields.temp_at(&world.grid, col, row);
        let rad = world.fields.rad_at(&world.grid, col, row);
        if temp > 0 || rad > 0 {
            text.push_str(&format!(
                " · 温度 {}% · 辐射 {}%",
                temp as u32 * 100 / 255,
                rad as u32 * 100 / 255
            ));
        }
        if owner >= 0 {
            if let Some(org) = world.organism(owner as u32) {
                let sp = org
                    .species_fingerprint
                    .as_deref()
                    .and_then(|fp| world.species.by_fingerprint(fp))
                    .map(|s| s.name.clone())
                    .unwrap_or_else(|| "?".to_string());
                text.push_str(&format!(
                    " · 个体 #{} · {} · 能量 {:.1}/{:.0} · 体型 {}",
                    org.id,
                    sp,
                    org.energy,
                    org.reproduce_at(&world.config),
                    org.cell_count()
                ));
            }
        }
        self.ui.hover = text;
    }

    fn apply_ui_actions(&mut self, act: ui::UiActions) {
        if act.toggle_pause {
            self.paused = !self.paused;
            self.last = Instant::now();
            self.acc = 0.0;
        }
        if act.step_once {
            self.paused = true;
            if let Some(world) = self.world.as_mut() {
                world.step();
            }
            self.upload_world();
        }
        if act.speed_delta != 0 {
            let i = self.speed_idx as i32 + act.speed_delta;
            self.speed_idx = i.clamp(0, SPEEDS.len() as i32 - 1) as usize;
        }
        if act.toggle_mode {
            // 四档循环：类型 → 物种 → 温度 → 辐射 → 类型
            if let Some(renderer) = self.renderer.as_mut() {
                renderer.set_mode((renderer.mode + 1) % 4);
            }
            if let Some(world) = self.world.as_ref() {
                if let Some(renderer) = self.renderer.as_mut() {
                    if renderer.mode == 1 {
                        renderer.upload_species(world);
                    }
                }
            }
            self.upload_field();
        }
        if act.fit {
            if let (Some(world), Some(config)) = (self.world.as_ref(), self.config.as_ref()) {
                self.camera.fit_grid(
                    world.grid.cols,
                    world.grid.rows,
                    config.width as f32,
                    config.height as f32,
                    20.0,
                );
            }
        }
        if let Some(id) = act.center_on {
            self.center_on_org(id);
        }
        if act.clear_selection {
            self.panels.selected_org = None;
            self.panels.follow = false;
            self.panels.pinned = false;
        }
        if act.toggle_workshop {
            self.workshop.open = !self.workshop.open;
        }
        if act.toggle_world {
            self.world_panel.open = !self.world_panel.open;
        }
    }

    fn view_size(&self) -> (f32, f32) {
        match self.config.as_ref() {
            Some(c) => (c.width as f32, c.height as f32),
            None => (1600.0, 950.0),
        }
    }

    /// 把相机对准某个个体
    fn center_on_org(&mut self, id: u32) {
        let Some(org) = self.world.as_ref().and_then(|w| w.organism(id)) else {
            return;
        };
        let (col, row) = (org.col as f32, org.row as f32);
        let (w, h) = self.view_size();
        self.camera.x = col - (w / self.camera.zoom) * 0.5;
        self.camera.y = row - (h / self.camera.zoom) * 0.5;
    }

    /// 跟随：每帧把相机钉在选中个体身上
    fn apply_follow(&mut self) {
        if !self.panels.follow {
            return;
        }
        let Some(id) = self.panels.selected_org else {
            return;
        };
        self.center_on_org(id);
    }

    /// 整体替换世界（新建 / 读档）。
    ///
    /// 三件事必须一起做，漏一件就会看到"旧世界的残影"：
    ///   1. **重建 `GridRenderer`** —— 纹理尺寸在构造时就固定了，换尺寸必须重来
    ///   2. **相机 fit** —— 否则新世界可能在视野之外
    ///   3. **清空引用旧世界的 UI 状态** —— 选中个体 / 曲线 / 待放置蓝图
    ///
    /// 这也是"世界窗口排在最前面做"的原因：所有面板都必须能正确处理
    /// "世界被换掉了"，先把这个入口立好，后面新加的面板天然会写对。
    fn install_world(&mut self, world: World) {
        let (cols, rows) = (world.grid.cols as u32, world.grid.rows as u32);
        let mode = self.renderer.as_ref().map(|r| r.mode).unwrap_or(0);

        let rebuilt = match (self.renderer.as_ref(), self.config.as_ref()) {
            (Some(old), Some(cfg)) => {
                // Device / Queue 是 Arc 包装的，clone 只是加引用计数
                let mut r = GridRenderer::new(
                    old.device.clone(),
                    old.queue.clone(),
                    cfg.format,
                    cols,
                    rows,
                );
                r.set_mode(mode);
                r.upload_types(&world);
                r.upload_species(&world);
                r.upload_field(&world.fields.temp);
                Some(r)
            }
            _ => None,
        };
        if let Some(r) = rebuilt {
            self.renderer = Some(r);
        }

        let (vw, vh) = self.view_size();
        self.camera
            .fit_grid(world.grid.cols, world.grid.rows, vw, vh, 20.0);

        self.panels.reset_world_state();
        self.pending_place = None;
        self.place_msg.clear();
        self.ui.highlights.clear();
        self.ui.place_preview = None;
        self.ui.hover.clear();
        self.hover_cell = None;

        // 换世界后先停住：让用户看清新的世界，也避免刚建好就窜过一屏 tick
        self.paused = true;
        self.acc = 0.0;
        self.last = Instant::now();

        self.world = Some(world);
        self.upload_field();
    }

    fn new_world(&mut self, spec: world_panel::NewWorld) {
        // 保留用户调过的参数，只换尺寸与种子
        let mut cfg = self
            .world
            .as_ref()
            .map(|w| w.config.clone())
            .unwrap_or_default();
        cfg.world.cols = spec.cols;
        cfg.world.rows = spec.rows;
        cfg.sim.seed = spec.seed;

        let mut world = World::new(cfg, Rng::new(spec.seed));
        if spec.demo_terrain {
            generate_demo_world(&mut world, 1.0, 1.0);
        }
        if spec.pop > 0 {
            seed_origin_of_life(&mut world);
            spawn_population(&mut world, spec.pop, 3);
        }
        let (c, r) = (world.grid.cols, world.grid.rows);
        self.install_world(world);
        self.world_panel
            .set_status(format!("已新建 {}×{} 的世界（种子 {}）", c, r, spec.seed), false);
    }

    fn load_world(&mut self, path: &std::path::Path) {
        match evosim_core::save::load_from(path) {
            Ok(world) => {
                let (tick, orgs) = (world.tick, world.organisms.len());
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("?")
                    .to_string();
                self.install_world(world);
                self.world_panel.set_status(
                    format!("已读取 {} · tick {} · {} 只", name, tick, orgs),
                    false,
                );
            }
            Err(e) => self.world_panel.set_status(format!("读取失败：{}", e), true),
        }
    }

    /// 把蓝图投放到当前视野中心
    fn spawn_blueprint_at_center(&mut self, bp: &evosim_core::blueprint::Blueprint) {
        let (w, h) = self.view_size();
        let (wx, wy) = self.camera.screen_to_world(w * 0.5, h * 0.5);
        let (col, row) = (wx.floor() as i32, wy.floor() as i32);
        let mut ok = false;
        if let Some(world) = self.world.as_mut() {
            ok = evosim_core::blueprint::spawn_from_blueprint(world, col, row, bp).is_some();
        }
        if ok {
            self.upload_world();
        }
    }

    /// 检视工具：点谁选谁（点空地取消选中）
    fn inspect_at_cursor(&mut self) {
        let Some((col, row)) = self.hover_cell else {
            return;
        };
        let Some(world) = self.world.as_ref() else {
            return;
        };
        let Some(idx) = world.grid.index(col, row) else {
            return;
        };
        let owner = world.grid.owners[idx];

        // 图钉：锁定当前检视目标——点空地或点别的个体都不会换目标。
        // （1.x 的图钉语义是"点画布不关闭浮窗"；2.0 的检视器是 egui 窗口，本来就不会
        //   被点关掉，所以把图钉落到"锁目标"上，功能才实在。）
        if self.panels.pinned && self.panels.selected_org.is_some() {
            return;
        }

        self.panels.selected_org = if owner >= 0 { Some(owner as u32) } else { None };
        if self.panels.selected_org.is_none() {
            self.panels.follow = false;
        }
    }

    fn paint_at_cursor(&mut self) {
        if self.ui.tool != ui::Tool::Paint || self.ui_wants_pointer {
            return;
        }
        let Some((col, row)) = self.hover_cell else {
            return;
        };

        if let Some(world) = self.world.as_mut() {
            paint::apply_brush(
                world,
                col,
                row,
                self.ui.brush,
                self.ui.brush_size,
                self.ui.brush_shape,
            );
        }
        self.upload_world();
    }

    /// 把待放置的蓝图放到光标处。
    ///
    /// **放成功后仍然留在放置模式**（1.x 的行为）：连点几下就能连放几个，
    /// 直到 Esc 退出。放不下（被占 / 越界）时给一条提示，蓝图不丢。
    fn place_pending_at_cursor(&mut self) {
        let Some((col, row)) = self.hover_cell else {
            return;
        };
        let Some(bp) = self.pending_place.clone() else {
            return;
        };

        let mut placed = false;
        if let Some(world) = self.world.as_mut() {
            placed = evosim_core::blueprint::spawn_from_blueprint(world, col, row, &bp).is_some();
        }

        if placed {
            self.upload_world();
            self.place_msg = format!("已放置「{}」于 ({}, {})", bp.name, col, row);
            self.pending_place = Some(bp); // 保持放置模式，可以接着放
        } else {
            self.place_msg = format!("({}, {}) 放不下——换个空一点的位置", col, row);
        }
    }

    fn draw(&mut self) {
        let Some(window) = self.window.clone() else {
            return;
        };

        // ── 1) egui：喂输入 → 建 UI → 拿到三角形 ──
        let raw = match self.egui_state.as_mut() {
            Some(s) => s.take_egui_input(&window),
            None => return,
        };

        let hud = ui::HudInfo {
            fps: self.fps,
            tps: self.tps,
            speed: SPEEDS[self.speed_idx],
            paused: self.paused,
            mode: self.renderer.as_ref().map(|r| r.mode).unwrap_or(0),
            workshop_open: self.workshop.open,
            world_open: self.world_panel.open,
        };

        let (vw, vh) = self.view_size();
        let view = ui::ViewInfo {
            cam_x: self.camera.x,
            cam_y: self.camera.y,
            zoom: self.camera.zoom,
            width: vw,
            height: vh,
        };

        // ── 放置模式：提示条 + 落点预览 ──
        let place_hint = self.pending_place.as_ref().map(|bp| {
            if self.place_msg.is_empty() {
                format!("放置模式：「{}」 · 左键放置 · Esc 取消", bp.name)
            } else {
                format!("{}　·　左键继续放 · Esc 取消", self.place_msg)
            }
        });
        self.ui.place_hint = place_hint;
        let preview = match (self.pending_place.as_ref(), self.hover_cell) {
            (Some(bp), Some((col, row))) => Some((
                col,
                row,
                bp.cells.iter().map(|c| (c.dx, c.dy)).collect::<Vec<_>>(),
            )),
            _ => None,
        };
        self.ui.place_preview = preview;

        let ctx = self.egui_ctx.clone();
        let mut actions = ui::UiActions::default();
        {
            let ui_state = &mut self.ui;
            let panels = &mut self.panels;
            let wshop = &mut self.workshop;
            let wpanel = &mut self.world_panel;
            let world = match self.world.as_mut() {
                Some(w) => w,
                None => return,
            };
            let mut shop_act = workshop::WorkshopActions::default();
            let mut world_act = world_panel::WorldActions::default();
            let full = ctx.run(raw, |ctx| {
                actions = ui::draw(ctx, ui_state, world, &hud, &view, panels);
                shop_act = wshop.draw(ctx, world);
                world_act = wpanel.draw(ctx, &*world);
            });
            if let Some(state) = self.egui_state.as_mut() {
                state.handle_platform_output(&window, full.platform_output);
            }
            self.apply_ui_actions(std::mem::take(&mut actions));
            if let Some(bp) = shop_act.place.take() {
                self.pending_place = Some(bp);
                self.place_msg.clear();
            }
            if let Some(bp) = shop_act.center.take() {
                self.spawn_blueprint_at_center(&bp);
            }
            // 世界窗口只收集意图，真正的替换在这里做（World 的所有权只有一处）
            if let Some(spec) = world_act.create.take() {
                self.new_world(spec);
            }
            if let Some(path) = world_act.load.take() {
                self.load_world(&path);
            }
            self.ui_wants_pointer = self.egui_ctx.wants_pointer_input();

            let tris = ctx.tessellate(full.shapes, full.pixels_per_point);
            self.render_frame(&tris, full.pixels_per_point, &full.textures_delta);
        }
    }

    /// 世界 + UI 一帧
    fn render_frame(
        &mut self,
        tris: &[egui::ClippedPrimitive],
        pixels_per_point: f32,
        textures: &egui::TexturesDelta,
    ) {
        let (Some(renderer), Some(surface), Some(config)) = (
            self.renderer.as_mut(),
            self.surface.as_ref(),
            self.config.as_ref(),
        ) else {
            return;
        };

        let frame = match surface.get_current_texture() {
            Ok(f) => f,
            Err(_) => return,
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        // 世界（清屏）
        renderer.draw(&view, config.width, config.height, &self.camera);

        // UI（叠在上面）
        if self.egui_renderer.is_some() {
            // 注意：`egui_wgpu::Renderer::render` 的签名把 render pass 的生命周期
            // 和渲染器的借用绑在一起，所以"更新资源（&mut）"与"录 pass（&self）"
            // 必须分成两段借用，否则 encoder 会被误判为活得不够久。
            if let Some(r) = self.egui_renderer.as_mut() {
                for (id, delta) in &textures.set {
                    r.update_texture(&renderer.device, &renderer.queue, *id, delta);
                }
            }
            let screen = egui_wgpu::ScreenDescriptor {
                size_in_pixels: [config.width, config.height],
                pixels_per_point,
            };
            let mut encoder =
                renderer
                    .device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("egui"),
                    });
            let cmds = match self.egui_renderer.as_mut() {
                Some(r) => r.update_buffers(
                    &renderer.device,
                    &renderer.queue,
                    &mut encoder,
                    tris,
                    &screen,
                ),
                None => Vec::new(),
            };
            if let Some(r) = self.egui_renderer.as_mut() {
                record_egui_pass(r, &mut encoder, &view, tris, &screen);
            }
            renderer.queue.submit(cmds.into_iter().chain([encoder.finish()]));
            if let Some(r) = self.egui_renderer.as_mut() {
                for id in &textures.free {
                    r.free_texture(id);
                }
            }
        }

        frame.present();

        self.frames += 1;
        let elapsed = self.fps_timer.elapsed().as_secs_f32();
        if elapsed >= 0.5 {
            self.fps = self.frames as f32 / elapsed;
            self.tps = self.ticks_this_second as f32 / elapsed;
            self.frames = 0;
            self.ticks_this_second = 0;
            self.fps_timer = Instant::now();
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            self.init_gpu(event_loop);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // ── Tab 必须在交给 egui 之前截走 ──
        //
        // egui-winit 对 Tab 是**无条件 consume** 的（它拿 Tab 做焦点轮换），
        // 所以只要交给它，我们的快捷键分支就永远不会执行，而且焦点会在顶栏
        // 按钮之间乱跳。这里抢先处理掉并直接返回，egui 根本收不到这个事件。
        // 例外：正在输入文字（工坊名字框）时让给 egui。
        if let WindowEvent::KeyboardInput { event: key, .. } = &event {
            if key.state == ElementState::Pressed
                && matches!(key.physical_key, PhysicalKey::Code(KeyCode::Tab))
                && !self.egui_ctx.wants_keyboard_input()
            {
                self.apply_ui_actions(ui::UiActions {
                    toggle_mode: true,
                    ..Default::default()
                });
                return;
            }
        }

        // 先让 egui 处理（它决定指针/键盘是否被 UI 吃掉）
        let egui_consumed = if let (Some(state), Some(window)) =
            (self.egui_state.as_mut(), self.window.as_ref())
        {
            state.on_window_event(window, &event).consumed
        } else {
            false
        };

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                if let (Some(config), Some(surface), Some(renderer)) = (
                    self.config.as_mut(),
                    self.surface.as_ref(),
                    self.renderer.as_ref(),
                ) {
                    config.width = size.width.max(1);
                    config.height = size.height.max(1);
                    surface.configure(&renderer.device, config);
                }
            }
            WindowEvent::RedrawRequested => {
                self.update_hover();
                self.step_simulation();
                if let Some(world) = self.world.as_ref() {
                    self.panels.sample(world);
                    self.panels.tick(world);
                }
                self.apply_follow();
                self.draw();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if egui_consumed || event.state != ElementState::Pressed {
                    return;
                }
                match event.physical_key {
                    PhysicalKey::Code(KeyCode::Escape) => {
                        if self.pending_place.take().is_some() {
                            // 只取消放置，不退出程序
                            self.place_msg.clear();
                        } else {
                            event_loop.exit();
                        }
                    }
                    PhysicalKey::Code(KeyCode::Space) => {
                        self.paused = !self.paused;
                        self.last = Instant::now();
                        self.acc = 0.0;
                    }
                    PhysicalKey::Code(KeyCode::KeyS) => {
                        self.paused = true;
                        if let Some(world) = self.world.as_mut() {
                            world.step();
                        }
                        self.upload_world();
                    }
                    PhysicalKey::Code(KeyCode::BracketRight) => {
                        self.speed_idx = (self.speed_idx + 1).min(SPEEDS.len() - 1);
                    }
                    PhysicalKey::Code(KeyCode::BracketLeft) => {
                        self.speed_idx = self.speed_idx.saturating_sub(1);
                    }
                    // Tab 已在上面抢先处理（egui-winit 会无条件吞掉它）
                    PhysicalKey::Code(KeyCode::KeyG) => {
                        self.workshop.open = !self.workshop.open;
                    }
                    PhysicalKey::Code(KeyCode::KeyW) => {
                        self.world_panel.open = !self.world_panel.open;
                    }
                    PhysicalKey::Code(KeyCode::KeyF) => {
                        self.apply_ui_actions(ui::UiActions {
                            fit: true,
                            ..Default::default()
                        });
                    }
                    _ => {}
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                if egui_consumed {
                    return;
                }
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(p) => (p.y as f32) / 50.0,
                };
                if dy != 0.0 {
                    let factor = if dy > 0.0 { 1.15 } else { 1.0 / 1.15 };
                    let (cx, cy) = self.last_cursor;
                    self.camera
                        .zoom_at(cx as f32, cy as f32, factor, MIN_ZOOM, MAX_ZOOM);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let (x, y) = (position.x, position.y);
                if self.dragging {
                    let dx = (x - self.last_cursor.0) as f32;
                    let dy = (y - self.last_cursor.1) as f32;
                    self.camera.pan_pixels(dx, dy);
                }
                self.last_cursor = (x, y);
                let (c, r) = paint::screen_to_cell(&self.camera, x as f32, y as f32);
                self.hover_cell = Some((c, r));

                if self.painting {
                    self.paint_at_cursor();
                }
            }
            WindowEvent::MouseInput { state, button, .. } => match button {
                MouseButton::Left => {
                    if state == ElementState::Pressed {
                        if !self.egui_ctx.wants_pointer_input() {
                            // ── 放置模式优先于一切工具 ──
                            //
                            // 用户反馈"手动关闭工坊后点画布放不上去"：原来投放藏在
                            // `paint_at_cursor` 里，而那条路径要求当前工具是「画笔」。
                            // 放置是一次性的显式意图，不该被"你手上拿的是什么工具"卡住。
                            match ui::left_action(self.pending_place.is_some(), self.ui.tool) {
                                ui::LeftAction::Place => self.place_pending_at_cursor(),
                                ui::LeftAction::Paint => {
                                    self.painting = true;
                                    self.paint_at_cursor();
                                }
                                ui::LeftAction::Inspect => self.inspect_at_cursor(),
                                ui::LeftAction::Pan => {
                                    // 平移工具：左键拖动即可平移（右键/中键任何时候都能拖）
                                    self.dragging = true;
                                }
                            }
                        }
                    } else {
                        self.painting = false;
                        if self.ui.tool == ui::Tool::Pan {
                            self.dragging = false;
                        }
                    }
                }
                MouseButton::Right | MouseButton::Middle => {
                    self.dragging = state == ElementState::Pressed;
                }
                _ => {}
            },
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(win) = self.window.as_ref() {
            win.request_redraw();
        }
    }
}

/// 录一帧 egui 的渲染 pass。
///
/// 单独成函数是因为 `egui_wgpu::Renderer::render` 的签名把 render pass 的
/// 生命周期与渲染器的借用绑成同一个 `'a`；写在 `render_frame` 里时编译器会把
/// `'a` 推到整个函数，于是 `encoder` 被判定"活得不够久"。放进函数参数里，
/// `'a` 就自然收窄成函数体。
fn record_egui_pass(
    egui_renderer: &mut egui_wgpu::Renderer,
    encoder: &mut wgpu::CommandEncoder,
    view: &wgpu::TextureView,
    tris: &[egui::ClippedPrimitive],
    screen: &egui_wgpu::ScreenDescriptor,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("egui-pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
            resolve_target: None,
            ops: wgpu::Operations {
                load: wgpu::LoadOp::Load,
                store: wgpu::StoreOp::Store,
            },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
    });
    // egui-wgpu 的 render 要求 `RenderPass<'static>`，所以这里用 forget_lifetime()
    // 把 pass 的生命周期抹掉（wgpu 的官方推荐写法，安全性由调用顺序保证：
    // pass 在 encoder.finish() 之前就 drop 了）。
    let mut pass_static = pass.forget_lifetime();
    egui_renderer.render(&mut pass_static, tris, screen);
}
