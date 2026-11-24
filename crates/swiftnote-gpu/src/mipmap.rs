use wgpu::util::DeviceExt;

use crate::error::GpuError;

#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct BlitUniforms {
    pub dst_rect_ndc: [f32; 4],
    pub tint: [f32; 4],
    pub src_uv_rect: [f32; 4],
    pub pad: [f32; 4],
}

impl BlitUniforms {
    pub const FULL_UV: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

    pub const fn new(dst_rect_ndc: [f32; 4], tint: [f32; 4], src_uv_rect: [f32; 4]) -> Self {
        Self {
            dst_rect_ndc,
            tint,
            src_uv_rect,
            pad: [0.0; 4],
        }
    }
}

/// Computes full mipmap chain length down to `1x1`.
#[inline]
pub fn compute_mip_level_count(width: u32, height: u32) -> u32 {
    let max_dim = width.max(height).max(1);
    u32::BITS - max_dim.leading_zeros()
}

pub fn create_mipmap_pipeline(
    device: &wgpu::Device,
    format: wgpu::TextureFormat,
    bgl: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("swiftnote_blit_shader"),
        source: wgpu::ShaderSource::Wgsl(include_str!("shaders/blit.wgsl").into()),
    });
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("swiftnote_mipmap_layout"),
        bind_group_layouts: &[Some(bgl)],
        immediate_size: 0,
    });
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("swiftnote_mipmap_pipeline"),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_mipmap"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

pub struct MipmapUploadCtx<'a> {
    pub device: &'a wgpu::Device,
    pub queue: &'a wgpu::Queue,
    pub mipmap_pipeline: &'a wgpu::RenderPipeline,
    pub blit_bgl: &'a wgpu::BindGroupLayout,
    pub linear_sampler: &'a wgpu::Sampler,
}

pub fn upload_mipmapped_rgba_texture(
    ctx: &MipmapUploadCtx<'_>,
    label: &str,
    width: u32,
    height: u32,
    rgba: &[u8],
) -> Result<(wgpu::Texture, wgpu::TextureView, u32), GpuError> {
    let device = ctx.device;
    let queue = ctx.queue;
    let mipmap_pipeline = ctx.mipmap_pipeline;
    let blit_bgl = ctx.blit_bgl;
    let linear_sampler = ctx.linear_sampler;
    if width == 0 || height == 0 {
        return Err(GpuError::ImageDecode("zero-dimension texture".into()));
    }
    let mip_level_count = compute_mip_level_count(width, height);
    let format = wgpu::TextureFormat::Rgba8UnormSrgb;

    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });

    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        rgba,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(4 * width),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );

    if mip_level_count > 1 {
        let uniform_buf = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("swiftnote_mipmap_uniforms"),
            contents: bytemuck::bytes_of(&BlitUniforms::new(
                [-1.0, 1.0, 1.0, -1.0],
                [1.0, 1.0, 1.0, 1.0],
                BlitUniforms::FULL_UV,
            )),
            usage: wgpu::BufferUsages::UNIFORM,
        });

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("swiftnote_mipmap_encoder"),
        });

        for level in 1..mip_level_count {
            let src_view = texture.create_view(&wgpu::TextureViewDescriptor {
                base_mip_level: level - 1,
                mip_level_count: Some(1),
                ..Default::default()
            });
            let dst_view = texture.create_view(&wgpu::TextureViewDescriptor {
                base_mip_level: level,
                mip_level_count: Some(1),
                ..Default::default()
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("swiftnote_mipmap_bg"),
                layout: blit_bgl,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform_buf.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(&src_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::Sampler(linear_sampler),
                    },
                ],
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("swiftnote_mipmap_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &dst_view,
                    depth_slice: None,
                    resolve_target: None,
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
            pass.set_pipeline(mipmap_pipeline);
            pass.set_bind_group(0, &bind_group, &[0]);
            pass.draw(0..6, 0..1);
        }

        queue.submit(std::iter::once(encoder.finish()));
    }

    let full_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    Ok((texture, full_view, mip_level_count))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn mipmap_level_count_computes_full_chain_to_1x1() {
        assert_eq!(compute_mip_level_count(1, 1), 1);
        assert_eq!(compute_mip_level_count(512, 512), 10);
        assert_eq!(compute_mip_level_count(3840, 2160), 12);
    }
}
