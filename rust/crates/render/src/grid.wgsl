// 世界网格渲染 —— 全屏三角形 + 逐像素反查格子。
//
// 为什么不用"每个细胞一个四边形"：200×150 = 30000 格，实例化 3 万个 quad
// 在低倍率下是纯浪费。这里反过来做：屏幕像素 → 世界坐标 → 查纹理。
// 缩放、平移、世界平铺全部在着色器里完成，CPU 每帧只更新一个 48 字节的 uniform。

struct Uniforms {
    // 视口像素尺寸
    viewport: vec2<f32>,
    // 世界网格尺寸（格）
    grid: vec2<f32>,
    // 相机：左上角对应的世界坐标（格）
    cam: vec2<f32>,
    // 每格像素
    zoom: f32,
    // 0 = 按类型着色，1 = 按物种着色
    mode: u32,
    _pad: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
// 顶层类型（R8Uint）：生物的身体会盖住地形
@group(0) @binding(1) var types_tex: texture_2d<u32>;
// 按物种着色的逐格颜色（RGBA8，切到物种模式时由 CPU 重建）
@group(0) @binding(2) var species_tex: texture_2d<f32>;
// 类型调色板（256 项 RGBA8）
@group(0) @binding(3) var type_palette: texture_2d<f32>;

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> @builtin(position) vec4<f32> {
    // 一个覆盖整个裁剪空间的大三角形
    var pos = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -3.0),
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(3.0, 1.0),
    );
    return vec4<f32>(pos[vi], 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) frag: vec4<f32>) -> @location(0) vec4<f32> {
    // frag.xy 是帧缓冲像素坐标（左上原点，与相机约定一致）
    let world = u.cam + frag.xy / u.zoom;

    let cols = i32(u.grid.x);
    let rows = i32(u.grid.y);

    // 世界坐标取整 → 环绕折回（与 Grid::index 的 rem_euclid 语义一致）
    var col = i32(floor(world.x));
    var row = i32(floor(world.y));
    col = ((col % cols) + cols) % cols;
    row = ((row % rows) + rows) % rows;

    let uv = vec2<u32>(u32(col), u32(row));

    if (u.mode == 1u) {
        return textureLoad(species_tex, uv, 0);
    }

    let idx = textureLoad(types_tex, uv, 0).r;
    return textureLoad(type_palette, vec2<u32>(idx, 0u), 0);
}
