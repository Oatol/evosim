//! 渲染层：wgpu 世界渲染（R8 索引纹理 + 调色板 / 物种色纹理）。
//!
//! 设计要点：
//!   · **屏幕像素 → 世界坐标 → 查纹理**，而不是给每个细胞画一个 quad。
//!     30000 格在低倍率下全屏覆盖，逐格实例化纯属浪费。
//!   · 缩放、平移、世界平铺全在着色器里做，CPU 每帧只写 48 字节 uniform。
//!   · 类型着色走 **R8 索引 + 256 项调色板纹理**（30 KB/帧），
//!     物种着色走一张 RGBA8 纹理（切模式时才重建，30k 格约 0.1 ms）。
//!   · 采样一律 `textureLoad`（等价 NEAREST），保证像素风不被糊掉。
//!
//! 这个 crate 不依赖 winit：既能挂到窗口 surface 上，也能离屏渲染成 PNG
//! （自动化截图、回归比对都靠它）。

pub mod colors;

use evosim_core::world::World;

pub const SHADER: &str = include_str!("grid.wgsl");

/// 相机：左上角对应的世界坐标 + 每格像素
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub x: f32,
    pub y: f32,
    pub zoom: f32,
}

impl Camera {
    pub fn new(zoom: f32) -> Self {
        Camera { x: 0.0, y: 0.0, zoom }
    }

    pub fn world_to_screen(&self, col: f32, row: f32) -> (f32, f32) {
        ((col - self.x) * self.zoom, (row - self.y) * self.zoom)
    }

    pub fn screen_to_world(&self, px: f32, py: f32) -> (f32, f32) {
        (self.x + px / self.zoom, self.y + py / self.zoom)
    }

    /// 以屏幕点 (sx, sy) 为锚缩放
    pub fn zoom_at(&mut self, sx: f32, sy: f32, factor: f32, min: f32, max: f32) {
        let (wx, wy) = self.screen_to_world(sx, sy);
        self.zoom = (self.zoom * factor).clamp(min, max);
        self.x = wx - sx / self.zoom;
        self.y = wy - sy / self.zoom;
    }

    /// 按屏幕像素平移
    pub fn pan_pixels(&mut self, dx: f32, dy: f32) {
        self.x -= dx / self.zoom;
        self.y -= dy / self.zoom;
    }

    /// 让整个世界装进视口
    pub fn fit_grid(&mut self, cols: usize, rows: usize, vw: f32, vh: f32, pad: f32) {
        let zx = (vw - pad * 2.0) / cols as f32;
        let zy = (vh - pad * 2.0) / rows as f32;
        self.zoom = zx.min(zy).max(0.2);
        self.x = (cols as f32 - vw / self.zoom) * 0.5;
        self.y = (rows as f32 - vh / self.zoom) * 0.5;
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    viewport: [f32; 2],
    grid: [f32; 2],
    cam: [f32; 2],
    zoom: f32,
    mode: u32,
    _pad: [f32; 4],
}

pub struct GridRenderer {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniform_buf: wgpu::Buffer,
    types_tex: wgpu::Texture,
    species_tex: wgpu::Texture,
    palette_tex: wgpu::Texture,
    cols: u32,
    rows: u32,
    /// 0 = 类型着色，1 = 物种着色
    pub mode: u32,
}

impl GridRenderer {
    pub fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        format: wgpu::TextureFormat,
        cols: u32,
        rows: u32,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("grid-shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("grid-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("grid-pipeline-layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("grid-pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });

        let uniform_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("grid-uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let types_tex = create_tex(&device, cols, rows, wgpu::TextureFormat::R8Uint, "grid-types");
        let species_tex = create_tex(
            &device,
            cols,
            rows,
            wgpu::TextureFormat::Rgba8Unorm,
            "grid-species",
        );

        let palette = colors::build_palette();
        let pal_bytes: Vec<u8> = palette.iter().flat_map(|c| c.iter().copied()).collect();
        let palette_tex = create_tex(&device, 256, 1, wgpu::TextureFormat::Rgba8Unorm, "grid-palette");
        write_texture(&queue, &palette_tex, 256, 1, 4, &pal_bytes);

        let types_view = types_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let species_view = species_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let palette_view = palette_tex.create_view(&wgpu::TextureViewDescriptor::default());

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("grid-bind-group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buf.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&types_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&species_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&palette_view),
                },
            ],
        });

        GridRenderer {
            device,
            queue,
            pipeline,
            bind_group,
            uniform_buf,
            types_tex,
            species_tex,
            palette_tex,
            cols,
            rows,
            mode: 0,
        }
    }

    /// 顶层类型 → R8 纹理（每帧一次，30 KB）
    pub fn upload_types(&mut self, world: &World) {
        let grid = &world.grid;
        write_texture(
            &self.queue,
            &self.types_tex,
            grid.cols,
            grid.rows,
            1,
            &grid.types,
        );
    }

    /// 按物种着色 → RGBA8 纹理（切模式或物种变化时重建）
    pub fn upload_species(&mut self, world: &World) {
        let pixels = colors::build_species_pixels(world);
        let bytes: Vec<u8> = pixels.iter().flat_map(|c| c.iter().copied()).collect();
        write_texture(
            &self.queue,
            &self.species_tex,
            self.cols as usize,
            self.rows as usize,
            4,
            &bytes,
        );
    }

    pub fn set_mode(&mut self, mode: u32) {
        self.mode = mode;
    }

    /// 画一帧到指定视图
    pub fn draw(&mut self, view: &wgpu::TextureView, width: u32, height: u32, cam: &Camera) {
        let uniforms = Uniforms {
            viewport: [width as f32, height as f32],
            grid: [self.cols as f32, self.rows as f32],
            cam: [cam.x, cam.y],
            zoom: cam.zoom,
            mode: self.mode,
            _pad: [0.0; 4],
        };
        self.queue
            .write_buffer(&self.uniform_buf, 0, bytemuck::bytes_of(&uniforms));

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("grid-enc") });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("grid-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // 与世界之外的背景色一致（着色器里的 BACKDROP）
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.019,
                            g: 0.027,
                            b: 0.047,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit(Some(encoder.finish()));
    }

    pub fn world_size(&self) -> (u32, u32) {
        (self.cols, self.rows)
    }

    pub fn palette_handle(&self) -> &wgpu::Texture {
        &self.palette_tex
    }
}

fn create_tex(
    device: &wgpu::Device,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    label: &str,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// 写纹理（每行补齐到 256 字节，满足 wgpu 的对齐要求）
fn write_texture(
    queue: &wgpu::Queue,
    tex: &wgpu::Texture,
    cols: usize,
    rows: usize,
    bytes_per_px: usize,
    data: &[u8],
) {
    let row_bytes = cols * bytes_per_px;
    let padded = row_bytes.div_ceil(256) * 256;

    let payload: Vec<u8> = if padded == row_bytes {
        data.to_vec()
    } else {
        let mut v = vec![0u8; padded * rows];
        for r in 0..rows {
            v[r * padded..r * padded + row_bytes]
                .copy_from_slice(&data[r * row_bytes..(r + 1) * row_bytes]);
        }
        v
    };

    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &payload,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(padded as u32),
            rows_per_image: Some(rows as u32),
        },
        wgpu::Extent3d {
            width: cols as u32,
            height: rows as u32,
            depth_or_array_layers: 1,
        },
    );
}

/// 创建一个无窗口的 wgpu 设备（离屏渲染 / 截图 / 测试用）
pub fn create_headless_device() -> Option<(wgpu::Device, wgpu::Queue)> {
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends: wgpu::Backends::from_env().unwrap_or(wgpu::Backends::all()),
        ..Default::default()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    }))?;
    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("evosim-headless"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::downlevel_defaults(),
            memory_hints: Default::default(),
        },
        None,
    ))
    .ok()?;
    Some((device, queue))
}

/// 离屏渲染一帧并读回 RGBA8 像素（行优先、无 padding）
pub fn render_to_rgba(
    renderer: &mut GridRenderer,
    width: u32,
    height: u32,
    cam: &Camera,
) -> Vec<u8> {
    let tex = renderer.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("offscreen"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    renderer.draw(&view, width, height, cam);

    let row_bytes = width as usize * 4;
    let padded = row_bytes.div_ceil(256) * 256;
    let buffer = renderer.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (padded * height as usize) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = renderer
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("copy") });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded as u32),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    renderer.queue.submit(Some(encoder.finish()));

    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    let _ = renderer.device.poll(wgpu::Maintain::Wait);

    let data = slice.get_mapped_range();
    let mut out = vec![0u8; row_bytes * height as usize];
    for r in 0..height as usize {
        out[r * row_bytes..(r + 1) * row_bytes]
            .copy_from_slice(&data[r * padded..r * padded + row_bytes]);
    }
    drop(data);
    buffer.unmap();
    out
}

/// 把 RGBA 像素写成 PNG
pub fn save_png(path: &str, width: u32, height: u32, rgba: &[u8]) -> std::io::Result<()> {
    let file = std::fs::File::create(path)?;
    let w = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(w, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header()?;
    writer.write_image_data(rgba)?;
    Ok(())
}
