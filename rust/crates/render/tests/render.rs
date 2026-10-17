//! 渲染正确性回归 —— 不靠肉眼，靠像素。
//!
//! 关键断言：
//!   1. 已知格子渲染出**调色板里的精确颜色**（不是近似色）
//!   2. NEAREST 采样：一格在 zoom=10 下正好铺满 10×10 像素的实心方块
//!   3. 世界平铺：相机移出世界范围后，对面边缘的格子会绕回来
//!   4. 物种着色模式与类型着色模式的像素确实不同

use evosim_core::cell_type::ct;
use evosim_core::config::Config;
use evosim_core::rng::Rng;
use evosim_core::world::World;
use evosim_render::colors::build_palette;
use evosim_render::{render_to_rgba, Camera, GridRenderer};

const W: u32 = 80;
const H: u32 = 80;

fn px(rgba: &[u8], x: u32, y: u32) -> [u8; 3] {
    let i = ((y * W + x) * 4) as usize;
    [rgba[i], rgba[i + 1], rgba[i + 2]]
}

fn palette_rgb(index: u8) -> [u8; 3] {
    let p = build_palette()[index as usize];
    [p[0], p[1], p[2]]
}

#[test]
fn known_cells_render_exact_palette_colors() {
    let Some((device, queue)) = evosim_render::create_headless_device() else {
        eprintln!("跳过：没有可用的 GPU 适配器");
        return;
    };

    let mut world = small_world(8, 8);
    world.fill_grid(ct::EMPTY, false);
    world.set_cell(2, 2, ct::WALL, -1);
    world.set_cell(3, 2, ct::FOOD, -1);
    world.set_cell(4, 2, ct::LAVA, -1);

    let mut r = GridRenderer::new(device, queue, wgpu::TextureFormat::Rgba8Unorm, 8, 8);
    r.upload_types(&world);
    r.upload_species(&world);
    r.set_mode(0);

    let cam = Camera {
        x: 0.0,
        y: 0.0,
        zoom: 10.0,
    };
    let rgba = render_to_rgba(&mut r, W, H, &cam);
    assert_eq!(rgba.len(), (W * H * 4) as usize);

    // 格子 (2,2) 覆盖像素 x∈[20,30) y∈[20,30)
    assert_eq!(px(&rgba, 25, 25), palette_rgb(ct::WALL), "墙的颜色不对");
    assert_eq!(px(&rgba, 35, 25), palette_rgb(ct::FOOD), "食物的颜色不对");
    assert_eq!(px(&rgba, 45, 25), palette_rgb(ct::LAVA), "岩浆的颜色不对");
    assert_eq!(px(&rgba, 5, 5), palette_rgb(ct::EMPTY), "空地的颜色不对");
}

#[test]
fn nearest_sampling_gives_solid_blocks() {
    let Some((device, queue)) = evosim_render::create_headless_device() else {
        return;
    };
    let mut world = small_world(8, 8);
    world.fill_grid(ct::EMPTY, false);
    world.set_cell(1, 1, ct::WALL, -1);

    let mut r = GridRenderer::new(device, queue, wgpu::TextureFormat::Rgba8Unorm, 8, 8);
    r.upload_types(&world);
    let cam = Camera {
        x: 0.0,
        y: 0.0,
        zoom: 10.0,
    };
    let rgba = render_to_rgba(&mut r, W, H, &cam);

    // 整块 10×10 必须是同一个颜色（NEAREST，不许插值出渐变）
    let wall = palette_rgb(ct::WALL);
    for y in 10..20 {
        for x in 10..20 {
            assert_eq!(px(&rgba, x, y), wall, "格子内 ({},{}) 出现了非纯色", x, y);
        }
    }
    // 边界外一格必须是空地色
    assert_eq!(px(&rgba, 9, 15), palette_rgb(ct::EMPTY), "左边界外不该是墙");
    assert_eq!(px(&rgba, 20, 15), palette_rgb(ct::EMPTY), "右边界外不该是墙");
}

#[test]
fn world_wraps_when_camera_goes_negative() {
    let Some((device, queue)) = evosim_render::create_headless_device() else {
        return;
    };
    let mut world = small_world(8, 8);
    world.fill_grid(ct::EMPTY, false);
    // 右下角放墙：相机往左上移一格时，它应该出现在左上角
    world.set_cell(7, 7, ct::WALL, -1);

    let mut r = GridRenderer::new(device, queue, wgpu::TextureFormat::Rgba8Unorm, 8, 8);
    r.upload_types(&world);

    let cam = Camera {
        x: -1.0,
        y: -1.0,
        zoom: 10.0,
    };
    let rgba = render_to_rgba(&mut r, W, H, &cam);
    // 世界 (-1,-1) → 屏幕 (0,0)，而 (-1,-1) 折回就是 (7,7)
    assert_eq!(
        px(&rgba, 5, 5),
        palette_rgb(ct::WALL),
        "世界环绕没生效：(-1,-1) 应折回 (7,7)"
    );
}

#[test]
fn species_mode_uses_fingerprint_color() {
    let Some((device, queue)) = evosim_render::create_headless_device() else {
        return;
    };
    use evosim_core::anatomy::Flags;
    use evosim_core::factory::create_origin_organism;

    let mut world = small_world(8, 8);
    world.fill_grid(ct::EMPTY, false);
    let org = create_origin_organism(&mut world, 3, 3);
    let fp = org.anatomy.fingerprint();
    assert!(org.anatomy.has(Flags::MOUTH));
    world.add_organism(org);

    let mut r = GridRenderer::new(device, queue, wgpu::TextureFormat::Rgba8Unorm, 8, 8);
    r.upload_types(&world);
    r.upload_species(&world);

    let cam = Camera {
        x: 0.0,
        y: 0.0,
        zoom: 10.0,
    };

    r.set_mode(0);
    let typed = render_to_rgba(&mut r, W, H, &cam);
    r.set_mode(1);
    let by_species = render_to_rgba(&mut r, W, H, &cam);

    // 枢轴格 (3,3) 是嘴：类型模式 = 嘴的颜色，物种模式 = 该物种指纹的颜色
    assert_eq!(px(&typed, 35, 35), palette_rgb(ct::MOUTH), "类型模式下枢轴格应是嘴色");

    let rgb = evosim_render::colors::species_rgb(&fp);
    assert_eq!(px(&by_species, 35, 35), rgb, "物种模式下枢轴格应是指纹色");

    // 背景（空格）在两种模式下都是空地色
    let empty = palette_rgb(ct::EMPTY);
    assert_eq!(px(&typed, 5, 5), empty);
    assert_eq!(px(&by_species, 5, 5), empty);
}

/// 指定尺寸的小世界（渲染测试用，避免 200×150 的默认世界）
fn small_world(cols: usize, rows: usize) -> World {
    let mut cfg = Config::default();
    cfg.world.cols = cols;
    cfg.world.rows = rows;
    World::new(cfg, Rng::new(1))
}
