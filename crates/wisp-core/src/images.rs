//! PNG encoding and thumbnails for image clips.

use std::io::Cursor;

/// Longest side of a stored thumbnail, in pixels.
pub const THUMB_MAX: u32 = 160;

#[derive(Debug)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    /// Straight (not premultiplied) RGBA, row-major, top row first.
    pub pixels: Vec<u8>,
}

pub fn encode_png(img: &Rgba) -> Result<Vec<u8>, png::EncodingError> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, img.width, img.height);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        let mut w = enc.write_header()?;
        w.write_image_data(&img.pixels)?;
    }
    Ok(out)
}

/// Decode a PNG into 8-bit RGBA.
pub fn decode_png(bytes: &[u8]) -> Option<Rgba> {
    let mut dec = png::Decoder::new(Cursor::new(bytes));
    dec.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16 | png::Transformations::ALPHA);
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    let (w, h) = (info.width, info.height);
    let pixels = match info.color_type {
        png::ColorType::Rgba => buf,
        png::ColorType::GrayscaleAlpha => buf.chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        _ => return None,
    };
    Some(Rgba { width: w, height: h, pixels })
}

/// Box-filter downscale so the longest side is at most `max`.
pub fn thumbnail(img: &Rgba, max: u32) -> Rgba {
    let scale = (max as f32 / img.width.max(img.height) as f32).min(1.0);
    let tw = ((img.width as f32 * scale).round() as u32).max(1);
    let th = ((img.height as f32 * scale).round() as u32).max(1);
    let mut out = vec![0u8; (tw * th * 4) as usize];
    for ty in 0..th {
        let y0 = ty * img.height / th;
        let y1 = ((ty + 1) * img.height / th).max(y0 + 1);
        for tx in 0..tw {
            let x0 = tx * img.width / tw;
            let x1 = ((tx + 1) * img.width / tw).max(x0 + 1);
            let mut acc = [0u32; 4];
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = ((y * img.width + x) * 4) as usize;
                    for (a, p) in acc.iter_mut().zip(&img.pixels[i..i + 4]) {
                        *a += *p as u32;
                    }
                }
            }
            let n = (x1 - x0) * (y1 - y0);
            let o = ((ty * tw + tx) * 4) as usize;
            for (dst, a) in out[o..o + 4].iter_mut().zip(acc) {
                *dst = (a / n) as u8;
            }
        }
    }
    Rgba { width: tw, height: th, pixels: out }
}

/// Width and height from a PNG's header, without decoding it.
pub fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    Some((w, h))
}

/// Convert a Windows device-independent bitmap (the contents of `CF_DIB` or
/// `CF_DIBV5`: a BITMAPINFOHEADER or larger header followed by pixels) to
/// RGBA. Supports the 24- and 32-bit uncompressed layouts that apps put on
/// the clipboard.
pub fn from_dib(dib: &[u8]) -> Option<Rgba> {
    let u16_at = |o: usize| dib.get(o..o + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |o: usize| dib.get(o..o + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let header = u32_at(0)? as usize;
    let width = u32_at(4)? as i32;
    let height = u32_at(8)? as i32;
    let bpp = u16_at(14)?;
    let compression = u32_at(16)?;
    const BI_RGB: u32 = 0;
    const BI_BITFIELDS: u32 = 3;
    if width <= 0 || height == 0 || !(bpp == 24 || bpp == 32) {
        return None;
    }
    if compression != BI_RGB && !(compression == BI_BITFIELDS && bpp == 32) {
        return None;
    }
    // A plain 40-byte header keeps its three colour masks after it.
    let masks = if compression == BI_BITFIELDS && header == 40 { 12 } else { 0 };
    let offset = header + masks;
    let (w, h) = (width as usize, height.unsigned_abs() as usize);
    let stride = (w * bpp as usize).div_ceil(32) * 4;
    if dib.len() < offset + stride * h {
        return None;
    }
    let bottom_up = height > 0;
    let mut pixels = Vec::with_capacity(w * h * 4);
    let mut any_alpha = false;
    for y in 0..h {
        let src_row = if bottom_up { h - 1 - y } else { y };
        let row = &dib[offset + src_row * stride..];
        for x in 0..w {
            if bpp == 32 {
                let p = &row[x * 4..x * 4 + 4];
                any_alpha |= p[3] != 0;
                pixels.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
            } else {
                let p = &row[x * 3..x * 3 + 3];
                pixels.extend_from_slice(&[p[2], p[1], p[0], 255]);
            }
        }
    }
    // Most apps leave the fourth byte at zero; treat that as opaque.
    if bpp == 32 && !any_alpha {
        pixels.chunks_exact_mut(4).for_each(|p| p[3] = 255);
    }
    Some(Rgba { width: w as u32, height: h as u32, pixels })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dib(w: i32, h: i32, bpp: u16, compression: u32, pixel_rows: &[&[u8]]) -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(&40u32.to_le_bytes());
        d.extend_from_slice(&w.to_le_bytes());
        d.extend_from_slice(&h.to_le_bytes());
        d.extend_from_slice(&1u16.to_le_bytes());
        d.extend_from_slice(&bpp.to_le_bytes());
        d.extend_from_slice(&compression.to_le_bytes());
        d.extend_from_slice(&[0u8; 20]);
        if compression == 3 {
            for m in [0x00FF0000u32, 0x0000FF00, 0x000000FF] {
                d.extend_from_slice(&m.to_le_bytes());
            }
        }
        for r in pixel_rows {
            d.extend_from_slice(r);
        }
        d
    }

    #[test]
    fn dib_24_bit_bottom_up_with_padding() {
        // 1x2, rows padded to 4 bytes; stored bottom row first (blue), then top (red).
        let d = dib(1, 2, 24, 0, &[&[255, 0, 0, 0], &[0, 0, 255, 0]]);
        let img = from_dib(&d).unwrap();
        assert_eq!(img.pixels, [255, 0, 0, 255, 0, 0, 255, 255]);
    }

    #[test]
    fn dib_32_bit_zero_alpha_becomes_opaque() {
        let d = dib(2, -1, 32, 0, &[&[0, 255, 0, 0, 10, 20, 30, 0]]);
        let img = from_dib(&d).unwrap();
        assert_eq!(img.pixels, [0, 255, 0, 255, 30, 20, 10, 255]);
    }

    #[test]
    fn dib_32_bit_bitfields_keeps_real_alpha() {
        let d = dib(1, 1, 32, 3, &[&[1, 2, 3, 128]]);
        let img = from_dib(&d).unwrap();
        assert_eq!(img.pixels, [3, 2, 1, 128]);
    }

    #[test]
    fn dib_rejects_unsupported_or_short() {
        assert!(from_dib(&dib(1, 1, 8, 0, &[&[0, 0, 0, 0]])).is_none());
        assert!(from_dib(&dib(4, 4, 32, 0, &[&[0; 8]])).is_none());
    }

    #[test]
    fn reads_png_size() {
        let png = encode_png(&Rgba { width: 3, height: 9, pixels: vec![0; 3 * 9 * 4] }).unwrap();
        assert_eq!(png_size(&png), Some((3, 9)));
        assert_eq!(png_size(b"not a png at all, sorry"), None);
    }

    fn checker(w: u32, h: u32) -> Rgba {
        let pixels = (0..w * h)
            .flat_map(|i| if (i % w + i / w) % 2 == 0 { [255, 0, 0, 255] } else { [0, 0, 255, 255] })
            .collect();
        Rgba { width: w, height: h, pixels }
    }

    #[test]
    fn png_round_trip() {
        let img = checker(7, 5);
        let png = encode_png(&img).unwrap();
        let back = decode_png(&png).unwrap();
        assert_eq!((back.width, back.height), (7, 5));
        assert_eq!(back.pixels, img.pixels);
    }

    #[test]
    fn thumbnail_keeps_aspect_and_averages() {
        let t = thumbnail(&checker(800, 400), 160);
        assert_eq!((t.width, t.height), (160, 80));
        // A fine checkerboard averages to purple.
        assert!((120..=135).contains(&t.pixels[0]) && (120..=135).contains(&t.pixels[2]));
    }

    #[test]
    fn small_images_are_not_upscaled() {
        let t = thumbnail(&checker(20, 10), 160);
        assert_eq!((t.width, t.height), (20, 10));
    }
}
