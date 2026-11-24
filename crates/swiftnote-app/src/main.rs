mod clipboard;
mod controller;
mod icons;
mod input;
mod theme;
mod ui;

use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use egui::{Order, OutputCommand, RawInput, ViewportId, ViewportInfo};
use sdl3::event::{DisplayEvent, Event as SdlEvent, WindowEvent};
use sdl3::rect::Rect as SdlRect;
use sdl3::video::{SystemTheme, Window};
use sdl3::VideoSubsystem;
use swiftnote_core::NoteLibrary;
use swiftnote_gpu::{default_wgpu_backends, UnifiedRenderer};
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use crate::controller::AppController;
use crate::icons::IconRegistry;
use crate::input::SdlInputMapper;
use crate::theme::{apply_matte_theme, resolve_theme, MattePalette};

/// Owns the `wgpu::Surface` strictly before the SDL3 `Window` so Rust drops
/// the swapchain surface before destroying the underlying OS window handle.
struct WindowSurface {
    surface: wgpu::Surface<'static>,
    window: Window,
}

#[derive(Debug, Clone, Copy)]
struct WindowMetrics {
    viewport_px: (u32, u32),
    display_scale: f32,
    coord_scale: f32,
    logical_w: f32,
    logical_h: f32,
}

fn read_window_metrics(window: &Window) -> WindowMetrics {
    let viewport_px = window.size_in_pixels();
    let sdl_win_size = window.size();
    let display_scale = window.display_scale().clamp(1.0, 4.0);
    let pixel_density = if sdl_win_size.0 > 0 && viewport_px.0 > 0 {
        (viewport_px.0 as f32 / sdl_win_size.0 as f32).clamp(0.5, 4.0)
    } else {
        1.0
    };
    let coord_scale = (pixel_density / display_scale).clamp(0.25, 4.0);
    let logical_w = (viewport_px.0.max(1) as f32 / display_scale).max(1.0);
    let logical_h = (viewport_px.1.max(1) as f32 / display_scale).max(1.0);
    WindowMetrics {
        viewport_px,
        display_scale,
        coord_scale,
        logical_w,
        logical_h,
    }
}

fn main() {
    init_logging();
    if let Err(err) = run_desktop_app() {
        error!("{err:#}");
        eprintln!("Swift Note fatal error: {err:#}");
        std::process::exit(1);
    }
}

fn init_logging() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("swiftnote=info,swiftnote_gpu=warn,wgpu=error"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();
}

fn run_desktop_app() -> Result<()> {
    let sdl = sdl3::init().map_err(|e| anyhow!("SDL3 init failed: {e}"))?;
    let video = sdl
        .video()
        .map_err(|e| anyhow!("SDL3 video subsystem init failed: {e}"))?;
    let mut event_pump = sdl
        .event_pump()
        .map_err(|e| anyhow!("SDL3 event pump init failed: {e}"))?;

    let window = video
        .window("Swift Note", 1280, 800)
        .resizable()
        .high_pixel_density()
        .position_centered()
        .build()
        .map_err(|e| anyhow!("Failed creating window: {e}"))?;

    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
        backends: default_wgpu_backends(),
        ..wgpu::InstanceDescriptor::new_without_display_handle()
    });

    // SAFETY: `WindowSurface` declares `surface` above `window`, guaranteeing
    // `wgpu::Surface` is dropped before `sdl3::video::Window` goes out of scope.
    #[allow(unsafe_code)]
    let surface = unsafe {
        let target = wgpu::SurfaceTargetUnsafe::from_display_and_window(&window, &window)
            .map_err(|e| anyhow!("Failed extracting native window/display handles: {e}"))?;
        instance.create_surface_unsafe(target)
    }
    .context("Failed creating wgpu surface for window")?;
    let ws = WindowSurface { surface, window };

    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: Some(&ws.surface),
        force_fallback_adapter: false,
        ..Default::default()
    }))
    .map_err(|err| anyhow!("No compatible GPU adapter found: {err}"))?;

    let limits = wgpu::Limits::downlevel_defaults().or_better_values_from(&adapter.limits());
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("swiftnote_device"),
        required_features: wgpu::Features::empty(),
        required_limits: limits,
        experimental_features: wgpu::ExperimentalFeatures::default(),
        memory_hints: wgpu::MemoryHints::Performance,
        trace: wgpu::Trace::Off,
    }))
    .map_err(|err| anyhow!("Failed creating wgpu device: {err}"))?;
    let device = Arc::new(device);
    let queue = Arc::new(queue);

    let caps = ws.surface.get_capabilities(&adapter);
    let surface_format = caps
        .formats
        .iter()
        .copied()
        .find(wgpu::TextureFormat::is_srgb)
        .or_else(|| caps.formats.first().copied())
        .ok_or_else(|| anyhow!("Surface reported zero supported texture formats"))?;
    let alpha_mode = caps
        .alpha_modes
        .iter()
        .copied()
        .find(|m| *m == wgpu::CompositeAlphaMode::Opaque)
        .or_else(|| caps.alpha_modes.first().copied())
        .unwrap_or(wgpu::CompositeAlphaMode::Auto);

    let mut metrics = read_window_metrics(&ws.window);
    let mut surface_config = wgpu::SurfaceConfiguration {
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
        format: surface_format,
        color_space: wgpu::SurfaceColorSpace::Auto,
        width: metrics.viewport_px.0.max(1),
        height: metrics.viewport_px.1.max(1),
        present_mode: wgpu::PresentMode::AutoVsync,
        desired_maximum_frame_latency: 2,
        alpha_mode,
        view_formats: vec![],
    };
    if metrics.viewport_px.0 > 0 && metrics.viewport_px.1 > 0 {
        ws.surface.configure(&device, &surface_config);
    }

    let adapter_info = adapter.get_info();
    info!(
        adapter = %adapter_info.name,
        backend = ?adapter_info.backend,
        ?surface_format,
        viewport_px = ?metrics.viewport_px,
        display_scale = metrics.display_scale,
        coord_scale = metrics.coord_scale,
        "Swift Note GPU & window initialized"
    );

    let mut renderer = UnifiedRenderer::new(
        Arc::clone(&device),
        Arc::clone(&queue),
        &adapter,
        surface_format,
    )
    .context("Failed initializing unified GPU canvas renderer")?;
    let mut egui_renderer = egui_wgpu::Renderer::new(
        &device,
        surface_format,
        egui_wgpu::RendererOptions::default(),
    );

    let library = match NoteLibrary::open_default() {
        Ok(lib) => Some(lib),
        Err(err) => {
            warn!("Running without persistent library: {err}");
            None
        }
    };
    let mut controller = AppController::new_with_library(library);
    let mut input_mapper = SdlInputMapper::new();
    input_mapper.set_viewport_logical_size(metrics.logical_w, metrics.logical_h);
    input_mapper.set_coord_scale(metrics.coord_scale);
    controller.ensure_initial_viewport_centered(metrics.viewport_px, metrics.display_scale);

    let egui_ctx = egui::Context::default();
    let mut icons = IconRegistry::new(&egui_ctx, metrics.display_scale);

    let mut system_dark = detect_system_dark();
    let mut resolved_theme = resolve_theme(controller.settings.ui_theme, system_dark);
    apply_matte_theme(&egui_ctx, resolved_theme);

    let start_instant = Instant::now();
    let mut running = true;
    let mut needs_redraw = true;
    let mut window_focused = true;
    let mut ime_active = false;
    let mut egui_repaint_deadline: Option<Instant> = None;

    while running {
        if !needs_redraw {
            let wait_ms = compute_wait_timeout_ms(&controller, egui_repaint_deadline);
            if let Some(first_ev) = event_pump.wait_event_timeout_ms(wait_ms) {
                let outcome = handle_event(
                    &first_ev,
                    &ws,
                    &device,
                    &mut surface_config,
                    &mut metrics,
                    &mut icons,
                    &egui_ctx,
                    &mut input_mapper,
                    &mut controller,
                    &mut renderer,
                    &mut window_focused,
                );
                if outcome.quit {
                    break;
                }
                needs_redraw |= outcome.redraw;
            }
        }

        for ev in event_pump.poll_iter() {
            let outcome = handle_event(
                &ev,
                &ws,
                &device,
                &mut surface_config,
                &mut metrics,
                &mut icons,
                &egui_ctx,
                &mut input_mapper,
                &mut controller,
                &mut renderer,
                &mut window_focused,
            );
            if outcome.quit {
                running = false;
                break;
            }
            needs_redraw |= outcome.redraw;
        }
        if !running {
            break;
        }

        system_dark = detect_system_dark();
        let next_theme = resolve_theme(controller.settings.ui_theme, system_dark);
        if next_theme != resolved_theme {
            resolved_theme = next_theme;
            apply_matte_theme(&egui_ctx, resolved_theme);
            needs_redraw = true;
        }

        if controller.tick(Some(&mut renderer)) {
            needs_redraw = true;
        }
        if egui_repaint_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            egui_repaint_deadline = None;
            needs_redraw = true;
        }

        if !needs_redraw || metrics.viewport_px.0 == 0 || metrics.viewport_px.1 == 0 {
            continue;
        }
        needs_redraw = false;

        let frame_ctx = FrameRenderContext {
            ws: &ws,
            video: &video,
            device: &device,
            queue: &queue,
            surface_config: &mut surface_config,
            metrics,
            start_instant,
            window_focused,
            system_dark,
            palette: MattePalette::for_theme(resolved_theme),
        };
        let frame_outcome = render_frame(
            frame_ctx,
            &egui_ctx,
            &icons,
            &mut input_mapper,
            &mut controller,
            &mut renderer,
            &mut egui_renderer,
            &mut ime_active,
        );
        needs_redraw |= frame_outcome.redraw;
        egui_repaint_deadline = frame_outcome.repaint_deadline;

        let post_action_theme = resolve_theme(controller.settings.ui_theme, system_dark);
        if post_action_theme != resolved_theme {
            resolved_theme = post_action_theme;
            apply_matte_theme(&egui_ctx, resolved_theme);
            needs_redraw = true;
        }
    }

    controller.flush_autosave_on_exit();
    Ok(())
}

fn detect_system_dark() -> Option<bool> {
    match VideoSubsystem::get_system_theme() {
        SystemTheme::Dark => Some(true),
        SystemTheme::Light => Some(false),
        SystemTheme::Unknown => None,
    }
}

fn compute_wait_timeout_ms(
    controller: &AppController,
    egui_repaint_deadline: Option<Instant>,
) -> u32 {
    let mut timeout_ms = if controller.has_active_stroke() {
        8_u32
    } else if controller.is_dirty() || controller.toast.is_some() {
        100_u32
    } else {
        250_u32
    };

    if let Some(deadline) = egui_repaint_deadline {
        let now = Instant::now();
        let remaining_ms = deadline
            .saturating_duration_since(now)
            .as_millis()
            .min(u128::from(timeout_ms)) as u32;
        timeout_ms = timeout_ms.min(remaining_ms.max(1));
    }
    timeout_ms
}

struct EventOutcome {
    quit: bool,
    redraw: bool,
}

#[allow(clippy::too_many_arguments)]
fn handle_event(
    event: &SdlEvent,
    ws: &WindowSurface,
    device: &wgpu::Device,
    surface_config: &mut wgpu::SurfaceConfiguration,
    metrics: &mut WindowMetrics,
    icons: &mut IconRegistry,
    egui_ctx: &egui::Context,
    input_mapper: &mut SdlInputMapper,
    controller: &mut AppController,
    renderer: &mut UnifiedRenderer,
    window_focused: &mut bool,
) -> EventOutcome {
    let mut outcome = EventOutcome {
        quit: false,
        redraw: false,
    };

    match event {
        SdlEvent::Quit { .. } => {
            outcome.quit = true;
            return outcome;
        }
        SdlEvent::Window { win_event, .. } => match win_event {
            WindowEvent::CloseRequested => {
                outcome.quit = true;
                return outcome;
            }
            WindowEvent::Resized(_, _)
            | WindowEvent::PixelSizeChanged(_, _)
            | WindowEvent::Exposed
            | WindowEvent::DisplayChanged(_)
            | WindowEvent::ICCProfChanged => {
                sync_window_metrics(
                    ws,
                    device,
                    surface_config,
                    metrics,
                    icons,
                    egui_ctx,
                    input_mapper,
                    controller,
                );
                outcome.redraw = true;
            }
            WindowEvent::FocusGained => {
                *window_focused = true;
                outcome.redraw = true;
            }
            WindowEvent::FocusLost | WindowEvent::Hidden | WindowEvent::Minimized => {
                *window_focused = false;
                outcome.redraw = true;
            }
            _ => {}
        },
        SdlEvent::Display {
            display_event: DisplayEvent::ContentScaleChanged,
            ..
        } => {
            sync_window_metrics(
                ws,
                device,
                surface_config,
                metrics,
                icons,
                egui_ctx,
                input_mapper,
                controller,
            );
            outcome.redraw = true;
        }
        _ => {}
    }

    let direct_ui_hit = input_mapper
        .sdl_pointer_pos_logical(event)
        .is_some_and(|pos| {
            egui_ctx
                .layer_id_at(pos)
                .is_some_and(|layer| layer.order != Order::Background)
        });
    let egui_wants_pointer = egui_ctx.egui_wants_pointer_input() || direct_ui_hit;

    let canvas_events = input_mapper.feed_sdl_event(
        event,
        &controller.settings,
        controller.tool_state.active_tool,
        egui_wants_pointer,
        egui_ctx.egui_wants_keyboard_input(),
    );
    if !canvas_events.is_empty() || input_mapper.has_pending_egui_events() {
        outcome.redraw = true;
    }
    for canvas_ev in canvas_events {
        if controller.handle_canvas_event(
            canvas_ev,
            metrics.viewport_px,
            metrics.display_scale,
            Some(renderer),
        ) {
            outcome.redraw = true;
        }
    }

    outcome
}

#[allow(clippy::too_many_arguments)]
fn sync_window_metrics(
    ws: &WindowSurface,
    device: &wgpu::Device,
    surface_config: &mut wgpu::SurfaceConfiguration,
    metrics: &mut WindowMetrics,
    icons: &mut IconRegistry,
    egui_ctx: &egui::Context,
    input_mapper: &mut SdlInputMapper,
    controller: &mut AppController,
) {
    let next = read_window_metrics(&ws.window);
    let px_changed = next.viewport_px != metrics.viewport_px;
    *metrics = next;

    input_mapper.set_viewport_logical_size(next.logical_w, next.logical_h);
    input_mapper.set_coord_scale(next.coord_scale);
    controller.update_viewport_logical_width(f64::from(next.logical_w));
    controller.ensure_initial_viewport_centered(next.viewport_px, next.display_scale);
    icons.ensure_scale(egui_ctx, next.display_scale);

    if px_changed && next.viewport_px.0 > 0 && next.viewport_px.1 > 0 {
        surface_config.width = next.viewport_px.0;
        surface_config.height = next.viewport_px.1;
        ws.surface.configure(device, surface_config);
    }
}

struct FrameRenderContext<'a> {
    ws: &'a WindowSurface,
    video: &'a VideoSubsystem,
    device: &'a wgpu::Device,
    queue: &'a wgpu::Queue,
    surface_config: &'a mut wgpu::SurfaceConfiguration,
    metrics: WindowMetrics,
    start_instant: Instant,
    window_focused: bool,
    system_dark: Option<bool>,
    palette: MattePalette,
}

struct FrameOutcome {
    redraw: bool,
    repaint_deadline: Option<Instant>,
}

#[allow(clippy::too_many_arguments)]
fn render_frame(
    ctx: FrameRenderContext<'_>,
    egui_ctx: &egui::Context,
    icons: &IconRegistry,
    input_mapper: &mut SdlInputMapper,
    controller: &mut AppController,
    renderer: &mut UnifiedRenderer,
    egui_renderer: &mut egui_wgpu::Renderer,
    ime_active: &mut bool,
) -> FrameOutcome {
    let m = ctx.metrics;

    let mut raw_input = RawInput {
        viewport_id: ViewportId::ROOT,
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(m.logical_w, m.logical_h),
        )),
        time: Some(ctx.start_instant.elapsed().as_secs_f64()),
        events: input_mapper.take_egui_events(),
        focused: ctx.window_focused,
        system_theme: ctx.system_dark.map(|dark| {
            if dark {
                egui::Theme::Dark
            } else {
                egui::Theme::Light
            }
        }),
        ..RawInput::default()
    };
    raw_input.viewports.insert(
        ViewportId::ROOT,
        ViewportInfo {
            native_pixels_per_point: Some(m.display_scale),
            focused: Some(ctx.window_focused),
            ..ViewportInfo::default()
        },
    );

    let mut queued_actions = Vec::new();
    let full_output = egui_ctx.run_ui(raw_input, |ui| {
        queued_actions = ui::build_ui(
            ui,
            controller,
            icons,
            ctx.palette,
            m.viewport_px,
            m.display_scale,
        );
    });

    sync_ime_output(
        ctx.video,
        &ctx.ws.window,
        full_output.platform_output.ime,
        m.coord_scale,
        ime_active,
    );
    handle_platform_commands(&full_output.platform_output.commands);

    let actions_changed = controller.apply_actions(
        queued_actions,
        m.viewport_px,
        m.display_scale,
        Some(renderer),
    );
    controller.request_visible_pdf_tiles(renderer, m.viewport_px, m.display_scale);

    for (tex_id, deltas) in &full_output.textures_delta.set {
        for delta in deltas {
            egui_renderer.update_texture(ctx.device, ctx.queue, *tex_id, delta);
        }
    }

    let mut reconfigure_after_present = false;
    let mut needs_retry_frame = false;
    let frame = match ctx.ws.surface.get_current_texture() {
        wgpu::CurrentSurfaceTexture::Success(frame) => Some(frame),
        wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
            reconfigure_after_present = true;
            Some(frame)
        }
        wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
            ctx.surface_config.width = m.viewport_px.0;
            ctx.surface_config.height = m.viewport_px.1;
            ctx.ws.surface.configure(ctx.device, ctx.surface_config);
            needs_retry_frame = true;
            None
        }
        wgpu::CurrentSurfaceTexture::Timeout
        | wgpu::CurrentSurfaceTexture::Occluded
        | wgpu::CurrentSurfaceTexture::Validation => None,
    };

    if let Some(frame) = frame {
        let frame_view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        renderer.render_canvas(
            &frame_view,
            m.viewport_px,
            m.display_scale,
            controller.viewport,
            &controller.doc,
            &controller.settings,
            ctx.palette.desk_linear_rgba(),
            controller.tool_state.active_tool,
            None,
        );

        let paint_jobs = egui_ctx.tessellate(full_output.shapes, full_output.pixels_per_point);
        let screen_desc = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [m.viewport_px.0, m.viewport_px.1],
            pixels_per_point: full_output.pixels_per_point,
        };
        let mut encoder = ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("swiftnote_egui_encoder"),
            });
        let user_cmds = egui_renderer.update_buffers(
            ctx.device,
            ctx.queue,
            &mut encoder,
            &paint_jobs,
            &screen_desc,
        );
        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("swiftnote_egui_pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &frame_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
                multiview_mask: None,
            });
            egui_renderer.render(&mut pass.forget_lifetime(), &paint_jobs, &screen_desc);
        }

        ctx.queue.submit(
            user_cmds
                .into_iter()
                .chain(std::iter::once(encoder.finish())),
        );
        ctx.queue.present(frame);
    }

    for tex_id in &full_output.textures_delta.free {
        egui_renderer.free_texture(tex_id);
    }

    if reconfigure_after_present && m.viewport_px.0 > 0 && m.viewport_px.1 > 0 {
        ctx.ws.surface.configure(ctx.device, ctx.surface_config);
    }

    let repaint_delay = full_output
        .viewport_output
        .get(&ViewportId::ROOT)
        .map(|vo| vo.repaint_delay)
        .unwrap_or(Duration::MAX);

    let immediate_repaint = actions_changed
        || repaint_delay == Duration::ZERO
        || reconfigure_after_present
        || needs_retry_frame;
    let repaint_deadline = if !immediate_repaint && repaint_delay <= Duration::from_secs(2) {
        Some(Instant::now() + repaint_delay)
    } else {
        None
    };

    FrameOutcome {
        redraw: immediate_repaint,
        repaint_deadline,
    }
}

fn sync_ime_output(
    video: &VideoSubsystem,
    window: &Window,
    ime: Option<egui::output::IMEOutput>,
    coord_scale: f32,
    ime_active: &mut bool,
) {
    let text_input = video.text_input();
    let inv = 1.0 / coord_scale.clamp(0.25, 4.0);
    match ime {
        Some(info) => {
            if !*ime_active {
                text_input.start(window);
                *ime_active = true;
            }
            let x = (info.cursor_rect.min.x * inv).round().max(0.0) as i32;
            let y = (info.cursor_rect.min.y * inv).round().max(0.0) as i32;
            let w = (info.cursor_rect.width() * inv).ceil().max(1.0) as u32;
            let h = (info.cursor_rect.height() * inv).ceil().max(16.0) as u32;
            text_input.set_rect(window, SdlRect::new(x, y, w, h), 0);
        }
        None if *ime_active => {
            text_input.stop(window);
            *ime_active = false;
        }
        None => {}
    }
}

fn handle_platform_commands(commands: &[OutputCommand]) {
    for cmd in commands {
        if let OutputCommand::CopyText(text) = cmd {
            if let Err(err) = clipboard::write_os_clipboard_text(text) {
                warn!("Clipboard copy failed: {err}");
            }
        }
    }
}
