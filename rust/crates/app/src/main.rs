//! evosim 2.0 —— 原生窗口（M2：世界渲染 + 相机 + 着色模式 + 暂停/调速）。
//!
//! 用法：
//!   evosim-app                                   打开窗口
//!   evosim-app --screenshot out.png --ticks 500  离屏渲染一张 PNG（不需要窗口）
//!
//! 快捷键：空格 暂停/继续 · S 单步 · [ ] 调速 · Tab 类型/物种着色 · F 适应窗口 · Esc 退出
//! 鼠标：滚轮以光标为锚缩放 · 右键/中键拖动平移

use std::sync::Arc;
use std::time::Instant;

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

/// 无窗口压测：给定倍速跑 N 秒，报告实际 tps 与单 tick 耗时
fn bench(seed: u32, pop: usize, seconds: f64, speed: f64) {
    let mut world = build_world(seed, pop);
    let target = 60.0 * speed;

    // 预热，让种群长到稳态
    for _ in 0..600 {
        world.step();
    }

    let t0 = Instant::now();
    let mut ticks: u64 = 0;
    while t0.elapsed().as_secs_f64() < seconds {
        // 每个 tick 都跑，不做节流 —— 测的是"能不能跟上"，不是"节流后多准"
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
        let speed: f64 = arg_value("--speed").and_then(|v| v.parse().ok()).unwrap_or(1.0);
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
    /// 离散步进累加器
    acc: f64,
    last: Instant,

    frames: u32,
    fps_timer: Instant,
    fps: f32,
    tps: f32,
    ticks_this_second: u64,

    dragging: bool,
    last_cursor: (f64, f64),
}

impl App {
    fn new(seed: u32, pop: usize, pre_ticks: u64) -> Self {
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
        println!("快捷键：空格 暂停 · S 单步 · [ ] 调速 · Tab 着色模式 · F 适应窗口 · Esc 退出");

        self.window = Some(window);
        self.surface = Some(surface);
        self.config = Some(config);
        self.renderer = Some(renderer);
        self.world = Some(world);
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
            self.acc = 0.0; // 追不上就丢掉积压
        }
        if steps > 0 {
            if let (Some(renderer), Some(world)) =
                (self.renderer.as_mut(), self.world.as_ref())
            {
                renderer.upload_types(world);
                if renderer.mode == 1 {
                    renderer.upload_species(world);
                }
            }
        }
    }

    fn draw(&mut self) {
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
        renderer.draw(&view, config.width, config.height, &self.camera);
        frame.present();

        self.frames += 1;
        let elapsed = self.fps_timer.elapsed().as_secs_f32();
        if elapsed >= 0.5 {
            self.fps = self.frames as f32 / elapsed;
            self.tps = self.ticks_this_second as f32 / elapsed;
            self.frames = 0;
            self.ticks_this_second = 0;
            self.fps_timer = Instant::now();

            if let Some(world) = self.world.as_ref() {
                let stats = world.snapshot_stats();
                if let Some(win) = self.window.as_ref() {
                    win.set_title(&format!(
                        "evosim 2.0 · tick {} · {} 只 · 食物 {} · 物种 {} · {:.0} fps · {:.0} tps · {:.0}× {}",
                        world.tick,
                        stats.organisms,
                        stats.food,
                        world.species.size(),
                        self.fps,
                        self.tps,
                        SPEEDS[self.speed_idx],
                        if self.paused { "（暂停）" } else { "" },
                    ));
                }
            }
        }
    }

    fn toggle_mode(&mut self) {
        if let (Some(renderer), Some(world)) = (self.renderer.as_mut(), self.world.as_ref()) {
            renderer.set_mode(if renderer.mode == 0 { 1 } else { 0 });
            if renderer.mode == 1 {
                renderer.upload_species(world);
            }
        }
    }

    fn fit_window(&mut self) {
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

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            self.init_gpu(event_loop);
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
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
                self.step_simulation();
                self.draw();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
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
                        if let (Some(renderer), Some(world)) =
                            (self.renderer.as_mut(), self.world.as_ref())
                        {
                            renderer.upload_types(world);
                        }
                    }
                    PhysicalKey::Code(KeyCode::BracketRight) => {
                        self.speed_idx = (self.speed_idx + 1).min(SPEEDS.len() - 1);
                    }
                    PhysicalKey::Code(KeyCode::BracketLeft) => {
                        self.speed_idx = self.speed_idx.saturating_sub(1);
                    }
                    PhysicalKey::Code(KeyCode::Tab) => self.toggle_mode(),
                    PhysicalKey::Code(KeyCode::KeyF) => self.fit_window(),
                    _ => {}
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
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
            }
            WindowEvent::MouseInput { state, button, .. } => {
                let pan_button = matches!(button, MouseButton::Right | MouseButton::Middle);
                if pan_button {
                    self.dragging = state == ElementState::Pressed;
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(win) = self.window.as_ref() {
            win.request_redraw();
        }
    }
}
