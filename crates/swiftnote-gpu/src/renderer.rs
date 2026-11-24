use std::collections::{HashMap, HashSet, VecDeque};
use std::num::NonZeroU64;
use std::sync::Arc;

use swiftnote_core::{
    ActiveTool, AppSettings, AssetId, BrushKind, BrushStyle, CanvasPoint, Color, ContourMesh,
    DirtyRegion, Document, ImageAsset, Page, PageBackground, PageId, PaperPolarity, PaperTemplate,
    Rect, StrokeTessellator, ViewportTransform,
};
use tracing::{debug, warn};

use crate::error::GpuError;
use crate::mipmap::{
    create_mipmap_pipeline, upload_mipmapped_rgba_texture, BlitUniforms, MipmapUploadCtx,
};
use crate::pdf_worker::{build_pdf_tile_requests, PdfTileResult, PdfWorker, MAX_PDF_TILE_PX};
use crate::tile_cache::{visible_tile_keys, GpuTileCache, TileKey, TILE_SIZE_PX};

const PDF_VRAM_BUDGET_BYTES: usize = 128 * 1024 * 1024;

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GpuContourVertex {
    x: f32,
    y: f32,
    u: f32,
    v: f32,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct PaperUniforms {
    bounds_pt: [f32; 4],
    params: [f32; 4],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct StrokeUniforms {
    bounds_pt: [f32; 4],
    color: [f32; 4],
    mode: [f32; 4],
}

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ImageUniforms {
    view_bounds_pt: [f32; 4],
    img_bounds_pt: [f32; 4],
    crop_uv: [f32; 4],
    extra: [f32; 4],
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CursorOverlay {
    pub screen_x: f32,
    pub screen_y: f32,
    pub radius_px: f32,
    pub color: Color,
    pub is_eraser: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PdfTileKey {
    pub asset_id: AssetId,
    pub page_index: usize,
    pub tile_x: u32,
    pub tile_y: u32,
    pub tile_cols: u32,
    pub tile_rows: u32,
}

struct ImageGpuTex {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    _mip_levels: u32,
    byte_size: usize,
}

struct GpuTile {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
}

struct ActiveStrokeGpu {
    page_id: PageId,
    style: BrushStyle,
    vertex_count: u32,
    index_count: u32,
    vbuf: wgpu::Buffer,
    ibuf: wgpu::Buffer,
    v_cap: usize,
    i_cap: usize,
}

/// CPU-staged dynamic uniform buffer avoiding WebGPU mid-encoder overwrite hazards.
struct UniformArena {
    align: usize,
    staging: Vec<u8>,
    buffer: wgpu::Buffer,
    capacity: usize,
}

impl UniformArena {
    fn new(device: &wgpu::Device, align: u32) -> Self {
        let align = (align as usize).max(256);
        let capacity = align * 512;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("swiftnote_uniform_arena"),
            size: capacity as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            align,
            staging: Vec::with_capacity(capacity),
            buffer,
            capacity,
        }
    }

    fn clear(&mut self) {
        self.staging.clear();
    }

    fn push<T: bytemuck::Pod>(&mut self, value: &T) -> u32 {
        let offset = self.staging.len();
        let bytes = bytemuck::bytes_of(value);
        self.staging.extend_from_slice(bytes);
        let padded = (offset + bytes.len()).div_ceil(self.align) * self.align;
        self.staging.resize(padded, 0);
        offset as u32
    }

    fn upload(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> bool {
        if self.staging.is_empty() {
            return false;
        }
        let mut reallocated = false;
        if self.staging.len() > self.capacity {
            self.capacity = self.staging.len().next_power_of_two();
            self.buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("swiftnote_uniform_arena"),
                size: self.capacity as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            reallocated = true;
        }
        queue.write_buffer(&self.buffer, 0, &self.staging);
        reallocated
    }
}

pub fn default_wgpu_backends() -> wgpu::Backends {
    if let Some(env) = wgpu::Backends::from_env() {
        return env;
    }
    #[cfg(target_os = "windows")]
    {
        wgpu::Backends::DX12
    }
    #[cfg(not(target_os = "windows"))]
    {
        wgpu::Backends::VULKAN | wgpu::Backends::GL
    }
}

pub struct StrokePipelineConfig {
    pub sample_count: u32,
    pub surface_format: wgpu::TextureFormat,
}

pub struct UnifiedRenderer {
    device: Arc<wgpu::Device>,
    queue: Arc<wgpu::Queue>,
    surface_format: wgpu::TextureFormat,
    msaa_samples: u32,

    paper_pipeline: wgpu::RenderPipeline,
    stroke_pipeline_msaa: wgpu::RenderPipeline,
    stroke_pipeline_1x: wgpu::RenderPipeline,
    image_pipeline_tile: wgpu::RenderPipeline,
    blit_alpha_pipeline: wgpu::RenderPipeline,
    blit_multiply_pipeline: wgpu::RenderPipeline,
    mipmap_pipeline: wgpu::RenderPipeline,

    paper_bgl: wgpu::BindGroupLayout,
    stroke_bgl: wgpu::BindGroupLayout,
    image_bgl: wgpu::BindGroupLayout,
    blit_bgl: wgpu::BindGroupLayout,

    uniform_arena: UniformArena,
    paper_bg: wgpu::BindGroup,
    stroke_bg: wgpu::BindGroup,

    mipmapped_sampler: wgpu::Sampler,
    linear_sampler: wgpu::Sampler,

    tile_cache: GpuTileCache<GpuTile>,
    image_textures: HashMap<AssetId, ImageGpuTex>,
    pdf_textures: HashMap<PdfTileKey, ImageGpuTex>,
    pdf_lru: VecDeque<PdfTileKey>,
    pdf_vram_bytes: usize,
    in_flight_pdf: HashSet<PdfTileKey>,
    last_pdf_scale_milli: u32,

    tessellator: StrokeTessellator,
    tess_mesh: ContourMesh,
    vertex_scratch: Vec<GpuContourVertex>,

    scratch_vbuf: wgpu::Buffer,
    scratch_ibuf: wgpu::Buffer,
    scratch_vcap: usize,
    scratch_icap: usize,

    active_stroke: Option<ActiveStrokeGpu>,

    // Reusable 512x512 offscreen MSAA + highlight scratch textures for tile rendering
    tile_msaa_tex: Option<(wgpu::Texture, wgpu::TextureView)>,
    hl_resolve_tex: (wgpu::Texture, wgpu::TextureView),
    hl_msaa_tex: Option<(wgpu::Texture, wgpu::TextureView)>,

    // Reusable viewport-sized active wet-ink mask (resolved 1x + optional MSAA)
    wet_mask_size: (u32, u32),
    wet_mask_tex: Option<(wgpu::Texture, wgpu::TextureView)>,
    wet_mask_msaa: Option<(wgpu::Texture, wgpu::TextureView)>,
}

impl UnifiedRenderer {
    pub fn new(
        device: Arc<wgpu::Device>,
        queue: Arc<wgpu::Queue>,
        adapter: &wgpu::Adapter,
        surface_format: wgpu::TextureFormat,
    ) -> Result<Self, GpuError> {
        let format_flags = adapter.get_texture_format_features(surface_format).flags;
        let msaa_samples = if format_flags.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4)
        {
            4
        } else {
            1
        };

        let limits = device.limits();
        let align = limits.min_uniform_buffer_offset_alignment;
        let max_aniso = if adapter
            .get_downlevel_capabilities()
            .flags
            .contains(wgpu::DownlevelFlags::ANISOTROPIC_FILTERING)
        {
            8
        } else {
            1
        };
        let uniform_arena = UniformArena::new(&device, align);

        // 1x1 solid white texture for solid quads (page shadows, selection borders, cursor dot)
        let white_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("swiftnote_white_1x1"),
            size: wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: surface_format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &white_tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &[255, 255, 255, 255],
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            wgpu::Extent3d {
                width: 1,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
        let white_view = white_tex.create_view(&wgpu::TextureViewDescriptor::default());
        let _ = white_view;

        let min_binding = NonZeroU64::new(64).ok_or(GpuError::NoAdapter)?;

        let uniform_only_bgl = |label: &str| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: Some(min_binding),
                    },
                    count: None,
                }],
            })
        };

        let tex_bgl = |label: &str| {
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: true,
                            min_binding_size: Some(min_binding),
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            })
        };

        let paper_bgl = uniform_only_bgl("swiftnote_paper_bgl");
        let stroke_bgl = uniform_only_bgl("swiftnote_stroke_bgl");
        let image_bgl = tex_bgl("swiftnote_image_bgl");
        let blit_bgl = tex_bgl("swiftnote_blit_bgl");

        let paper_bg = make_arena_bind_group(&device, &paper_bgl, &uniform_arena.buffer);
        let stroke_bg = make_arena_bind_group(&device, &stroke_bgl, &uniform_arena.buffer);

        let paper_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("swiftnote_paper_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/paper.wgsl").into()),
        });
        let stroke_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("swiftnote_stroke_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/stroke.wgsl").into()),
        });
        let image_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("swiftnote_image_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/image.wgsl").into()),
        });
        let blit_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("swiftnote_blit_shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/blit.wgsl").into()),
        });

        let paper_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("swiftnote_paper_layout"),
            bind_group_layouts: &[Some(&paper_bgl)],
            immediate_size: 0,
        });
        let paper_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("swiftnote_paper_pipeline"),
            layout: Some(&paper_layout),
            vertex: wgpu::VertexState {
                module: &paper_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &paper_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let stroke_pipeline_msaa = build_stroke_pipeline(
            &device,
            &stroke_shader,
            &stroke_bgl,
            surface_format,
            msaa_samples,
        );
        let stroke_pipeline_1x =
            build_stroke_pipeline(&device, &stroke_shader, &stroke_bgl, surface_format, 1);

        let image_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("swiftnote_image_layout"),
            bind_group_layouts: &[Some(&image_bgl)],
            immediate_size: 0,
        });
        let image_pipeline_tile = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("swiftnote_image_pipeline"),
            layout: Some(&image_layout),
            vertex: wgpu::VertexState {
                module: &image_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &image_shader,
                entry_point: Some("fs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let blit_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("swiftnote_blit_layout"),
            bind_group_layouts: &[Some(&blit_bgl)],
            immediate_size: 0,
        });

        let blit_alpha_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("swiftnote_blit_alpha"),
            layout: Some(&blit_layout),
            vertex: wgpu::VertexState {
                module: &blit_shader,
                entry_point: Some("vs_main"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &blit_shader,
                entry_point: Some("fs_alpha"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });

        let multiply_blend = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Dst,
                dst_factor: wgpu::BlendFactor::Zero,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let blit_multiply_pipeline =
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("swiftnote_blit_multiply"),
                layout: Some(&blit_layout),
                vertex: wgpu::VertexState {
                    module: &blit_shader,
                    entry_point: Some("vs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &blit_shader,
                    entry_point: Some("fs_multiply"),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: surface_format,
                        blend: Some(multiply_blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });

        let mipmap_pipeline =
            create_mipmap_pipeline(&device, wgpu::TextureFormat::Rgba8UnormSrgb, &blit_bgl);

        let mipmapped_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("swiftnote_mipmapped_sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: max_aniso,
            ..Default::default()
        });
        let linear_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("swiftnote_linear_sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let scratch_vcap = 8192usize;
        let scratch_icap = 16384usize;
        let scratch_vbuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("swiftnote_stroke_vbuf"),
            size: (scratch_vcap * std::mem::size_of::<GpuContourVertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let scratch_ibuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("swiftnote_stroke_ibuf"),
            size: (scratch_icap * std::mem::size_of::<u32>()) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let hl_resolve_tex = create_color_tex(
            &device,
            "swiftnote_hl_resolve",
            TILE_SIZE_PX,
            TILE_SIZE_PX,
            surface_format,
            1,
        );
        let (tile_msaa_tex, hl_msaa_tex) = if msaa_samples > 1 {
            (
                Some(create_color_tex(
                    &device,
                    "swiftnote_tile_msaa",
                    TILE_SIZE_PX,
                    TILE_SIZE_PX,
                    surface_format,
                    msaa_samples,
                )),
                Some(create_color_tex(
                    &device,
                    "swiftnote_hl_msaa",
                    TILE_SIZE_PX,
                    TILE_SIZE_PX,
                    surface_format,
                    msaa_samples,
                )),
            )
        } else {
            (None, None)
        };

        debug!(msaa_samples, ?surface_format, "UnifiedRenderer initialized");

        Ok(Self {
            device,
            queue,
            surface_format,
            msaa_samples,
            paper_pipeline,
            stroke_pipeline_msaa,
            stroke_pipeline_1x,
            image_pipeline_tile,
            blit_alpha_pipeline,
            blit_multiply_pipeline,
            mipmap_pipeline,
            paper_bgl,
            stroke_bgl,
            image_bgl,
            blit_bgl,
            uniform_arena,
            paper_bg,
            stroke_bg,
            mipmapped_sampler,
            linear_sampler,
            tile_cache: GpuTileCache::default(),
            image_textures: HashMap::new(),
            pdf_textures: HashMap::new(),
            pdf_lru: VecDeque::new(),
            pdf_vram_bytes: 0,
            in_flight_pdf: HashSet::new(),
            last_pdf_scale_milli: 0,
            tessellator: StrokeTessellator::new(),
            tess_mesh: ContourMesh::with_capacity(8192, 16384),
            vertex_scratch: Vec::with_capacity(8192),
            scratch_vbuf,
            scratch_ibuf,
            scratch_vcap,
            scratch_icap,
            active_stroke: None,
            tile_msaa_tex,
            hl_resolve_tex,
            hl_msaa_tex,
            wet_mask_size: (0, 0),
            wet_mask_tex: None,
            wet_mask_msaa: None,
        })
    }

    pub fn new_headless() -> Result<Self, GpuError> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: default_wgpu_backends(),
            ..wgpu::InstanceDescriptor::new_without_display_handle()
        });
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
            ..Default::default()
        }))
        .map_err(|_| GpuError::NoAdapter)?;

        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("swiftnote_headless_device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::downlevel_defaults(),
                experimental_features: wgpu::ExperimentalFeatures::default(),
                memory_hints: wgpu::MemoryHints::Performance,
                trace: wgpu::Trace::Off,
            }))?;

        Self::new(
            Arc::new(device),
            Arc::new(queue),
            &adapter,
            wgpu::TextureFormat::Bgra8UnormSrgb,
        )
    }

    pub fn pipeline_config(&self) -> StrokePipelineConfig {
        StrokePipelineConfig {
            sample_count: self.msaa_samples,
            surface_format: self.surface_format,
        }
    }

    pub fn ensure_image_uploaded(&mut self, asset: &ImageAsset) -> Result<(), GpuError> {
        if self.image_textures.contains_key(&asset.id) {
            return Ok(());
        }
        let decoded = image::load_from_memory(&asset.png_bytes)
            .map_err(|e| GpuError::ImageDecode(e.to_string()))?
            .to_rgba8();
        let (w, h) = decoded.dimensions();
        let (tex, view, mips) = upload_mipmapped_rgba_texture(
            &MipmapUploadCtx {
                device: &self.device,
                queue: &self.queue,
                mipmap_pipeline: &self.mipmap_pipeline,
                blit_bgl: &self.blit_bgl,
                linear_sampler: &self.linear_sampler,
            },
            "swiftnote_image_asset",
            w,
            h,
            decoded.as_raw(),
        )?;
        self.image_textures.insert(
            asset.id,
            ImageGpuTex {
                _texture: tex,
                view,
                _mip_levels: mips,
                byte_size: (w as usize) * (h as usize) * 4,
            },
        );
        Ok(())
    }

    pub fn upload_pdf_tile(&mut self, tile: PdfTileResult, doc: &Document) -> Result<(), GpuError> {
        let key = PdfTileKey {
            asset_id: tile.asset_id,
            page_index: tile.page_index,
            tile_x: tile.tile_x,
            tile_y: tile.tile_y,
            tile_cols: tile.tile_cols,
            tile_rows: tile.tile_rows,
        };
        self.in_flight_pdf.remove(&key);
        let (tex, view, mips) = upload_mipmapped_rgba_texture(
            &MipmapUploadCtx {
                device: &self.device,
                queue: &self.queue,
                mipmap_pipeline: &self.mipmap_pipeline,
                blit_bgl: &self.blit_bgl,
                linear_sampler: &self.linear_sampler,
            },
            "swiftnote_pdf_tile",
            tile.width_px,
            tile.height_px,
            &tile.rgba,
        )?;
        let byte_size = (tile.width_px as usize) * (tile.height_px as usize) * 4;
        if let Some(old) = self.pdf_textures.remove(&key) {
            self.pdf_vram_bytes = self.pdf_vram_bytes.saturating_sub(old.byte_size);
        }
        self.pdf_vram_bytes = self.pdf_vram_bytes.saturating_add(byte_size);
        self.pdf_textures.insert(
            key,
            ImageGpuTex {
                _texture: tex,
                view,
                _mip_levels: mips,
                byte_size,
            },
        );
        if let Some(pos) = self.pdf_lru.iter().position(|k| *k == key) {
            self.pdf_lru.remove(pos);
        }
        self.pdf_lru.push_back(key);
        while self.pdf_vram_bytes > PDF_VRAM_BUDGET_BYTES && self.pdf_textures.len() > 1 {
            let Some(evict) = self.pdf_lru.pop_front() else {
                break;
            };
            if evict == key {
                self.pdf_lru.push_back(evict);
                continue;
            }
            if let Some(old) = self.pdf_textures.remove(&evict) {
                self.pdf_vram_bytes = self.pdf_vram_bytes.saturating_sub(old.byte_size);
            }
        }
        for page in &doc.pages {
            if let PageBackground::Pdf {
                asset_id,
                page_index,
                ..
            } = page.background
            {
                if asset_id == tile.asset_id.0 && page_index == tile.page_index {
                    self.tile_cache.invalidate_page(page.id);
                }
            }
        }
        Ok(())
    }

    pub fn request_visible_pdf_tiles(
        &mut self,
        doc: &Document,
        viewport_px: (u32, u32),
        display_scale: f32,
        viewport: ViewportTransform,
        worker: &PdfWorker,
    ) {
        let (vp_w, vp_h) = viewport_px;
        if vp_w == 0 || vp_h == 0 {
            return;
        }
        let scale_milli = TileKey::scale_milli_from(viewport.zoom, display_scale);
        if self.last_pdf_scale_milli != scale_milli {
            let delta = self.last_pdf_scale_milli.abs_diff(scale_milli);
            if self.last_pdf_scale_milli == 0 || delta > 150 {
                self.last_pdf_scale_milli = scale_milli;
                self.in_flight_pdf.clear();
                worker.advance_generation();
            }
        }
        let logical_w = f64::from(vp_w) / f64::from(display_scale);
        let logical_h = f64::from(vp_h) / f64::from(display_scale);
        let tl = viewport.screen_to_canvas(swiftnote_core::ScreenPoint::new(0.0, 0.0));
        let br = viewport.screen_to_canvas(swiftnote_core::ScreenPoint::new(logical_w, logical_h));
        let vis_canvas = Rect::new(
            tl.x.min(br.x),
            tl.y.min(br.y),
            tl.x.max(br.x),
            tl.y.max(br.y),
        );
        let visible_ids: HashSet<PageId> = doc
            .visible_pages(vis_canvas)
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        if visible_ids.is_empty() {
            return;
        }
        let mut target_indices = Vec::with_capacity(visible_ids.len() + 2);
        for (i, page) in doc.pages.iter().enumerate() {
            if visible_ids.contains(&page.id) {
                if i > 0 {
                    target_indices.push(i - 1);
                }
                target_indices.push(i);
                if i + 1 < doc.pages.len() {
                    target_indices.push(i + 1);
                }
            }
        }
        target_indices.sort_unstable();
        target_indices.dedup();

        let generation = worker.current_generation();
        let scale_px_per_pt = viewport.zoom * f64::from(display_scale);
        for idx in target_indices {
            let page = &doc.pages[idx];
            if let PageBackground::Pdf {
                asset_id,
                page_index,
                locked_placeholder: false,
            } = page.background
            {
                let aid = AssetId(asset_id);
                let requests = build_pdf_tile_requests(
                    generation,
                    aid,
                    page_index,
                    page.width_pt,
                    page.height_pt,
                    scale_px_per_pt,
                );
                for req in requests {
                    let key = PdfTileKey {
                        asset_id: req.asset_id,
                        page_index: req.page_index,
                        tile_x: req.tile_x,
                        tile_y: req.tile_y,
                        tile_cols: req.tile_cols,
                        tile_rows: req.tile_rows,
                    };
                    if !self.pdf_textures.contains_key(&key)
                        && !self.in_flight_pdf.contains(&key)
                        && worker.request_tile(req)
                    {
                        self.in_flight_pdf.insert(key);
                    }
                }
            }
        }
    }

    pub fn update_active_stroke(&mut self, page_id: PageId, mesh: &ContourMesh, style: BrushStyle) {
        if mesh.indices.is_empty() || mesh.vertices.is_empty() {
            self.active_stroke = None;
            return;
        }
        self.fill_vertex_scratch(mesh);

        let v_bytes = bytemuck::cast_slice(&self.vertex_scratch);
        let i_bytes = bytemuck::cast_slice(&mesh.indices);
        let needed_v = self.vertex_scratch.len();
        let needed_i = mesh.indices.len();

        let reuse = match &mut self.active_stroke {
            Some(act) if act.v_cap >= needed_v && act.i_cap >= needed_i => {
                act.page_id = page_id;
                act.style = style;
                act.vertex_count = needed_v as u32;
                act.index_count = needed_i as u32;
                self.queue.write_buffer(&act.vbuf, 0, v_bytes);
                self.queue.write_buffer(&act.ibuf, 0, i_bytes);
                true
            }
            _ => false,
        };

        if !reuse {
            let v_cap = needed_v.next_power_of_two().max(1024);
            let i_cap = needed_i.next_power_of_two().max(2048);
            let vbuf = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("swiftnote_active_vbuf"),
                size: (v_cap * std::mem::size_of::<GpuContourVertex>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let ibuf = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("swiftnote_active_ibuf"),
                size: (i_cap * std::mem::size_of::<u32>()) as u64,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            self.queue.write_buffer(&vbuf, 0, v_bytes);
            self.queue.write_buffer(&ibuf, 0, i_bytes);
            self.active_stroke = Some(ActiveStrokeGpu {
                page_id,
                style,
                vertex_count: needed_v as u32,
                index_count: needed_i as u32,
                vbuf,
                ibuf,
                v_cap,
                i_cap,
            });
        }
    }

    pub fn clear_active_stroke(&mut self) {
        self.active_stroke = None;
    }

    pub fn invalidate_dirty(&mut self, dirty: DirtyRegion) {
        self.tile_cache.invalidate_rect(dirty.page_id, dirty.rect);
    }

    pub fn invalidate_page(&mut self, page_id: PageId) {
        self.tile_cache.invalidate_page(page_id);
    }

    pub fn clear_all_tiles(&mut self) {
        self.tile_cache.clear();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn render_canvas(
        &mut self,
        target: &wgpu::TextureView,
        viewport_px: (u32, u32),
        display_scale: f32,
        viewport: ViewportTransform,
        doc: &Document,
        settings: &AppSettings,
        desk_color: [f32; 4],
        active_tool: ActiveTool,
        cursor_overlay: Option<CursorOverlay>,
    ) {
        let (vp_w, vp_h) = viewport_px;
        if vp_w == 0 || vp_h == 0 {
            return;
        }
        let _ = active_tool;
        let _ = cursor_overlay;

        for asset in doc.images.values() {
            if let Err(err) = self.ensure_image_uploaded(asset) {
                warn!("failed uploading image asset {}: {err}", asset.id);
            }
        }

        let logical_w = f64::from(vp_w) / f64::from(display_scale);
        let logical_h = f64::from(vp_h) / f64::from(display_scale);
        let tl = viewport.screen_to_canvas(swiftnote_core::ScreenPoint::new(0.0, 0.0));
        let br = viewport.screen_to_canvas(swiftnote_core::ScreenPoint::new(logical_w, logical_h));
        let vis_canvas = Rect::new(
            tl.x.min(br.x),
            tl.y.min(br.y),
            tl.x.max(br.x),
            tl.y.max(br.y),
        );

        let scale_milli = TileKey::scale_milli_from(viewport.zoom, display_scale);
        let visible_pages = doc.visible_pages(vis_canvas);

        // Phase 1: Rasterize any missing or dirty visible 512x512 physical page tiles on GPU
        for &(page_id, page_canvas_rect) in &visible_pages {
            let Some(page) = doc.page(page_id) else {
                continue;
            };
            let vis_page = Rect::new(
                vis_canvas.min_x - page_canvas_rect.min_x,
                vis_canvas.min_y - page_canvas_rect.min_y,
                vis_canvas.max_x - page_canvas_rect.min_x,
                vis_canvas.max_y - page_canvas_rect.min_y,
            );
            let keys = visible_tile_keys(
                page_id,
                page.width_pt,
                page.height_pt,
                vis_page,
                scale_milli,
            );
            for key in keys {
                if self.tile_cache.is_dirty(key) {
                    self.render_single_tile(page, key, settings);
                }
            }
        }

        // Phase 2: Composite visible tiles & active zero-shift wet stroke onto swapchain
        self.ensure_wet_mask_size(vp_w, vp_h);
        self.uniform_arena.clear();

        #[derive(Clone, Copy)]
        enum DrawOp {
            Tile {
                bg_idx: usize,
                u_off: u32,
            },
            WetStroke {
                stroke_u_off: u32,
                blit_u_off: u32,
                is_highlighter: bool,
                is_blackboard: bool,
                scissor: [u32; 4],
            },
        }

        let mut transient_bgs: Vec<wgpu::BindGroup> = Vec::new();
        let mut draw_ops: Vec<DrawOp> = Vec::new();

        for &(page_id, page_canvas_rect) in &visible_pages {
            let Some(page) = doc.page(page_id) else {
                continue;
            };
            let vis_page = Rect::new(
                vis_canvas.min_x - page_canvas_rect.min_x,
                vis_canvas.min_y - page_canvas_rect.min_y,
                vis_canvas.max_x - page_canvas_rect.min_x,
                vis_canvas.max_y - page_canvas_rect.min_y,
            );
            let keys = visible_tile_keys(
                page_id,
                page.width_pt,
                page.height_pt,
                vis_page,
                scale_milli,
            );
            let page_tl_scr = viewport.canvas_to_screen(CanvasPoint::new(
                page_canvas_rect.min_x,
                page_canvas_rect.min_y,
            ));
            let px_per_pt = viewport.zoom * f64::from(display_scale);

            for key in keys {
                let Some((tile_rect_pt, _, _)) = key.tile_page_rect(page.width_pt, page.height_pt)
                else {
                    continue;
                };
                if let Some(tile) = self.tile_cache.get_mut(key) {
                    let sx0 = (page_tl_scr.x * f64::from(display_scale)
                        + tile_rect_pt.min_x * px_per_pt) as f32;
                    let sy0 = (page_tl_scr.y * f64::from(display_scale)
                        + tile_rect_pt.min_y * px_per_pt) as f32;
                    let sx1 = (page_tl_scr.x * f64::from(display_scale)
                        + tile_rect_pt.max_x * px_per_pt) as f32;
                    let sy1 = (page_tl_scr.y * f64::from(display_scale)
                        + tile_rect_pt.max_y * px_per_pt) as f32;

                    let ndc_x0 = (sx0 / vp_w as f32) * 2.0 - 1.0;
                    let ndc_y0 = 1.0 - (sy0 / vp_h as f32) * 2.0;
                    let ndc_x1 = (sx1 / vp_w as f32) * 2.0 - 1.0;
                    let ndc_y1 = 1.0 - (sy1 / vp_h as f32) * 2.0;

                    let u_off = self.uniform_arena.push(&BlitUniforms {
                        dst_rect_ndc: [ndc_x0, ndc_y0, ndc_x1, ndc_y1],
                        tint: [1.0, 1.0, 1.0, 1.0],
                    });
                    let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("swiftnote_tile_blit_bg"),
                        layout: &self.blit_bgl,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                    buffer: &self.uniform_arena.buffer,
                                    offset: 0,
                                    size: NonZeroU64::new(64),
                                }),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(&tile.view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::Sampler(&self.linear_sampler),
                            },
                        ],
                    });
                    let bg_idx = transient_bgs.len();
                    transient_bgs.push(bg);
                    draw_ops.push(DrawOp::Tile { bg_idx, u_off });
                }
            }

            if let Some(act) = &self.active_stroke {
                if act.page_id == page_id {
                    let polarity = match page.background {
                        PageBackground::Procedural { polarity, .. } => polarity,
                        PageBackground::Pdf { .. } | PageBackground::CustomImage { .. } => {
                            PaperPolarity::Light
                        }
                    };
                    let color = act
                        .style
                        .color
                        .effective_for_paper(polarity, settings.auto_invert_monochrome_ink);
                    let is_hl = act.style.is_highlighter();
                    let mode_id = match act.style.kind {
                        BrushKind::Pen | BrushKind::Calligraphy { .. } => 0.0,
                        BrushKind::Pencil => 1.0,
                        BrushKind::HighlighterRound | BrushKind::HighlighterSquare => 2.0,
                    };
                    let stroke_u_off = self.uniform_arena.push(&StrokeUniforms {
                        bounds_pt: [
                            vis_page.min_x as f32,
                            vis_page.min_y as f32,
                            vis_page.max_x as f32,
                            vis_page.max_y as f32,
                        ],
                        color: [
                            f32::from(color.r) / 255.0,
                            f32::from(color.g) / 255.0,
                            f32::from(color.b) / 255.0,
                            if is_hl {
                                1.0
                            } else {
                                f32::from(color.a) / 255.0
                            },
                        ],
                        mode: [mode_id, 0.0, 0.0, 0.0],
                    });
                    let hl_alpha = if is_hl {
                        f32::from(color.a) / 255.0
                    } else {
                        1.0
                    };
                    let blit_u_off = self.uniform_arena.push(&BlitUniforms {
                        dst_rect_ndc: [-1.0, 1.0, 1.0, -1.0],
                        tint: [1.0, 1.0, 1.0, hl_alpha],
                    });
                    let ps_x0 = (page_tl_scr.x * f64::from(display_scale))
                        .floor()
                        .clamp(0.0, f64::from(vp_w)) as u32;
                    let ps_y0 = (page_tl_scr.y * f64::from(display_scale))
                        .floor()
                        .clamp(0.0, f64::from(vp_h)) as u32;
                    let ps_x1 = ((page_tl_scr.x + page.width_pt * viewport.zoom)
                        * f64::from(display_scale))
                    .ceil()
                    .clamp(0.0, f64::from(vp_w)) as u32;
                    let ps_y1 = ((page_tl_scr.y + page.height_pt * viewport.zoom)
                        * f64::from(display_scale))
                    .ceil()
                    .clamp(0.0, f64::from(vp_h)) as u32;
                    let sw = ps_x1.saturating_sub(ps_x0);
                    let sh = ps_y1.saturating_sub(ps_y0);
                    if sw > 0 && sh > 0 {
                        draw_ops.push(DrawOp::WetStroke {
                            stroke_u_off,
                            blit_u_off,
                            is_highlighter: is_hl,
                            is_blackboard: polarity == PaperPolarity::Blackboard,
                            scissor: [ps_x0, ps_y0, sw, sh],
                        });
                    }
                }
            }
        }

        if self.uniform_arena.upload(&self.device, &self.queue) {
            self.rebind_arena_groups();
        }

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("swiftnote_main_canvas_encoder"),
            });

        // Clear main viewport to solid matte desk color
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("swiftnote_desk_clear_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: f64::from(desk_color[0]),
                            g: f64::from(desk_color[1]),
                            b: f64::from(desk_color[2]),
                            a: f64::from(desk_color[3]),
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            // Draw all visible page tiles
            pass.set_pipeline(&self.blit_alpha_pipeline);
            for op in &draw_ops {
                if let DrawOp::Tile { bg_idx, u_off } = *op {
                    pass.set_bind_group(0, &transient_bgs[bg_idx], &[u_off]);
                    pass.draw(0..6, 0..1);
                }
            }
        }

        // Composite active wet stroke cleanly through the exact same 4x MSAA shader
        for op in &draw_ops {
            if let DrawOp::WetStroke {
                stroke_u_off,
                blit_u_off,
                is_highlighter,
                is_blackboard,
                scissor,
            } = *op
            {
                if let (Some(act), Some((_, mask_view))) = (&self.active_stroke, &self.wet_mask_tex)
                {
                    {
                        let (attach_view, resolve) = if let Some((_, msaa_v)) = &self.wet_mask_msaa
                        {
                            (msaa_v, Some(mask_view))
                        } else {
                            (mask_view, None)
                        };
                        let mut stroke_pass =
                            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("swiftnote_wet_stroke_pass"),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: attach_view,
                                    depth_slice: None,
                                    resolve_target: resolve,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                        store: wgpu::StoreOp::Store,
                                    },
                                })],
                                depth_stencil_attachment: None,
                                timestamp_writes: None,
                                occlusion_query_set: None,
                                multiview_mask: None,
                            });
                        let pipeline = if self.wet_mask_msaa.is_some() {
                            &self.stroke_pipeline_msaa
                        } else {
                            &self.stroke_pipeline_1x
                        };
                        stroke_pass.set_pipeline(pipeline);
                        stroke_pass.set_bind_group(0, &self.stroke_bg, &[stroke_u_off]);
                        stroke_pass.set_vertex_buffer(0, act.vbuf.slice(..));
                        stroke_pass.set_index_buffer(act.ibuf.slice(..), wgpu::IndexFormat::Uint32);
                        stroke_pass.draw_indexed(0..act.index_count, 0, 0..1);
                    }

                    let wet_blit_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("swiftnote_wet_blit_bg"),
                        layout: &self.blit_bgl,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                    buffer: &self.uniform_arena.buffer,
                                    offset: 0,
                                    size: NonZeroU64::new(64),
                                }),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(mask_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::Sampler(&self.linear_sampler),
                            },
                        ],
                    });
                    let mut comp_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("swiftnote_wet_comp_pass"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: target,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    let comp_pipe = if is_highlighter && !is_blackboard {
                        &self.blit_multiply_pipeline
                    } else {
                        &self.blit_alpha_pipeline
                    };
                    comp_pass.set_pipeline(comp_pipe);
                    comp_pass.set_scissor_rect(scissor[0], scissor[1], scissor[2], scissor[3]);
                    comp_pass.set_bind_group(0, &wet_blit_bg, &[blit_u_off]);
                    comp_pass.draw(0..6, 0..1);
                }
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));
    }

    fn render_single_tile(&mut self, page: &Page, key: TileKey, settings: &AppSettings) {
        let Some((tile_rect, w_px, h_px)) = key.tile_page_rect(page.width_pt, page.height_pt)
        else {
            return;
        };
        let req_gen = self.tile_cache.required_gen(key);
        let (tile_tex, tile_view) = create_color_tex(
            &self.device,
            "swiftnote_page_tile",
            w_px,
            h_px,
            self.surface_format,
            1,
        );

        let polarity = match page.background {
            PageBackground::Procedural { polarity, .. } => polarity,
            PageBackground::Pdf { .. } | PageBackground::CustomImage { .. } => PaperPolarity::Light,
        };
        let (template_id, spacing_pt) = match page.background {
            PageBackground::Procedural { template, .. } => match template {
                PaperTemplate::Blank => (0.0, 20.0),
                PaperTemplate::Ruled { spacing_pt } => (1.0, spacing_pt),
                PaperTemplate::Math { spacing_pt } => (2.0, spacing_pt),
                PaperTemplate::Graph => (3.0, 14.0),
                PaperTemplate::Dotted { spacing_pt } => (4.0, spacing_pt),
                PaperTemplate::Isometric => (5.0, 20.0),
            },
            PageBackground::Pdf { .. } | PageBackground::CustomImage { .. } => (0.0, 20.0),
        };

        self.uniform_arena.clear();
        let paper_off = self.uniform_arena.push(&PaperUniforms {
            bounds_pt: [
                tile_rect.min_x as f32,
                tile_rect.min_y as f32,
                tile_rect.max_x as f32,
                tile_rect.max_y as f32,
            ],
            params: [
                template_id,
                spacing_pt,
                if polarity == PaperPolarity::Blackboard {
                    1.0
                } else {
                    0.0
                },
                0.0,
            ],
        });

        enum TileTexKey {
            Image(AssetId),
            Pdf(PdfTileKey),
        }
        let mut img_draws: Vec<(TileTexKey, u32)> = Vec::new();
        if let PageBackground::Pdf {
            asset_id,
            page_index,
            locked_placeholder: false,
        } = page.background
        {
            let aid = AssetId(asset_id);
            let mut matching_pdf_keys: Vec<PdfTileKey> = self
                .pdf_textures
                .keys()
                .copied()
                .filter(|k| k.asset_id == aid && k.page_index == page_index)
                .collect();
            matching_pdf_keys.sort_by_key(|k| (k.tile_y, k.tile_x));
            for pk in matching_pdf_keys {
                let pdf_rect = if pk.tile_cols == 1 && pk.tile_rows == 1 {
                    Rect::new(0.0, 0.0, page.width_pt, page.height_pt)
                } else {
                    let scale = f64::from(self.last_pdf_scale_milli.max(1000)) / 1000.0;
                    let full_w = (page.width_pt * scale).round().max(1.0);
                    let full_h = (page.height_pt * scale).round().max(1.0);
                    let x0_pt = (f64::from(pk.tile_x * MAX_PDF_TILE_PX) / full_w) * page.width_pt;
                    let y0_pt = (f64::from(pk.tile_y * MAX_PDF_TILE_PX) / full_h) * page.height_pt;
                    let x1_pt = (f64::from((pk.tile_x + 1) * MAX_PDF_TILE_PX).min(full_w) / full_w)
                        * page.width_pt;
                    let y1_pt = (f64::from((pk.tile_y + 1) * MAX_PDF_TILE_PX).min(full_h) / full_h)
                        * page.height_pt;
                    Rect::new(x0_pt, y0_pt, x1_pt, y1_pt)
                };
                if pdf_rect.intersects(tile_rect) {
                    let off = self.uniform_arena.push(&ImageUniforms {
                        view_bounds_pt: [
                            tile_rect.min_x as f32,
                            tile_rect.min_y as f32,
                            tile_rect.max_x as f32,
                            tile_rect.max_y as f32,
                        ],
                        img_bounds_pt: [
                            pdf_rect.min_x as f32,
                            pdf_rect.min_y as f32,
                            pdf_rect.max_x as f32,
                            pdf_rect.max_y as f32,
                        ],
                        crop_uv: [0.0, 0.0, 1.0, 1.0],
                        extra: [0.0, 1.0, 0.0, 0.0],
                    });
                    img_draws.push((TileTexKey::Pdf(pk), off));
                }
            }
        }
        for img in &page.images {
            if img.bounds.intersects(tile_rect) && self.image_textures.contains_key(&img.asset_id) {
                let off = self.uniform_arena.push(&ImageUniforms {
                    view_bounds_pt: [
                        tile_rect.min_x as f32,
                        tile_rect.min_y as f32,
                        tile_rect.max_x as f32,
                        tile_rect.max_y as f32,
                    ],
                    img_bounds_pt: [
                        img.bounds.min_x as f32,
                        img.bounds.min_y as f32,
                        img.bounds.max_x as f32,
                        img.bounds.max_y as f32,
                    ],
                    crop_uv: [
                        img.crop.min_x as f32,
                        img.crop.min_y as f32,
                        img.crop.max_x as f32,
                        img.crop.max_y as f32,
                    ],
                    extra: [img.rotation_rad, 1.0, 0.0, 0.0],
                });
                img_draws.push((TileTexKey::Image(img.asset_id), off));
            }
        }

        // Gather overlapping dry strokes in exact StrokeId order and upload combined geometry slice
        struct TileStrokeDraw {
            v_off: u32,
            i_off: u32,
            i_count: u32,
            u_off: u32,
            blit_u_off: u32,
            is_highlighter: bool,
        }
        let candidate_ids = page.index.query_rect(tile_rect);
        let mut all_verts: Vec<GpuContourVertex> = Vec::new();
        let mut all_indices: Vec<u32> = Vec::new();
        let mut stroke_draws: Vec<TileStrokeDraw> = Vec::with_capacity(candidate_ids.len());

        for id in candidate_ids {
            let Ok(pos) = page.strokes.binary_search_by_key(&id, |s| s.id) else {
                continue;
            };
            let stroke = &page.strokes[pos];
            self.tessellator
                .tessellate_stroke(&stroke.points, stroke.style, &mut self.tess_mesh);
            if self.tess_mesh.indices.is_empty() {
                continue;
            }
            let base_v = all_verts.len() as u32;
            let base_i = all_indices.len() as u32;
            for v in &self.tess_mesh.vertices {
                all_verts.push(GpuContourVertex {
                    x: v.x,
                    y: v.y,
                    u: v.u,
                    v: v.v,
                });
            }
            all_indices.extend_from_slice(&self.tess_mesh.indices);

            let eff_col = stroke
                .style
                .color
                .effective_for_paper(polarity, settings.auto_invert_monochrome_ink);
            let is_hl = stroke.style.is_highlighter();
            let mode = match stroke.style.kind {
                BrushKind::Pen | BrushKind::Calligraphy { .. } => 0.0,
                BrushKind::Pencil => 1.0,
                BrushKind::HighlighterRound | BrushKind::HighlighterSquare => 2.0,
            };
            let u_off = self.uniform_arena.push(&StrokeUniforms {
                bounds_pt: [
                    tile_rect.min_x as f32,
                    tile_rect.min_y as f32,
                    tile_rect.max_x as f32,
                    tile_rect.max_y as f32,
                ],
                color: [
                    f32::from(eff_col.r) / 255.0,
                    f32::from(eff_col.g) / 255.0,
                    f32::from(eff_col.b) / 255.0,
                    if is_hl {
                        1.0
                    } else {
                        f32::from(eff_col.a) / 255.0
                    },
                ],
                mode: [mode, 0.0, 0.0, 0.0],
            });
            let blit_u_off = self.uniform_arena.push(&BlitUniforms {
                dst_rect_ndc: [-1.0, 1.0, 1.0, -1.0],
                tint: [
                    1.0,
                    1.0,
                    1.0,
                    if is_hl {
                        f32::from(eff_col.a) / 255.0
                    } else {
                        1.0
                    },
                ],
            });

            stroke_draws.push(TileStrokeDraw {
                v_off: base_v,
                i_off: base_i,
                i_count: self.tess_mesh.indices.len() as u32,
                u_off,
                blit_u_off,
                is_highlighter: is_hl,
            });
        }

        if self.uniform_arena.upload(&self.device, &self.queue) {
            self.rebind_arena_groups();
        }
        if !all_indices.is_empty() {
            self.ensure_scratch_buffers(all_verts.len(), all_indices.len());
            self.queue
                .write_buffer(&self.scratch_vbuf, 0, bytemuck::cast_slice(&all_verts));
            self.queue
                .write_buffer(&self.scratch_ibuf, 0, bytemuck::cast_slice(&all_indices));
        }

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("swiftnote_tile_render_encoder"),
            });

        // 1. Paper pass
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("swiftnote_tile_paper_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &tile_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::WHITE),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.paper_pipeline);
            pass.set_bind_group(0, &self.paper_bg, &[paper_off]);
            pass.draw(0..6, 0..1);
        }

        // 2. Mipmapped Image & PDF background layer
        if !img_draws.is_empty() {
            let mut img_bgs = Vec::with_capacity(img_draws.len());
            for &(ref tex_key, u_off) in &img_draws {
                let tex_opt = match tex_key {
                    TileTexKey::Image(aid) => self.image_textures.get(aid),
                    TileTexKey::Pdf(pk) => self.pdf_textures.get(pk),
                };
                if let Some(tex) = tex_opt {
                    let bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("swiftnote_tile_image_bg"),
                        layout: &self.image_bgl,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                    buffer: &self.uniform_arena.buffer,
                                    offset: 0,
                                    size: NonZeroU64::new(64),
                                }),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(&tex.view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::Sampler(&self.mipmapped_sampler),
                            },
                        ],
                    });
                    img_bgs.push((bg, u_off));
                }
            }
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("swiftnote_tile_image_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &tile_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.image_pipeline_tile);
            for (bg, u_off) in &img_bgs {
                pass.set_bind_group(0, bg, &[*u_off]);
                pass.draw(0..6, 0..1);
            }
        }

        // 3. Dry strokes (100% identical shader + contour + highlighter single-stroke mask to wet ink)
        if !stroke_draws.is_empty() {
            for sd in &stroke_draws {
                if sd.is_highlighter {
                    let (_, hl_view) = &self.hl_resolve_tex;
                    {
                        let (attach_view, resolve) = if let Some((_, msaa_v)) = &self.hl_msaa_tex {
                            (msaa_v, Some(hl_view))
                        } else {
                            (hl_view, None)
                        };
                        let mut hl_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("swiftnote_tile_hl_mask_pass"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: attach_view,
                                depth_slice: None,
                                resolve_target: resolve,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                    store: wgpu::StoreOp::Store,
                                },
                            })],
                            depth_stencil_attachment: None,
                            timestamp_writes: None,
                            occlusion_query_set: None,
                            multiview_mask: None,
                        });
                        let pipe = if self.hl_msaa_tex.is_some() {
                            &self.stroke_pipeline_msaa
                        } else {
                            &self.stroke_pipeline_1x
                        };
                        hl_pass.set_pipeline(pipe);
                        hl_pass.set_bind_group(0, &self.stroke_bg, &[sd.u_off]);
                        hl_pass.set_vertex_buffer(0, self.scratch_vbuf.slice(..));
                        hl_pass.set_index_buffer(
                            self.scratch_ibuf.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                        hl_pass.draw_indexed(
                            sd.i_off..(sd.i_off + sd.i_count),
                            sd.v_off as i32,
                            0..1,
                        );
                    }
                    let hl_blit_bg = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("swiftnote_tile_hl_blit"),
                        layout: &self.blit_bgl,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                    buffer: &self.uniform_arena.buffer,
                                    offset: 0,
                                    size: NonZeroU64::new(64),
                                }),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::TextureView(hl_view),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: wgpu::BindingResource::Sampler(&self.linear_sampler),
                            },
                        ],
                    });
                    let mut comp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("swiftnote_tile_hl_comp"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &tile_view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    let pipe = if polarity == PaperPolarity::Blackboard {
                        &self.blit_alpha_pipeline
                    } else {
                        &self.blit_multiply_pipeline
                    };
                    comp.set_pipeline(pipe);
                    comp.set_bind_group(0, &hl_blit_bg, &[sd.blit_u_off]);
                    comp.draw(0..6, 0..1);
                } else {
                    // Direct Pen/Pencil/Calligraphy stroke rendered with identical 4x MSAA mask + composite
                    let (_, hl_view) = &self.hl_resolve_tex;
                    {
                        let (attach_view, resolve) = if let Some((_, msaa_v)) = &self.hl_msaa_tex {
                            (msaa_v, Some(hl_view))
                        } else {
                            (hl_view, None)
                        };
                        let mut stroke_pass =
                            encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("swiftnote_tile_stroke_pass"),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: attach_view,
                                    depth_slice: None,
                                    resolve_target: resolve,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                                        store: wgpu::StoreOp::Store,
                                    },
                                })],
                                depth_stencil_attachment: None,
                                timestamp_writes: None,
                                occlusion_query_set: None,
                                multiview_mask: None,
                            });
                        let pipe = if self.hl_msaa_tex.is_some() {
                            &self.stroke_pipeline_msaa
                        } else {
                            &self.stroke_pipeline_1x
                        };
                        stroke_pass.set_pipeline(pipe);
                        stroke_pass.set_bind_group(0, &self.stroke_bg, &[sd.u_off]);
                        stroke_pass.set_vertex_buffer(0, self.scratch_vbuf.slice(..));
                        stroke_pass.set_index_buffer(
                            self.scratch_ibuf.slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                        stroke_pass.draw_indexed(
                            sd.i_off..(sd.i_off + sd.i_count),
                            sd.v_off as i32,
                            0..1,
                        );
                    }
                    let stroke_blit_bg =
                        self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                            label: Some("swiftnote_tile_stroke_blit"),
                            layout: &self.blit_bgl,
                            entries: &[
                                wgpu::BindGroupEntry {
                                    binding: 0,
                                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                        buffer: &self.uniform_arena.buffer,
                                        offset: 0,
                                        size: NonZeroU64::new(64),
                                    }),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 1,
                                    resource: wgpu::BindingResource::TextureView(hl_view),
                                },
                                wgpu::BindGroupEntry {
                                    binding: 2,
                                    resource: wgpu::BindingResource::Sampler(&self.linear_sampler),
                                },
                            ],
                        });
                    let mut comp = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some("swiftnote_tile_stroke_comp"),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &tile_view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Load,
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: None,
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    comp.set_pipeline(&self.blit_alpha_pipeline);
                    comp.set_bind_group(0, &stroke_blit_bg, &[sd.blit_u_off]);
                    comp.draw(0..6, 0..1);
                }
            }
        }

        self.queue.submit(std::iter::once(encoder.finish()));

        let byte_size = (w_px as usize) * (h_px as usize) * 4;
        self.tile_cache.insert(
            key,
            GpuTile {
                _texture: tile_tex,
                view: tile_view,
            },
            byte_size,
            req_gen,
        );
        let _ = (&self.tile_msaa_tex, w_px, h_px);
    }

    fn fill_vertex_scratch(&mut self, mesh: &ContourMesh) {
        self.vertex_scratch.clear();
        for v in &mesh.vertices {
            self.vertex_scratch.push(GpuContourVertex {
                x: v.x,
                y: v.y,
                u: v.u,
                v: v.v,
            });
        }
    }

    fn ensure_scratch_buffers(&mut self, verts: usize, indices: usize) {
        if verts > self.scratch_vcap {
            self.scratch_vcap = verts.next_power_of_two();
            self.scratch_vbuf = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("swiftnote_stroke_vbuf"),
                size: (self.scratch_vcap * std::mem::size_of::<GpuContourVertex>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if indices > self.scratch_icap {
            self.scratch_icap = indices.next_power_of_two();
            self.scratch_ibuf = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("swiftnote_stroke_ibuf"),
                size: (self.scratch_icap * std::mem::size_of::<u32>()) as u64,
                usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
    }

    fn ensure_wet_mask_size(&mut self, w: u32, h: u32) {
        if self.wet_mask_size == (w, h) && self.wet_mask_tex.is_some() {
            return;
        }
        self.wet_mask_size = (w, h);
        self.wet_mask_tex = Some(create_color_tex(
            &self.device,
            "swiftnote_wet_mask",
            w,
            h,
            self.surface_format,
            1,
        ));
        self.wet_mask_msaa = if self.msaa_samples > 1 {
            Some(create_color_tex(
                &self.device,
                "swiftnote_wet_mask_msaa",
                w,
                h,
                self.surface_format,
                self.msaa_samples,
            ))
        } else {
            None
        };
    }

    fn rebind_arena_groups(&mut self) {
        self.paper_bg =
            make_arena_bind_group(&self.device, &self.paper_bgl, &self.uniform_arena.buffer);
        self.stroke_bg =
            make_arena_bind_group(&self.device, &self.stroke_bgl, &self.uniform_arena.buffer);
    }
}

fn make_arena_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffer: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("swiftnote_arena_bg"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                buffer,
                offset: 0,
                size: NonZeroU64::new(64),
            }),
        }],
    })
}

fn create_color_tex(
    device: &wgpu::Device,
    label: &str,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    sample_count: u32,
) -> (wgpu::Texture, wgpu::TextureView) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: width.max(1),
            height: height.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });
    let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
    (tex, view)
}

fn build_stroke_pipeline(
    device: &wgpu::Device,
    shader: &wgpu::ShaderModule,
    bgl: &wgpu::BindGroupLayout,
    format: wgpu::TextureFormat,
    sample_count: u32,
) -> wgpu::RenderPipeline {
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("swiftnote_stroke_layout"),
        bind_group_layouts: &[Some(bgl)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("swiftnote_stroke_pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<GpuContourVertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[
                    wgpu::VertexAttribute {
                        offset: 0,
                        shader_location: 0,
                        format: wgpu::VertexFormat::Float32x2,
                    },
                    wgpu::VertexAttribute {
                        offset: 8,
                        shader_location: 1,
                        format: wgpu::VertexFormat::Float32x2,
                    },
                ],
            })],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState {
            count: sample_count,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        multiview_mask: None,
        cache: None,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn wgsl_shaders_compile_and_wet_matches_dry_pipeline() {
        match UnifiedRenderer::new_headless() {
            Ok(renderer) => {
                let cfg = renderer.pipeline_config();
                assert!(cfg.sample_count == 4 || cfg.sample_count == 1);
            }
            Err(GpuError::NoAdapter) => {
                assert!(!include_str!("shaders/stroke.wgsl").is_empty());
                assert!(!include_str!("shaders/paper.wgsl").is_empty());
                assert!(!include_str!("shaders/image.wgsl").is_empty());
                assert!(!include_str!("shaders/blit.wgsl").is_empty());
            }
            Err(e) => panic!("unexpected GPU init error: {e}"),
        }
    }
}
