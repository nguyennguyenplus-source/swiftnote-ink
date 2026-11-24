use arboard::Clipboard;

/// Content decoded from the operating-system clipboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClipboardContent {
    Image {
        width: u32,
        height: u32,
        rgba: Vec<u8>,
    },
    Text(String),
    Empty,
}

/// Normalizes Windows CF_DIB / Snipping Tool bitmaps where every pixel carries `alpha == 0`.
///
/// If at least one pixel has a non-zero RGB channel and **all** alpha bytes in `rgba` are `0`,
/// sets every alpha channel to `255` and returns `true`. Real transparent PNG/RGBA images with
/// any non-zero alpha byte are preserved untouched and return `false`.
pub fn sanitize_dib_alpha(rgba: &mut [u8]) -> bool {
    if rgba.len() < 4 || !rgba.len().is_multiple_of(4) {
        return false;
    }

    let mut has_non_zero_rgb = false;
    let mut has_non_zero_alpha = false;

    for px in rgba.as_chunks::<4>().0 {
        if px[0] != 0 || px[1] != 0 || px[2] != 0 {
            has_non_zero_rgb = true;
        }
        if px[3] != 0 {
            has_non_zero_alpha = true;
            break;
        }
    }

    if has_non_zero_rgb && !has_non_zero_alpha {
        for px in rgba.as_chunks_mut::<4>().0 {
            px[3] = 255;
        }
        return true;
    }

    false
}

/// Reads the OS clipboard, preferring raw lossless RGBA image pixels before plain text.
pub fn read_os_clipboard() -> Result<ClipboardContent, String> {
    let mut clipboard =
        Clipboard::new().map_err(|err| format!("Failed to access OS clipboard: {err}"))?;

    if let Ok(image) = clipboard.get_image() {
        let width =
            u32::try_from(image.width).map_err(|_| "Clipboard image width overflow".to_owned())?;
        let height = u32::try_from(image.height)
            .map_err(|_| "Clipboard image height overflow".to_owned())?;
        if width > 0 && height > 0 {
            let expected_len = (width as usize)
                .checked_mul(height as usize)
                .and_then(|pixels| pixels.checked_mul(4))
                .ok_or_else(|| "Clipboard image byte count overflow".to_owned())?;
            let mut rgba = image.into_owned_bytes().into_owned();
            if rgba.len() == expected_len {
                let _ = sanitize_dib_alpha(&mut rgba);
                return Ok(ClipboardContent::Image {
                    width,
                    height,
                    rgba,
                });
            }
        }
    }

    match clipboard.get_text() {
        Ok(text) if !text.is_empty() => Ok(ClipboardContent::Text(text)),
        Ok(_) | Err(arboard::Error::ContentNotAvailable) => Ok(ClipboardContent::Empty),
        Err(err) => Err(format!("Failed to read clipboard text: {err}")),
    }
}

/// Writes plain text to the OS clipboard (used by UI text copy actions).
pub fn write_os_clipboard_text(text: &str) -> Result<(), String> {
    let mut clipboard =
        Clipboard::new().map_err(|err| format!("Failed to access OS clipboard: {err}"))?;
    clipboard
        .set_text(text.to_owned())
        .map_err(|err| format!("Failed to write clipboard text: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_dib_alpha_fixes_all_zero_alpha_bitmaps() {
        let mut dib = vec![
            10, 20, 30, 0, // pixel 1 (alpha 0 from CF_DIB)
            255, 128, 0, 0, // pixel 2
            0, 0, 0, 0, // pixel 3 (black pixel in opaque DIB)
        ];
        let fixed = sanitize_dib_alpha(&mut dib);
        assert!(fixed);
        assert_eq!(dib, vec![10, 20, 30, 255, 255, 128, 0, 255, 0, 0, 0, 255]);
    }

    #[test]
    fn sanitize_dib_alpha_preserves_real_transparency() {
        let original = vec![
            10, 20, 30, 0, // intentionally transparent background pixel
            255, 128, 0, 200, // semi-opaque foreground pixel
            0, 0, 0, 0, // transparent pixel
        ];
        let mut bytes = original.clone();
        let fixed = sanitize_dib_alpha(&mut bytes);
        assert!(!fixed);
        assert_eq!(bytes, original);
    }
}
