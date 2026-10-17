// 世界网格渲染 —— 全屏三角形 + 逐像素反查格子。
//
// 为什么不用"每个细胞一个四边形"：200×150 = 30000 格，实例化 3 万个 quad
// 在低倍率下是纯浪费。这里反过来做：屏幕像素 → 世界坐标 → 查纹理。
// 缩放、平移、世界环绕全部在着色器里完成，CPU 每帧只更新一个 48 字节的 uniform。

struct Uniforms {
    // 视口像素尺寸
    viewport: vec2<f32>,
    // 世界网格尺寸（格）
    grid: vec2<f32>,
    // 相机：左上角对应的世界坐标（格）
    cam: vec2<f32>,
    // 每格像素
    zoom: f32,
    // 0 = 类型，1 = 物种，2 = 温度场，3 = 辐射场
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
// 环境场（R8）：温度或辐射，由 CPU 决定这次上传的是哪一张
@group(0) @binding(4) var field_tex: texture_2d<f32>;
// 场的 256 项色表（RGBA8，切换滤镜时由 CPU 重建）
@group(0) @binding(5) var field_lut: texture_2d<f32>;

// 世界之外的背景色。刻意比"空格"（#0b1020）更暗一点，
// 这样缩小时能一眼看出世界的边界在哪。
const BACKDROP: vec4<f32> = vec4<f32>(0.019, 0.027, 0.047, 1.0);

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

    // 世界在屏幕上的像素尺寸
    let world_px = u.grid * u.zoom;

    var col = i32(floor(world.x));
    var row = i32(floor(world.y));

    // ── 只在"世界比视口大"的方向做环绕 ──
    //
    // 两个方向都要单独判断：世界可能一边比视口宽、另一边比视口窄。
    // 如果无条件取模，缩小到能看见整个世界时，画面会把同一个世界重复铺满屏幕
    // —— 1.x 踩过这个坑，这里用世界尺寸和视口尺寸的比较来根治。
    if (world_px.x < u.viewport.x) {
        if (col < 0 || col >= cols) {
            return BACKDROP;
        }
    } else {
        col = ((col % cols) + cols) % cols;
    }

    if (world_px.y < u.viewport.y) {
        if (row < 0 || row >= rows) {
            return BACKDROP;
        }
    } else {
        row = ((row % rows) + rows) % rows;
    }

    let uv = vec2<u32>(u32(col), u32(row));

    if (u.mode == 1u) {
        return textureLoad(species_tex, uv, 0);
    }

    // ── 环境场滤镜（温度 / 辐射）──
    //
    // 场值是 0–255，这里映射到 256 项色表上。和类型着色一样用 textureLoad，
    // 保证"一格一个纯色块"的像素风；色表由 CPU 按滤镜种类换，着色器不用改。
    if (u.mode >= 2u) {
        let v = clamp(textureLoad(field_tex, uv, 0).r, 0.0, 1.0);
        let k = u32(v * 255.0 + 0.5);
        return textureLoad(field_lut, vec2<u32>(k, 0u), 0);
    }

    let idx = textureLoad(types_tex, uv, 0).r;
    return textureLoad(type_palette, vec2<u32>(idx, 0u), 0);
}
