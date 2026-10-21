use crate::prng::FastPrng;

/// Common resolution presets for random screenshot generation.
const COMMON_RESOLUTIONS: &[(u32, u32)] = &[
    (800, 600),
    (1024, 768),
    (1280, 720),
    (1366, 768),
    (1920, 1080),
];

const CRC32_TABLE: [u32; 256] = {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut j = 0;
        while j < 8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
            j += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
};

#[inline]
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        let idx = ((crc as u8) ^ byte) as usize;
        crc = (crc >> 8) ^ CRC32_TABLE[idx];
    }
    !crc
}

#[inline]
fn adler32(data: &[u8]) -> u32 {
    let mut s1: u32 = 1;
    let mut s2: u32 = 0;
    for chunk in data.chunks(5552) {
        for &b in chunk {
            s1 += b as u32;
            s2 += s1;
        }
        s1 %= 65521;
        s2 %= 65521;
    }
    (s2 << 16) | s1
}

fn write_png_chunk(buf: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
    let len = data.len() as u32;
    buf.extend_from_slice(&len.to_be_bytes());
    let type_offset = buf.len();
    buf.extend_from_slice(chunk_type);
    buf.extend_from_slice(data);
    let crc = crc32(&buf[type_offset..]);
    buf.extend_from_slice(&crc.to_be_bytes());
}

/// Generates a random PNG screenshot with random dimensions and colors.
pub fn generate_random_screenshot() -> Vec<u8> {
    let idx = FastPrng::gen_range_usize(0, COMMON_RESOLUTIONS.len());
    let (width, height) = COMMON_RESOLUTIONS[idx];
    generate_random_screenshot_with_dimensions(width, height)
}

/// Zero-dependency, ultra-fast PNG generator in pure Rust.
/// Generates a valid, lightweight RGB PNG image for the given `width` and `height`.
pub fn generate_random_screenshot_with_dimensions(width: u32, height: u32) -> Vec<u8> {
    let width = width.max(1);
    let height = height.max(1);

    let row_len = 1 + (width as usize) * 3;
    let total_scanline_bytes = (height as usize) * row_len;

    // Generate scanline raw data (filter byte 0x00 + RGB bytes per scanline)
    let mut scanlines = Vec::with_capacity(total_scanline_bytes);

    let base_r: u8 = FastPrng::gen_range_u8_inclusive(20, 220);
    let base_g: u8 = FastPrng::gen_range_u8_inclusive(20, 220);
    let base_b: u8 = FastPrng::gen_range_u8_inclusive(20, 220);

    for y in 0..height {
        scanlines.push(0x00); // Filter type: None
        let row_r = base_r.wrapping_add((y as u8).wrapping_mul(3));
        let row_g = base_g.wrapping_add((y as u8).wrapping_mul(5));
        let row_b = base_b.wrapping_add((y as u8).wrapping_mul(7));

        for _ in 0..width {
            scanlines.push(row_r);
            scanlines.push(row_g);
            scanlines.push(row_b);
        }
    }

    let adler = adler32(&scanlines);

    // Build ZLIB compressed stream (RFC 1950 zlib wrapper over DEFLATE stored blocks)
    // Estimate zlib size: 2 header + blocks (5 bytes header + len data) + 4 adler trailer
    let max_block_size = 65535usize;
    let num_blocks = scanlines.len().div_ceil(max_block_size);
    let zlib_capacity = 2 + num_blocks * 5 + scanlines.len() + 4;
    let mut zlib_data = Vec::with_capacity(zlib_capacity);

    // ZLIB header: CINFO=7 (32K window), CM=8 (deflate), FCHECK=0x01 -> 0x78 0x01
    zlib_data.extend_from_slice(&[0x78, 0x01]);

    let chunks: Vec<&[u8]> = scanlines.chunks(max_block_size).collect();
    let num_chunks = chunks.len();

    for (idx, chunk) in chunks.into_iter().enumerate() {
        let is_last = idx == num_chunks - 1;
        let header_byte = if is_last { 0x01 } else { 0x00 };
        let len = chunk.len() as u16;
        let nlen = !len;

        zlib_data.push(header_byte);
        zlib_data.extend_from_slice(&len.to_le_bytes());
        zlib_data.extend_from_slice(&nlen.to_le_bytes());
        zlib_data.extend_from_slice(chunk);
    }

    zlib_data.extend_from_slice(&adler.to_be_bytes());

    // Allocate PNG output buffer
    let mut png_buf = Vec::with_capacity(8 + 25 + 12 + zlib_data.len() + 12);

    // 1. PNG Header
    png_buf.extend_from_slice(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);

    // 2. IHDR Chunk (13 bytes)
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&height.to_be_bytes());
    ihdr[8] = 8; // 8 bits per channel
    ihdr[9] = 2; // RGB color type
    ihdr[10] = 0; // Deflate compression
    ihdr[11] = 0; // Standard filter method
    ihdr[12] = 0; // No interlace
    write_png_chunk(&mut png_buf, b"IHDR", &ihdr);

    // 3. IDAT Chunk
    write_png_chunk(&mut png_buf, b"IDAT", &zlib_data);

    // 4. IEND Chunk
    write_png_chunk(&mut png_buf, b"IEND", &[]);

    png_buf
}

/// Zero-allocation O(1) PNG header parser.
/// Verifies PNG magic bytes and `IHDR` chunk marker, returning `(width, height)`.
pub fn parse_png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    const PNG_HEADER: &[u8; 8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if bytes.len() < 24 || &bytes[..8] != PNG_HEADER || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let w = u32::from_be_bytes(bytes[16..20].try_into().ok()?);
    let h = u32::from_be_bytes(bytes[20..24].try_into().ok()?);
    if w == 0 || h == 0 {
        return None;
    }
    Some((w, h))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_random_screenshot_png_header() {
        let bytes = generate_random_screenshot();
        assert!(!bytes.is_empty(), "Screenshot bytes should not be empty");

        let png_header = &[137, 80, 78, 71, 13, 10, 26, 10];
        assert_eq!(
            &bytes[..8],
            png_header,
            "Generated image should have valid PNG magic header"
        );
    }

    #[test]
    fn test_generate_random_screenshot_with_custom_dimensions() {
        let (width, height) = (640, 480);
        let bytes = generate_random_screenshot_with_dimensions(width, height);
        assert!(!bytes.is_empty());

        let (w, h) = parse_png_dimensions(&bytes).expect("Should parse dimensions");
        assert_eq!(w, width);
        assert_eq!(h, height);
    }

    #[test]
    fn test_crc32_and_adler32_correctness() {
        let data = b"123456789";
        assert_eq!(crc32(data), 0xCBF43926);

        let adler = adler32(b"Wikipedia");
        assert_eq!(adler, 0x11E60398);
    }
}
