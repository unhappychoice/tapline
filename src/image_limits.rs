use image::Limits;

/// Chart-supplied artwork (banners, stage files, BGA frames) is always
/// downscaled to small terminal thumbnails or a 256x256 BGA canvas, so a
/// crafted image that declares far larger dimensions is decompression-bomb
/// abuse rather than a legitimate asset.
const MAX_IMAGE_DIMENSION: u32 = 4096;
const MAX_IMAGE_ALLOC_BYTES: u64 = 64 * 1024 * 1024;

pub(crate) fn decode_limits() -> Limits {
    let mut limits = Limits::no_limits();
    limits.max_image_width = Some(MAX_IMAGE_DIMENSION);
    limits.max_image_height = Some(MAX_IMAGE_DIMENSION);
    limits.max_alloc = Some(MAX_IMAGE_ALLOC_BYTES);
    limits
}

/// A minimal PNG containing only a signature and an `IHDR` chunk that
/// declares dimensions beyond `MAX_IMAGE_DIMENSION`. The dimension check
/// runs right after the header is parsed, before any pixel data (`IDAT`) is
/// read, so this file never needs to contain real image data.
#[cfg(test)]
pub(crate) fn oversized_png_bytes() -> Vec<u8> {
    fn crc32(data: &[u8]) -> u32 {
        let mut crc = 0xFFFF_FFFFu32;
        for &byte in data {
            crc ^= u32::from(byte);
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }
    fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut bytes = (data.len() as u32).to_be_bytes().to_vec();
        bytes.extend_from_slice(kind);
        bytes.extend_from_slice(data);
        let crc_input: Vec<u8> = kind.iter().chain(data.iter()).copied().collect();
        bytes.extend_from_slice(&crc32(&crc_input).to_be_bytes());
        bytes
    }

    let side = MAX_IMAGE_DIMENSION + 1;
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&side.to_be_bytes());
    ihdr.extend_from_slice(&side.to_be_bytes());
    ihdr.extend_from_slice(&[8, 6, 0, 0, 0]); // 8-bit depth, RGBA, no interlace

    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.extend(chunk(b"IHDR", &ihdr));
    png
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_limits_caps_dimensions_and_allocation() {
        let limits = decode_limits();
        assert_eq!(limits.max_image_width, Some(MAX_IMAGE_DIMENSION));
        assert_eq!(limits.max_image_height, Some(MAX_IMAGE_DIMENSION));
        assert_eq!(limits.max_alloc, Some(MAX_IMAGE_ALLOC_BYTES));
    }
}
