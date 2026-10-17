//! evosim 2.0 —— 原生窗口（M2 渲染 + M3 界面）。
//!
//! 用法：
//!   evosim-app                                   打开窗口
//!   evosim-app --screenshot out.png --ticks 500  离屏渲染一张 PNG（不需要窗口）
//!   evosim-app --bench --seconds 4 --speed 50    无窗口稳态压测
//!
//! 快捷键：空格 暂停/继续 · S 单步 · [ ] 调速 · Tab 类型/物种着色 · F 适应窗口 · Esc 退出
//! 鼠标：左键落笔（画笔工具）· 滚轮以光标为锚缩放 · 右键/中键拖动平移

mod paint;
mod ui;

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
    let mut world = build_world(seed, pop);
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

    let mode = if arg_value("--mode").as_deref() == Some("species") {
        1
    } else {
        0
    };
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

        let mut world = build_world(self.seed, self.pop);
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
        println!("空格 暂停 · S 单步 · [ ] 调速 · Tab 着色 · F 适应 · 左键画 · 右键拖 · 滚轮缩放");

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
            if let (Some(renderer), Some(world)) = (self.renderer.as_mut(), self.world.as_ref()) {
                renderer.set_mode(if renderer.mode == 0 { 1 } else { 0 });
                if renderer.mode == 1 {
                    renderer.upload_species(world);
                }
            }
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
    }

    fn paint_at_cursor(&mut self) {
        if self.ui.tool != ui::Tool::Paint || self.ui_wants_pointer {
            return;
        }
        let Some((col, row)) = self.hover_cell else {
            return;
        };
        if let Some(world) = self.world.as_mut() {
            paint::apply_brush(world, col, row, self.ui.brush, self.ui.brush_size);
        }
        self.upload_world();
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
        };

        let ctx = self.egui_ctx.clone();
        let mut actions = ui::UiActions::default();
        {
            let ui_state = &mut self.ui;
            let world = match self.world.as_mut() {
                Some(w) => w,
                None => return,
            };
            let full = ctx.run(raw, |ctx| {
                actions = ui::draw(ctx, ui_state, world, &hud);
            });
            if let Some(state) = self.egui_state.as_mut() {
                state.handle_platform_output(&window, full.platform_output);
            }
            self.apply_ui_actions(std::mem::take(&mut actions));
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
                self.draw();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if egui_consumed || event.state != ElementState::Pressed {
                    return;
                }
                match event.physical_key {
                    PhysicalKey::Code(KeyCode::Escape) => event_loop.exit(),
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
                    PhysicalKey::Code(KeyCode::Tab) => {
                        self.apply_ui_actions(ui::UiActions {
                            toggle_mode: true,
                            ..Default::default()
                        });
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
                            self.painting = true;
                            self.paint_at_cursor();
                        }
                    } else {
                        self.painting = false;
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
