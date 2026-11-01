//! Decoder for the compact, content-addressed optional HD art pack.

use std::collections::BTreeMap;

pub const LIST: &str = "pack.txt";
pub const HEADER: &str = "cmr-hd pack 1";
pub const SETS: [&str; 2] = ["x2", "x4"];

/// FNV-1a-64 of an original BMP, used as its edition-safe replacement name.
#[must_use]
pub fn name(bmp: &[u8]) -> [u8; 16] {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for &byte in bmp {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let mut out = [0; 16];
    for (index, digit) in out.iter_mut().enumerate() {
        let nibble = ((hash >> (60 - 4 * index)) & 0x0f) as u8;
        *digit = if nibble < 10 {
            b'0' + nibble
        } else {
            b'a' + nibble - 10
        };
    }
    out
}

/// Convert one compact PNG/WebP entry back to the BMP consumed by the game.
pub fn to_bmp(file: &str, bytes: &[u8]) -> Result<(String, Vec<u8>), String> {
    let (stem, extension) = file
        .rsplit_once('.')
        .ok_or_else(|| format!("{file}: no extension"))?;
    let bmp = match extension {
        "png" => png_to_bmp(bytes),
        "webp" => colour_to_bmp(
            image::load_from_memory_with_format(bytes, image::ImageFormat::WebP)
                .map_err(|error| error.to_string())?,
        ),
        _ => return Err(format!("{file}: not a pack image")),
    }
    .map_err(|error| format!("{file}: {error}"))?;
    Ok((format!("{stem}.bmp"), bmp))
}

fn png_to_bmp(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().map_err(|error| error.to_string())?;
    let info = reader.info();
    if info.color_type == png::ColorType::Indexed && info.bit_depth == png::BitDepth::Eight {
        let (width, height) = (info.width, info.height);
        let palette: Vec<[u8; 3]> = info
            .palette
            .as_deref()
            .ok_or("indexed PNG has no palette")?
            .as_chunks::<3>()
            .0
            .to_vec();
        let mut indices = vec![0; reader.output_buffer_size().ok_or("image is too large")?];
        reader
            .next_frame(&mut indices)
            .map_err(|error| error.to_string())?;
        indices.truncate((width * height) as usize);
        return Ok(bmp8(width, height, &palette, &indices));
    }
    colour_to_bmp(
        image::load_from_memory_with_format(bytes, image::ImageFormat::Png)
            .map_err(|error| error.to_string())?,
    )
}

fn colour_to_bmp(image: image::DynamicImage) -> Result<Vec<u8>, String> {
    let keyed = image.color().has_alpha();
    let rgba = image.to_rgba8();
    let rgb: Vec<[u8; 3]> = rgba
        .pixels()
        .map(|pixel| match (keyed, pixel.0) {
            (true, [_, _, _, alpha]) if alpha < 128 => [0, 0, 0],
            (true, [0, 0, 0, _]) => [0, 0, 8],
            (_, [red, green, blue, _]) => [red, green, blue],
        })
        .collect();
    let (palette, indices) = adaptive_indexed(&rgb);
    Ok(bmp8(rgba.width(), rgba.height(), &palette, &indices))
}

#[derive(Clone, Copy)]
struct ColourBin {
    key: u16,
    count: u32,
    sums: [u64; 3],
}

fn adaptive_indexed(rgb: &[[u8; 3]]) -> (Vec<[u8; 3]>, Vec<u8>) {
    // Reduce the histogram to RGB555 before median-cut. This bounds hostile or
    // photographic inputs to 32,768 bins while retaining substantially more
    // source colour information than the game's original 256-entry palettes.
    let mut histogram = BTreeMap::<u16, (u32, [u64; 3])>::new();
    for &[red, green, blue] in rgb {
        let key = rgb555(red, green, blue);
        if key == 0 {
            continue;
        }
        let entry = histogram.entry(key).or_default();
        entry.0 += 1;
        entry.1[0] += u64::from(red);
        entry.1[1] += u64::from(green);
        entry.1[2] += u64::from(blue);
    }
    let bins = histogram
        .into_iter()
        .map(|(key, (count, sums))| ColourBin { key, count, sums })
        .collect::<Vec<_>>();
    let mut boxes = if bins.is_empty() {
        Vec::new()
    } else {
        vec![bins]
    };
    while boxes.len() < 255 {
        let Some((index, channel)) = boxes
            .iter()
            .enumerate()
            .filter_map(|(index, colours)| split_channel(colours).map(|channel| (index, channel)))
            .max_by_key(|(index, channel)| split_score(&boxes[*index], *channel))
        else {
            break;
        };
        let mut colours = boxes.swap_remove(index);
        colours.sort_unstable_by_key(|bin| component(bin.key, channel));
        let half = colours
            .iter()
            .map(|bin| u64::from(bin.count))
            .sum::<u64>()
            .div_ceil(2);
        let mut accumulated = 0_u64;
        let split = colours
            .iter()
            .position(|bin| {
                accumulated += u64::from(bin.count);
                accumulated >= half
            })
            .map_or(1, |position| (position + 1).min(colours.len() - 1));
        let right = colours.split_off(split);
        boxes.push(colours);
        boxes.push(right);
    }

    // Index zero is reserved for alpha-keyed black. Opaque source black was
    // moved to [0, 0, 8] above, exactly as in the original pack loader.
    let mut palette = vec![[0, 0, 0]];
    palette.extend(boxes.iter().map(|colours| {
        let count = colours.iter().map(|bin| u64::from(bin.count)).sum::<u64>();
        let mut sums = [0_u64; 3];
        for bin in colours {
            for (sum, contribution) in sums.iter_mut().zip(bin.sums) {
                *sum += contribution;
            }
        }
        [
            (sums[0] / count) as u8,
            (sums[1] / count) as u8,
            (sums[2] / count) as u8,
        ]
    }));

    let mut lookup = vec![0_u8; 1 << 15];
    for bin in boxes.iter().flatten() {
        let colour = bin.sums.map(|sum| (sum / u64::from(bin.count)) as u8);
        let best = palette
            .iter()
            .enumerate()
            .skip(1)
            .min_by_key(|(_, candidate)| colour_distance(colour, **candidate))
            .map_or(0, |(index, _)| index as u8);
        lookup[usize::from(bin.key)] = best;
    }
    let indices = rgb
        .iter()
        .map(|&[red, green, blue]| lookup[usize::from(rgb555(red, green, blue))])
        .collect();
    (palette, indices)
}

fn rgb555(red: u8, green: u8, blue: u8) -> u16 {
    (u16::from(red >> 3) << 10) | (u16::from(green >> 3) << 5) | u16::from(blue >> 3)
}

fn component(key: u16, channel: usize) -> u16 {
    match channel {
        0 => key >> 10,
        1 => (key >> 5) & 31,
        _ => key & 31,
    }
}

fn split_channel(colours: &[ColourBin]) -> Option<usize> {
    if colours.len() < 2 {
        return None;
    }
    (0..3).max_by_key(|&channel| {
        let min = colours
            .iter()
            .map(|bin| component(bin.key, channel))
            .min()
            .unwrap_or(0);
        let max = colours
            .iter()
            .map(|bin| component(bin.key, channel))
            .max()
            .unwrap_or(0);
        max - min
    })
}

fn split_score(colours: &[ColourBin], channel: usize) -> u64 {
    let min = colours
        .iter()
        .map(|bin| component(bin.key, channel))
        .min()
        .unwrap_or(0);
    let max = colours
        .iter()
        .map(|bin| component(bin.key, channel))
        .max()
        .unwrap_or(0);
    u64::from(max - min) * colours.iter().map(|bin| u64::from(bin.count)).sum::<u64>()
}

fn colour_distance(left: [u8; 3], right: [u8; 3]) -> u32 {
    left.into_iter()
        .zip(right)
        .map(|(left, right)| u32::from(left.abs_diff(right)).pow(2))
        .sum()
}

fn header(width: u32, height: u32, bits: u16, colours: u32, row: usize) -> Vec<u8> {
    let image_size = row as u32 * height;
    let offset = 54 + 4 * colours;
    let mut bytes = Vec::with_capacity((offset + image_size) as usize);
    bytes.extend_from_slice(b"BM");
    bytes.extend_from_slice(&(offset + image_size).to_le_bytes());
    bytes.extend_from_slice(&[0; 4]);
    bytes.extend_from_slice(&offset.to_le_bytes());
    bytes.extend_from_slice(&40_u32.to_le_bytes());
    bytes.extend_from_slice(&width.to_le_bytes());
    bytes.extend_from_slice(&height.to_le_bytes());
    bytes.extend_from_slice(&1_u16.to_le_bytes());
    bytes.extend_from_slice(&bits.to_le_bytes());
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    bytes.extend_from_slice(&image_size.to_le_bytes());
    bytes.extend_from_slice(&[0; 8]);
    bytes.extend_from_slice(&colours.to_le_bytes());
    bytes.extend_from_slice(&colours.to_le_bytes());
    bytes
}

#[must_use]
pub fn bmp8(width: u32, height: u32, palette: &[[u8; 3]], indices: &[u8]) -> Vec<u8> {
    let row = (width as usize + 3) & !3;
    let mut bytes = header(width, height, 8, 256, row);
    for index in 0..256 {
        let [red, green, blue] = palette.get(index).copied().unwrap_or([0; 3]);
        bytes.extend_from_slice(&[blue, green, red, 0]);
    }
    for y in (0..height as usize).rev() {
        let line = &indices[y * width as usize..(y + 1) * width as usize];
        bytes.extend_from_slice(line);
        bytes.resize(bytes.len() + row - line.len(), 0);
    }
    bytes
}

#[must_use]
pub fn bmp24(width: u32, height: u32, rgb: &[[u8; 3]]) -> Vec<u8> {
    let row = (3 * width as usize + 3) & !3;
    let mut bytes = header(width, height, 24, 0, row);
    for y in (0..height as usize).rev() {
        let start = bytes.len();
        for [red, green, blue] in &rgb[y * width as usize..(y + 1) * width as usize] {
            bytes.extend_from_slice(&[*blue, *green, *red]);
        }
        bytes.resize(start + row, 0);
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_stable_hex() {
        assert_eq!(&name(b""), b"cbf29ce484222325");
        assert_eq!(&name(b"a"), b"af63dc4c8601ec8c");
    }

    #[test]
    fn bitmap_rows_are_padded_and_bottom_up() {
        let bmp = bmp24(1, 2, &[[1, 2, 3], [4, 5, 6]]);
        assert_eq!(bmp.len(), 54 + 8);
        assert_eq!(&bmp[54..58], [6, 5, 4, 0]);
    }

    #[test]
    fn colour_images_become_indexed_with_keyed_black_at_zero() {
        let (palette, indices) =
            adaptive_indexed(&[[0, 0, 0], [0, 0, 8], [255, 0, 0], [0, 255, 0]]);
        assert_eq!(palette[0], [0, 0, 0]);
        assert_eq!(indices[0], 0);
        assert_ne!(indices[1], 0);
        assert_ne!(indices[2], indices[3]);
        assert!(palette.len() <= 256);
    }
}
