//! Pure icon bitmap normalization and layout helpers.

use super::ffi_types::Pixel32;

/// Returns the non-transparent pixel bounds as `(left, top, width, height)`.
pub(crate) fn visible_pixel_bounds(
    pixels: &[Pixel32],
    width: usize,
    height: usize,
) -> Option<(usize, usize, usize, usize)> {
    let expected_len = width.checked_mul(height)?;
    if width == 0 || height == 0 || pixels.len() < expected_len {
        return None;
    }

    let mut min_x = width;
    let mut min_y = height;
    let mut max_x = 0;
    let mut max_y = 0;

    for (index, pixel) in pixels[..expected_len].iter().enumerate() {
        if pixel.a == 0 {
            continue;
        }

        let x = index % width;
        let y = index / width;
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }

    if min_x == width {
        None
    } else {
        Some((min_x, min_y, max_x - min_x + 1, max_y - min_y + 1))
    }
}

/// Fits bitmap dimensions into a square while preserving aspect ratio and centering.
pub(crate) fn icon_destination_rect(
    bitmap_width: u32,
    bitmap_height: u32,
    x: i32,
    y: i32,
    size: i32,
) -> (f32, f32, f32, f32) {
    if bitmap_width == 0 || bitmap_height == 0 || size <= 0 {
        return (x as f32, y as f32, x as f32, y as f32);
    }

    let scale = size as f32 / bitmap_width.max(bitmap_height) as f32;
    let draw_width = bitmap_width as f32 * scale;
    let draw_height = bitmap_height as f32 * scale;
    let left = x as f32 + (size as f32 - draw_width) / 2.0;
    let top = y as f32 + (size as f32 - draw_height) / 2.0;
    (left, top, left + draw_width, top + draw_height)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(alpha: u8) -> Pixel32 {
        Pixel32 {
            b: 0,
            g: 0,
            r: 0,
            a: alpha,
        }
    }

    #[test]
    fn visible_pixel_bounds_removes_transparent_padding() {
        let mut pixels = vec![pixel(0); 8 * 6];
        for y in 1..5 {
            for x in 2..6 {
                pixels[y * 8 + x] = pixel(255);
            }
        }

        assert_eq!(visible_pixel_bounds(&pixels, 8, 6), Some((2, 1, 4, 4)));
    }

    #[test]
    fn visible_pixel_bounds_keeps_antialiased_edge_pixels() {
        let mut pixels = vec![pixel(0); 4 * 4];
        pixels[4 + 1] = pixel(1);
        pixels[2 * 4 + 2] = pixel(255);

        assert_eq!(visible_pixel_bounds(&pixels, 4, 4), Some((1, 1, 2, 2)));
    }

    #[test]
    fn visible_pixel_bounds_returns_none_for_empty_canvas() {
        let pixels = vec![pixel(0); 4 * 4];
        assert_eq!(visible_pixel_bounds(&pixels, 4, 4), None);
    }

    #[test]
    fn visible_pixel_bounds_keeps_full_canvas_icon() {
        let pixels = vec![pixel(255); 3 * 2];
        assert_eq!(visible_pixel_bounds(&pixels, 3, 2), Some((0, 0, 3, 2)));
    }

    #[test]
    fn visible_pixel_bounds_rejects_invalid_dimensions() {
        assert_eq!(visible_pixel_bounds(&[], 0, 4), None);
        assert_eq!(visible_pixel_bounds(&[pixel(255); 3], 2, 2), None);
        assert_eq!(visible_pixel_bounds(&[], usize::MAX, 2), None);
    }

    #[test]
    fn icon_destination_rect_preserves_aspect_ratio_and_centers() {
        assert_eq!(
            icon_destination_rect(32, 16, 10, 20, 48),
            (10.0, 32.0, 58.0, 56.0)
        );
    }

    #[test]
    fn icon_destination_rect_centers_portrait_content() {
        assert_eq!(
            icon_destination_rect(16, 32, 10, 20, 48),
            (22.0, 20.0, 46.0, 68.0)
        );
    }

    #[test]
    fn icon_destination_rect_handles_invalid_size() {
        assert_eq!(
            icon_destination_rect(32, 32, 10, 20, 0),
            (10.0, 20.0, 10.0, 20.0)
        );
        assert_eq!(
            icon_destination_rect(0, 32, 10, 20, 48),
            (10.0, 20.0, 10.0, 20.0)
        );
    }
}
