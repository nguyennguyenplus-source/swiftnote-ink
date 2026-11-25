use std::io::Cursor;

use crate::error::{Result, SwiftNoteError};

/// Neutral chroma threshold (max(r,g,b) - min(r,g,b)) for treating an ink color
/// as achromatic (black / charcoal / gray / white).
const ACHROMATIC_CHROMA_THRESHOLD: u8 = 28;

/// Non-destructive display-layer ink inversion for Blackboard mode.
///
/// - Neutral dark/light inks (black, dark charcoal, gray, white) are inverted:
///   `v' = 255 - v` per channel (so `#000000` <-> `#FFFFFF`, `#111111` <-> `#EEEEEE`).
/// - Chromatic inks (blue, red, green, yellow, purple, orange, etc.) are returned unchanged
///   so highlighters and color-coded math/study notes keep their intended colors.
#[inline]
pub fn invert_bw_ink_rgb(rgb24: u32) -> u32 {
    let r = ((rgb24 >> 16) & 0xFF) as u8;
    let g = ((rgb24 >> 8) & 0xFF) as u8;
    let b = (rgb24 & 0xFF) as u8;

    let max_c = r.max(g).max(b);
    let min_c = r.min(g).min(b);
    let chroma = max_c - min_c;

    if chroma <= ACHROMATIC_CHROMA_THRESHOLD {
        let ir = 255u32 - u32::from(r);
        let ig = 255u32 - u32::from(g);
        let ib = 255u32 - u32::from(b);
        (ir << 16) | (ig << 8) | ib
    } else {
        rgb24 & 0x00FF_FFFF
    }
}

/// Validates raw clipboard PNG bytes and normalizes them into lossless RGBA8 PNG
/// preserving exact pixel resolution and alpha channel.
pub fn normalize_clipboard_png(png_bytes: &[u8]) -> Result<(Vec<u8>, u32, u32)> {
    if png_bytes.is_empty() {
        return Err(SwiftNoteError::Media("empty PNG input".to_string()));
    }

    let mut decoder = png::Decoder::new(Cursor::new(png_bytes));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|e| SwiftNoteError::Media(format!("PNG decode header failed: {e}")))?;

    let mut raw = vec![0u8; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut raw)
        .map_err(|e| SwiftNoteError::Media(format!("PNG decode frame failed: {e}")))?;

    let width = info.width;
    let height = info.height;
    let frame = &raw[..info.buffer_size()];

    let rgba = match info.color_type {
        png::ColorType::Rgba => frame.to_vec(),
        png::ColorType::Rgb => {
            let mut out = Vec::with_capacity((width as usize) * (height as usize) * 4);
            for chunk in frame.as_chunks::<3>().0 {
                out.extend_from_slice(&[chunk[0], chunk[1], chunk[2], 255]);
            }
            out
        }
        png::ColorType::Grayscale => {
            let mut out = Vec::with_capacity((width as usize) * (height as usize) * 4);
            for &g in frame {
                out.extend_from_slice(&[g, g, g, 255]);
            }
            out
        }
        png::ColorType::GrayscaleAlpha => {
            let mut out = Vec::with_capacity((width as usize) * (height as usize) * 4);
            for chunk in frame.as_chunks::<2>().0 {
                out.extend_from_slice(&[chunk[0], chunk[0], chunk[0], chunk[1]]);
            }
            out
        }
        png::ColorType::Indexed => {
            return Err(SwiftNoteError::Media(
                "unexpected unexpanded indexed PNG".to_string(),
            ));
        }
    };

    let encoded = encode_lossless_rgba_png(&rgba, width, height)?;
    Ok((encoded, width, height))
}

/// Converts raw RGBA/BGRA pixel buffer (e.g. from Windows CF_DIB or GdkPixbuf)
/// to lossless RGBA8 PNG.
///
/// When `fix_all_zero_alpha` is true, detects the notorious Windows 32-bit DIB quirk
/// where every pixel has `alpha == 0` while RGB is non-zero, and repairs alpha to `255`
/// without altering genuinely transparent images.
pub fn pixels_to_lossless_png(
    pixels: &[u8],
    width: u32,
    height: u32,
    stride: u32,
    is_bgra: bool,
    fix_all_zero_alpha: bool,
) -> Result<Vec<u8>> {
    if width == 0 || height == 0 {
        return Err(SwiftNoteError::Media("zero image dimensions".to_string()));
    }
    let row_bytes = (width as usize)
        .checked_mul(4)
        .ok_or_else(|| SwiftNoteError::Media("image dimensions overflow".to_string()))?;
    let stride_usize = stride as usize;
    if stride_usize < row_bytes {
        return Err(SwiftNoteError::Media(
            "stride smaller than width * 4".to_string(),
        ));
    }
    let min_needed = stride_usize
        .checked_mul((height - 1) as usize)
        .and_then(|v| v.checked_add(row_bytes))
        .ok_or_else(|| SwiftNoteError::Media("buffer size overflow".to_string()))?;
    if pixels.len() < min_needed {
        return Err(SwiftNoteError::Media(
            "pixel buffer shorter than declared dimensions".to_string(),
        ));
    }

    let mut rgba = Vec::with_capacity(row_bytes * (height as usize));
    let mut any_nonzero_alpha = false;
    let mut any_nonzero_rgb = false;

    for y in 0..(height as usize) {
        let row_start = y * stride_usize;
        let row = &pixels[row_start..row_start + row_bytes];
        for px in row.as_chunks::<4>().0 {
            let (r, g, b, a) = if is_bgra {
                (px[2], px[1], px[0], px[3])
            } else {
                (px[0], px[1], px[2], px[3])
            };
            if a != 0 {
                any_nonzero_alpha = true;
            }
            if (r | g | b) != 0 {
                any_nonzero_rgb = true;
            }
            rgba.extend_from_slice(&[r, g, b, a]);
        }
    }

    if fix_all_zero_alpha && !any_nonzero_alpha && any_nonzero_rgb {
        for px in rgba.as_chunks_mut::<4>().0 {
            px[3] = 255;
        }
    }

    encode_lossless_rgba_png(&rgba, width, height)
}

fn encode_lossless_rgba_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, width, height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder
            .write_header()
            .map_err(|e| SwiftNoteError::Media(format!("PNG header write failed: {e}")))?;
        writer
            .write_image_data(rgba)
            .map_err(|e| SwiftNoteError::Media(format!("PNG data write failed: {e}")))?;
    }
    Ok(out)
}
