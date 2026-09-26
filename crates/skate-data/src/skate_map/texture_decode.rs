//! Bounded parallel decompression with deterministic texture indices.
use super::{StoredBlock, Texture};
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

/// Number of times embedded textures are halved after decoding. Zero (the
/// default) keeps authored resolution; phones set it to fit texture memory.
static REDUCTION: AtomicU32 = AtomicU32::new(0);
/// Textures whose shorter side is below this are never reduced further.
const MIN_REDUCED_SIDE: u32 = 128;

/// Halve every embedded texture `levels` times in maps parsed afterwards.
pub fn set_texture_reduction(levels: u32) {
    REDUCTION.store(levels.min(4), Ordering::Relaxed);
}

/// 2x2 box-filter halving while both sides stay even and the shorter side is
/// at least MIN_REDUCED_SIDE. Cube strips (height = 6 x width) halve per face:
/// the face height stays even, so no filtered pair crosses a face boundary.
pub(super) fn reduce(mut width: u32, mut height: u32, mut rgba: Vec<u8>, levels: u32) -> (u32, u32, Vec<u8>) {
    for _ in 0..levels {
        if width % 2 != 0 || height % 2 != 0 || width.min(height) < MIN_REDUCED_SIDE {
            break;
        }
        let (w, h, row) = (width as usize / 2, height as usize / 2, width as usize * 4);
        let mut out = vec![0u8; w * h * 4];
        for y in 0..h {
            let (top, bottom) = (&rgba[2 * y * row..][..row], &rgba[(2 * y + 1) * row..][..row]);
            for (x, pixel) in out[y * w * 4..][..w * 4].chunks_exact_mut(4).enumerate() {
                for (c, value) in pixel.iter_mut().enumerate() {
                    let i = x * 8 + c;
                    let sum = u16::from(top[i]) + u16::from(top[i + 4]) + u16::from(bottom[i]) + u16::from(bottom[i + 4]);
                    *value = ((sum + 2) / 4) as u8;
                }
            }
        }
        (width, height, rgba) = (w as u32, h as u32, out);
    }
    (width, height, rgba)
}

pub(super) fn decode(textures: &mut [Texture], blocks: &[StoredBlock<'_>], workers: usize) -> Result<usize, String> {
    let bytes = blocks.iter().fold(0usize, |sum, block| sum.saturating_add(block.expected));
    let workers = if bytes < 1024 * 1024 { 1 } else { workers.clamp(1, 8).min(blocks.len().max(1)) };
    let levels = REDUCTION.load(Ordering::Relaxed);
    let sizes: Vec<(u32, u32)> = textures.iter().map(|t| (t.width, t.height)).collect();
    // Reduce inside the workers so full-size pixels never accumulate.
    let finish = |index: usize, rgba: Vec<u8>| -> (u32, u32, Vec<u8>) {
        let (width, height) = sizes[index];
        if levels == 0 { (width, height, rgba) } else { reduce(width, height, rgba, levels) }
    };
    let store = |texture: &mut Texture, (width, height, rgba): (u32, u32, Vec<u8>)| {
        (texture.width, texture.height, texture.rgba) = (width, height, rgba);
    };
    if workers == 1 {
        for (index, (texture, block)) in textures.iter_mut().zip(blocks).enumerate() {
            store(texture, finish(index, block.decode()?));
        }
        return Ok(1);
    }
    // Dynamic assignment balances many tiny textures against a few large ones.
    // Compressed slices borrow the original file; no second package copy exists.
    let next = AtomicUsize::new(0);
    let decode_batch = || -> Result<Vec<(usize, (u32, u32, Vec<u8>))>, String> {
        let mut decoded = Vec::new();
        loop {
            let index = next.fetch_add(1, Ordering::Relaxed);
            let Some(block) = blocks.get(index) else { break; };
            decoded.push((index, finish(index, block.decode().map_err(|e| format!("Texture {index}: {e}"))?)));
        }
        Ok(decoded)
    };
    let decoded = std::thread::scope(|scope| -> Result<_, String> {
        let mut jobs = Vec::new();
        for _ in 1..workers {
            jobs.push(std::thread::Builder::new().name("map-texture-decode".into())
                .spawn_scoped(scope, &decode_batch).map_err(|e| format!("Texture worker: {e}"))?);
        }
        let local = decode_batch();
        // Join every worker before returning either success or an error.
        let batches: Vec<_> = jobs.into_iter().map(|job| job.join()
            .unwrap_or_else(|_| Err("Texture decoder failed unexpectedly".into()))).collect();
        let mut decoded = local?;
        for batch in batches { decoded.extend(batch?); }
        Ok(decoded)
    })?;
    for (index, texture) in decoded { store(&mut textures[index], texture); }
    Ok(workers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn mixed_storage_keeps_order_and_rejects_corruption_in_parallel() {
        let raw: Vec<Vec<u8>> = (0..16).map(|i| vec![i as u8; (i + 1) * 32 * 1024]).collect();
        let mut encoded: Vec<Vec<u8>> = raw.iter().enumerate().map(|(i, bytes)| match i % 3 {
            0 => bytes.clone(),
            1 => {
                let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
                encoder.write_all(bytes).unwrap();
                encoder.finish().unwrap()
            }
            _ => zstd::stream::encode_all(bytes.as_slice(), 1).unwrap(),
        }).collect();
        let make_textures = || (0..raw.len()).map(|i| Texture {
            name: format!("texture-{i}"), width: 1, height: 1, color_space: 0, rgba: Vec::new(),
        }).collect::<Vec<_>>();
        {
            let blocks: Vec<_> = encoded.iter().enumerate().map(|(i, bytes)| StoredBlock {
                bytes, method: (i % 3) as u32, expected: raw[i].len(),
            }).collect();
            let mut serial = make_textures();
            decode(&mut serial, &blocks, 1).unwrap();
            for workers in [2, 4, usize::MAX] {
                let mut parallel = make_textures();
                assert_eq!(decode(&mut parallel, &blocks, workers).unwrap(), workers.min(8));
                assert_eq!(serial, parallel);
                for (texture, bytes) in parallel.iter().zip(&raw) { assert_eq!(&texture.rgba, bytes); }
            }
        }
        encoded[5].truncate(3);
        let blocks: Vec<_> = encoded.iter().enumerate().map(|(i, bytes)| StoredBlock {
            bytes, method: (i % 3) as u32, expected: raw[i].len(),
        }).collect();
        assert!(decode(&mut make_textures(), &blocks, 4).is_err());
    }

    #[test]
    fn reduction_halves_even_textures_and_keeps_cube_faces_separate() {
        // 256x256 with a vertical gradient per 2x2 block.
        let (w, h) = (256u32, 256u32);
        let rgba: Vec<u8> = (0..w * h).flat_map(|i| {
            let (x, y) = (i % w, i / w);
            [(x % 2 * 100) as u8, (y % 2 * 200) as u8, 7, 255]
        }).collect();
        let (rw, rh, out) = reduce(w, h, rgba.clone(), 1);
        assert_eq!((rw, rh, out.len()), (128, 128, 128 * 128 * 4));
        assert_eq!(&out[..4], &[50, 100, 7, 255]);
        assert_eq!(reduce(w, h, rgba.clone(), 0), (w, h, rgba.clone()));
        // Stops at the minimum side and on odd sizes.
        assert_eq!(reduce(w, h, rgba.clone(), 4).0, 64);
        let odd = vec![1u8; 130 * 129 * 4];
        assert_eq!(reduce(130, 129, odd.clone(), 1), (130, 129, odd));
        // Cube strip: each face a solid colour; faces must not blend.
        let face = 128u32;
        let cube: Vec<u8> = (0..face * face * 6).flat_map(|i| [(i / (face * face)) as u8 * 40, 0, 0, 255]).collect();
        let (cw, ch, reduced) = reduce(face, face * 6, cube, 1);
        assert_eq!((cw, ch), (64, 384));
        for f in 0..6usize {
            let start = f * 64 * 64 * 4;
            assert!(reduced[start..start + 64 * 64 * 4].chunks_exact(4).all(|p| p[0] == f as u8 * 40));
        }
    }

    #[test]
    fn decode_applies_reduction_to_sizes_in_parallel() {
        let raw: Vec<Vec<u8>> = (0..8).map(|i| vec![i as u8; 256 * 256 * 4]).collect();
        let blocks: Vec<_> = raw.iter().map(|bytes| StoredBlock { bytes, method: 0, expected: bytes.len() }).collect();
        let mut textures: Vec<_> = (0..raw.len()).map(|i| Texture {
            name: format!("t{i}"), width: 256, height: 256, color_space: 0, rgba: Vec::new(),
        }).collect();
        set_texture_reduction(1);
        let result = decode(&mut textures, &blocks, 4);
        set_texture_reduction(0);
        result.unwrap();
        for (i, texture) in textures.iter().enumerate() {
            assert_eq!((texture.width, texture.height, texture.rgba.len()), (128, 128, 128 * 128 * 4));
            assert!(texture.rgba.iter().all(|&b| b == i as u8));
        }
    }

    #[test]
    fn stored_block_retains_size_and_deflate_tail_validation() {
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&[7; 64]).unwrap();
        let mut bytes = encoder.finish().unwrap();
        assert!(StoredBlock { expected: 63, method: 1, bytes: &bytes }.decode().is_err());
        bytes.push(42);
        assert!(StoredBlock { expected: 64, method: 1, bytes: &bytes }.decode().is_err());
        assert!(StoredBlock { expected: 1, method: 0, bytes: &[] }.decode().is_err());
        assert!(StoredBlock { expected: 0, method: 99, bytes: &[] }.decode().is_err());
    }
}
