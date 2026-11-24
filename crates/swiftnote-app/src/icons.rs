use std::collections::HashMap;

use egui::{ColorImage, TextureHandle, TextureOptions};
use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg;

const BASE_ICON_PT: f32 = 18.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IconId {
    Gallery,
    Undo,
    Redo,
    Pen,
    Pencil,
    Highlighter,
    Eraser,
    Select,
    Paper,
    File,
    Pages,
    Plus,
    Trash,
    ZoomIn,
    ZoomOut,
    Fit,
    Close,
    Settings,
    Lock,
}

const ALL_ICONS: [IconId; 19] = [
    IconId::Gallery,
    IconId::Undo,
    IconId::Redo,
    IconId::Pen,
    IconId::Pencil,
    IconId::Highlighter,
    IconId::Eraser,
    IconId::Select,
    IconId::Paper,
    IconId::File,
    IconId::Pages,
    IconId::Plus,
    IconId::Trash,
    IconId::ZoomIn,
    IconId::ZoomOut,
    IconId::Fit,
    IconId::Close,
    IconId::Settings,
    IconId::Lock,
];

/// Rasterized monochrome `18x18` vector icons cached in `egui` textures.
pub struct IconRegistry {
    textures: HashMap<IconId, TextureHandle>,
    raster_scale_key: u32,
}

impl IconRegistry {
    pub fn new(ctx: &egui::Context, pixels_per_point: f32) -> Self {
        let mut reg = Self {
            textures: HashMap::with_capacity(ALL_ICONS.len()),
            raster_scale_key: 0,
        };
        reg.ensure_scale(ctx, pixels_per_point);
        reg
    }

    /// Re-rasterizes SVGs only when display scale changes significantly.
    pub fn ensure_scale(&mut self, ctx: &egui::Context, pixels_per_point: f32) {
        let clamped_scale = pixels_per_point.clamp(1.0, 4.0);
        let target_px = (BASE_ICON_PT * clamped_scale).round().max(18.0) as u32;
        if self.raster_scale_key == target_px && !self.textures.is_empty() {
            return;
        }
        self.raster_scale_key = target_px;
        let usvg_opts = usvg::Options::default();

        for icon in ALL_ICONS {
            let color_image = rasterize_white_svg(svg_markup(icon), target_px, &usvg_opts);
            let name = format!("swiftnote-icon-{icon:?}");
            let handle = ctx.load_texture(name, color_image, TextureOptions::LINEAR);
            self.textures.insert(icon, handle);
        }
    }

    pub fn image(&self, id: IconId, size_pt: f32, tint: egui::Color32) -> egui::Image<'_> {
        let handle = &self.textures[&id];
        egui::Image::new(handle)
            .fit_to_exact_size(egui::vec2(size_pt, size_pt))
            .tint(tint)
    }
}

fn rasterize_white_svg(svg: &str, px_size: u32, opts: &usvg::Options<'_>) -> ColorImage {
    let Ok(tree) = usvg::Tree::from_str(svg, opts) else {
        return ColorImage::filled([px_size as usize, px_size as usize], egui::Color32::WHITE);
    };
    let Some(mut pixmap) = Pixmap::new(px_size, px_size) else {
        return ColorImage::filled([px_size as usize, px_size as usize], egui::Color32::WHITE);
    };
    let scale = px_size as f32 / BASE_ICON_PT;
    resvg::render(
        &tree,
        Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );

    // Rasterize in pure white so egui can tint icons cleanly without dark-halo artifacts.
    let mut rgba = pixmap.take();
    for px in rgba.as_chunks_mut::<4>().0 {
        let a = px[3];
        if a > 0 {
            px[0] = 255;
            px[1] = 255;
            px[2] = 255;
        } else {
            px[0] = 0;
            px[1] = 0;
            px[2] = 0;
        }
    }
    ColorImage::from_rgba_unmultiplied([px_size as usize, px_size as usize], &rgba)
}

fn svg_markup(icon: IconId) -> &'static str {
    match icon {
        IconId::Gallery => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 18 18" fill="none" stroke="#FFFFFF" stroke-width="1.5" stroke-linecap="round"><path d="M3 4.5h12M3 9h12M3 13.5h12"/></svg>"##
        }
        IconId::Undo => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 18 18" fill="none" stroke="#FFFFFF" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M6.5 4.5 3 8l3.5 3.5"/><path d="M3.5 8H11a3.75 3.75 0 0 1 0 7.5H8.5"/></svg>"##
        }
        IconId::Redo => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 18 18" fill="none" stroke="#FFFFFF" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M11.5 4.5 15 8l-3.5 3.5"/><path d="M14.5 8H7a3.75 3.75 0 0 0 0 7.5h2.5"/></svg>"##
        }
        IconId::Pen => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 18 18" fill="none" stroke="#FFFFFF" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="m12.8 3.2 2 2L6 14l-3 .7.7-3 9.1-8.5z"/><path d="m11 5 2 2"/></svg>"##
        }
        IconId::Pencil => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 18 18" fill="none" stroke="#FFFFFF" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M13.2 2.8a1.4 1.4 0 0 1 2 2L5.5 14.5 2.5 15.5l1-3z"/><path d="m3.7 12.3 2 2"/></svg>"##
        }
        IconId::Highlighter => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 18 18" fill="none" stroke="#FFFFFF" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="m11 3.5 3.5 3.5-6.5 6.5H4.5V10z"/><path d="M3 15.5h12"/></svg>"##
        }
        IconId::Eraser => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 18 18" fill="none" stroke="#FFFFFF" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="m7.2 14.5-4-4a1.4 1.4 0 0 1 0-2L9.4 2.3a1.4 1.4 0 0 1 2 0l3.3 3.3a1.4 1.4 0 0 1 0 2l-5.7 5.7"/><path d="M3.5 14.5h11"/></svg>"##
        }
        IconId::Select => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 18 18" fill="none" stroke="#FFFFFF" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M4 3v12l3.4-3.2L10.3 15l1.6-.8-2.8-3.2H14z"/></svg>"##
        }
        IconId::Paper => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 18 18" fill="none" stroke="#FFFFFF" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><rect x="3.5" y="2.5" width="11" height="13" rx="1.5"/><path d="M6 6.5h6M6 9.5h6M6 12.5h4"/></svg>"##
        }
        IconId::File => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 18 18" fill="none" stroke="#FFFFFF" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M2.5 5a1.5 1.5 0 0 1 1.5-1.5h3.5l1.5 1.8h5A1.5 1.5 0 0 1 15.5 6.8v6.7a1.5 1.5 0 0 1-1.5 1.5H4a1.5 1.5 0 0 1-1.5-1.5z"/></svg>"##
        }
        IconId::Pages => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 18 18" fill="none" stroke="#FFFFFF" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><rect x="3" y="3" width="12" height="12" rx="1.5"/><path d="M11 3v12"/></svg>"##
        }
        IconId::Plus => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 18 18" fill="none" stroke="#FFFFFF" stroke-width="1.5" stroke-linecap="round"><path d="M9 3.5v11M3.5 9h11"/></svg>"##
        }
        IconId::Trash => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 18 18" fill="none" stroke="#FFFFFF" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M3.5 5h11M7 5V3.5h4V5M5.5 5l.7 9.5h5.6l.7-9.5"/></svg>"##
        }
        IconId::ZoomIn => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 18 18" fill="none" stroke="#FFFFFF" stroke-width="1.5" stroke-linecap="round"><circle cx="8" cy="8" r="4.5"/><path d="m11.5 11.5 3.5 3.5M8 6v4M6 8h4"/></svg>"##
        }
        IconId::ZoomOut => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 18 18" fill="none" stroke="#FFFFFF" stroke-width="1.5" stroke-linecap="round"><circle cx="8" cy="8" r="4.5"/><path d="m11.5 11.5 3.5 3.5M6 8h4"/></svg>"##
        }
        IconId::Fit => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 18 18" fill="none" stroke="#FFFFFF" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6.5V3h3.5M11.5 3H15v3.5M15 11.5V15h-3.5M6.5 15H3v-3.5"/></svg>"##
        }
        IconId::Close => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 18 18" fill="none" stroke="#FFFFFF" stroke-width="1.5" stroke-linecap="round"><path d="M4.5 4.5l9 9M13.5 4.5l-9 9"/></svg>"##
        }
        IconId::Settings => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 18 18" fill="none" stroke="#FFFFFF" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><circle cx="9" cy="9" r="2.2"/><path d="M9 2.5v1.6M9 13.9v1.6M2.5 9h1.6M13.9 9h1.6M4.4 4.4l1.1 1.1M12.5 12.5l1.1 1.1M13.6 4.4l-1.1 1.1M5.5 12.5l-1.1 1.1"/></svg>"##
        }
        IconId::Lock => {
            r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 18 18" fill="none" stroke="#FFFFFF" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><rect x="4" y="8" width="10" height="7" rx="1.5"/><path d="M6.2 8V6a2.8 2.8 0 0 1 5.6 0v2"/></svg>"##
        }
    }
}
