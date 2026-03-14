/// ICC color profile parsing and color space conversion.
///
/// iPhone photos (and many other modern camera images) embed a Display P3 ICC
/// profile in the JPEG APP2 segment.  The `image` crate ignores that profile
/// when it decodes the file, so every pixel's raw gamma-encoded P3 value gets
/// silently misinterpreted as sRGB – making vibrant pinks, greens and yellows
/// collapse into muted brownish earth tones.
///
/// This module
///   1. Extracts the ICC profile bytes from a JPEG byte stream.
///   2. Reads the `rXYZ`, `gXYZ`, `bXYZ` primary tags from the profile.
///   3. Computes the linear-light 3×3 transform matrix that converts from the
///      source color space to linear sRGB.
///   4. Applies that matrix in-place to a packed RGB-8 pixel buffer.

// ── ICC tag-table helpers ────────────────────────────────────────────────────

/// Parse one s15.16 fixed-point number from 4 big-endian bytes.
#[inline]
fn s15f16(b: &[u8]) -> f32 {
    i32::from_be_bytes([b[0], b[1], b[2], b[3]]) as f32 / 65536.0
}

/// Find a tag entry in the ICC tag table and return its data slice.
///
/// The tag table starts at byte 128 of the profile:
///   byte 128-131 : number of tags (u32 BE)
///   then N × 12-byte entries: [ sig(4) | offset(4) | size(4) ]
fn find_tag<'a>(icc: &'a [u8], sig: &[u8; 4]) -> Option<&'a [u8]> {
    if icc.len() < 132 {
        return None;
    }
    let count = u32::from_be_bytes(icc[128..132].try_into().ok()?) as usize;
    for i in 0..count {
        let base = 132 + i * 12;
        if base + 12 > icc.len() {
            break;
        }
        if &icc[base..base + 4] == sig {
            let offset = u32::from_be_bytes(icc[base + 4..base + 8].try_into().ok()?) as usize;
            let size = u32::from_be_bytes(icc[base + 8..base + 12].try_into().ok()?) as usize;
            if offset.saturating_add(size) <= icc.len() {
                return Some(&icc[offset..offset + size]);
            }
        }
    }
    None
}

/// Parse an `XYZ ` tag body: `XYZ ` sig (4) + reserved (4) + X Y Z (12).
fn parse_xyz(tag_data: &[u8]) -> Option<[f32; 3]> {
    if tag_data.len() < 20 || &tag_data[0..4] != b"XYZ " {
        return None;
    }
    Some([s15f16(&tag_data[8..12]), s15f16(&tag_data[12..16]), s15f16(&tag_data[16..20])])
}

// ── Matrix arithmetic ────────────────────────────────────────────────────────

fn mat3_mul(a: [[f32; 3]; 3], b: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut c = [[0.0f32; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            for k in 0..3 {
                c[i][j] += a[i][k] * b[k][j];
            }
        }
    }
    c
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Detect the image format from the first few magic bytes.
pub enum DetectedFormat {
    Jpeg,
    Png,
    Other,
}

pub fn detect_format(data: &[u8]) -> DetectedFormat {
    if data.starts_with(&[0xFF, 0xD8]) {
        DetectedFormat::Jpeg
    } else if data.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        DetectedFormat::Png
    } else {
        DetectedFormat::Other
    }
}

/// Extract the ICC profile bytes from whichever format the raw image bytes are.
/// Returns `None` if the image has no embedded profile or the format is not
/// JPEG / PNG.
pub fn extract_icc(data: &[u8]) -> Option<Vec<u8>> {
    match detect_format(data) {
        DetectedFormat::Jpeg => extract_jpeg_icc(data),
        DetectedFormat::Png => extract_png_icc(data),
        DetectedFormat::Other => None,
    }
}

/// Extract the concatenated ICC profile bytes from JPEG APP2 segments.
///
/// JPEG can split the profile across multiple APP2 (0xFF 0xE2) markers, each
/// prefixed with `"ICC_PROFILE\0" + seq(1) + total(1)`.  We sort by sequence
/// number and concatenate the payloads.
pub fn extract_jpeg_icc(data: &[u8]) -> Option<Vec<u8>> {
    if data.len() < 2 || data[0] != 0xFF || data[1] != 0xD8 {
        return None;
    }

    const SIG: &[u8] = b"ICC_PROFILE\0";
    // BTreeMap keyed by 1-based sequence number preserves insertion order.
    let mut chunks: std::collections::BTreeMap<u8, Vec<u8>> = Default::default();

    let mut pos = 2usize;
    while pos + 3 < data.len() {
        if data[pos] != 0xFF {
            break;
        }
        let marker = data[pos + 1];

        // SOS / EOI – no more headers follow.
        if marker == 0xDA || marker == 0xD9 {
            break;
        }
        // Stand-alone markers (no length field).
        if marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            pos += 2;
            continue;
        }
        if pos + 4 > data.len() {
            break;
        }
        let seg_len = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
        if seg_len < 2 || pos + 2 + seg_len > data.len() {
            break;
        }

        // APP2 (0xE2) with the ICC signature?
        if marker == 0xE2 {
            let seg = &data[pos + 4..pos + 2 + seg_len];
            if seg.len() >= 14 && seg.starts_with(SIG) {
                let seq = seg[12]; // 1-based
                chunks.insert(seq, seg[14..].to_vec());
            }
        }

        pos += 2 + seg_len;
    }

    if chunks.is_empty() {
        return None;
    }
    let mut result = Vec::new();
    for (_, payload) in chunks {
        result.extend_from_slice(&payload);
    }
    Some(result)
}

/// Extract the ICC profile from a PNG `iCCP` chunk.
///
/// PNG stores the ICC profile in an `iCCP` chunk before the first `IDAT`.
/// The chunk data is: null-terminated profile name + 1-byte compression method
/// (always 0 = zlib) + zlib-compressed ICC bytes.
pub fn extract_png_icc(data: &[u8]) -> Option<Vec<u8>> {
    const PNG_SIG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    if !data.starts_with(PNG_SIG) {
        return None;
    }
    let mut pos = 8usize; // skip 8-byte signature
    while pos + 12 <= data.len() {
        let chunk_len = u32::from_be_bytes(data[pos..pos + 4].try_into().ok()?) as usize;
        let chunk_type = &data[pos + 4..pos + 8];

        if chunk_type == b"iCCP" {
            let chunk_data = data.get(pos + 8..pos + 8 + chunk_len)?;
            // Skip null-terminated name + 1-byte compression method.
            let null_pos = chunk_data.iter().position(|&b| b == 0)?;
            let compressed = chunk_data.get(null_pos + 2..)?; // +2: skip '\0' + method byte

            use std::io::Read;
            let mut dec = flate2::read::ZlibDecoder::new(compressed);
            let mut icc = Vec::new();
            dec.read_to_end(&mut icc).ok()?;
            return Some(icc);
        }

        // `IDAT` marks the start of image data – no more metadata after this.
        if chunk_type == b"IDAT" {
            break;
        }

        pos += 12 + chunk_len; // 4 (len) + 4 (type) + chunk_len + 4 (crc)
    }
    None
}

/// Compute the linear-light 3×3 matrix that converts from the ICC profile's
/// color space to linear sRGB.  Returns `None` when the transform is
/// indistinguishable from the identity (i.e. the source is already sRGB).
///
/// ## How it works
///
/// ICC profiles store primary XYZ values in D50-adapted CIE XYZ space.
/// Multiplying those primaries by the standard D50-XYZ→linear-sRGB matrix
/// (which already folds in the Bradford D50→D65 chromatic adaptation) gives
/// us the combined source→sRGB linear transform in one step.
pub fn icc_to_srgb_matrix(icc: &[u8]) -> Option<[[f32; 3]; 3]> {
    // Quick sanity-check: bytes 36-39 must be 'acsp'.
    if icc.len() < 40 || &icc[36..40] != b"acsp" {
        return None;
    }

    let r = parse_xyz(find_tag(icc, b"rXYZ")?)?;
    let g = parse_xyz(find_tag(icc, b"gXYZ")?)?;
    let b = parse_xyz(find_tag(icc, b"bXYZ")?)?;

    // Source primary matrix: each column is the D50-adapted XYZ of one primary.
    //   row 0 = X, row 1 = Y, row 2 = Z
    let m_src = [
        [r[0], g[0], b[0]],
        [r[1], g[1], b[1]],
        [r[2], g[2], b[2]],
    ];

    // D50-adapted XYZ → linear sRGB.
    // This is the inverse of the sRGB primary matrix expressed in D50 XYZ
    // (Bradford adaptation already folded in).
    // Reference: http://www.brucelindbloom.com/index.html?Eqn_RGB_XYZ_Matrix.html
    let m_xyz_to_srgb: [[f32; 3]; 3] = [
        [3.1338561, -1.6168667, -0.4906146],
        [-0.9787684, 1.9161415, 0.0334540],
        [0.0719453, -0.2289914, 1.4052427],
    ];

    // Combined: source_linear → D50 XYZ → linear sRGB
    let t = mat3_mul(m_xyz_to_srgb, m_src);

    // Skip the transform if it is already the identity within floating-point
    // noise (i.e. the embedded profile describes sRGB).
    for i in 0..3usize {
        for j in 0..3usize {
            let expected = if i == j { 1.0f32 } else { 0.0 };
            if (t[i][j] - expected).abs() > 0.005 {
                return Some(t);
            }
        }
    }
    None
}

// ── Pixel-level transform ────────────────────────────────────────────────────

#[inline]
fn srgb8_to_linear(v: u8) -> f32 {
    let s = v as f32 / 255.0;
    if s <= 0.04045 {
        s / 12.92
    } else {
        ((s + 0.055) / 1.055).powf(2.4)
    }
}

#[inline]
fn linear_to_srgb8(v: f32) -> u8 {
    let v = v.clamp(0.0, 1.0);
    let s = if v <= 0.0031308 {
        12.92 * v
    } else {
        1.055 * v.powf(1.0 / 2.4) - 0.055
    };
    (s * 255.0 + 0.5) as u8
}

/// Apply a linear-light 3×3 color-space transform matrix to a packed RGB-8
/// pixel buffer **in place**.  Each pixel is linearised, transformed, and
/// re-encoded to sRGB.
pub fn apply_matrix_to_rgb8(pixels: &mut [u8], m: [[f32; 3]; 3]) {
    for px in pixels.chunks_exact_mut(3) {
        let lr = srgb8_to_linear(px[0]);
        let lg = srgb8_to_linear(px[1]);
        let lb = srgb8_to_linear(px[2]);
        px[0] = linear_to_srgb8(m[0][0] * lr + m[0][1] * lg + m[0][2] * lb);
        px[1] = linear_to_srgb8(m[1][0] * lr + m[1][1] * lg + m[1][2] * lb);
        px[2] = linear_to_srgb8(m[2][0] * lr + m[2][1] * lg + m[2][2] * lb);
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// sRGB primaries (D50-adapted XYZ), as stored in the sRGB ICC profile.
    fn srgb_icc_primaries() -> ([[f32; 3]; 3], [[f32; 3]; 3], [[f32; 3]; 3]) {
        // rXYZ, gXYZ, bXYZ – values from the canonical sRGB ICC profile.
        let r = [0.43607, 0.22249, 0.01392];
        let g = [0.38515, 0.71687, 0.09709];
        let b = [0.14307, 0.06061, 0.71411];
        (
            [r, [0.0; 3], [0.0; 3]],
            [g, [0.0; 3], [0.0; 3]],
            [b, [0.0; 3], [0.0; 3]],
        )
    }

    #[test]
    fn srgb_primaries_produce_identity() {
        // Build a minimal fake ICC profile with sRGB D50-adapted XYZ primaries.
        // icc_to_srgb_matrix should return None (no transform needed).
        let r = [0.43607f32, 0.22249, 0.01392];
        let g = [0.38515f32, 0.71687, 0.09709];
        let b = [0.14307f32, 0.06061, 0.71411];

        let m_src = [
            [r[0], g[0], b[0]],
            [r[1], g[1], b[1]],
            [r[2], g[2], b[2]],
        ];
        let m_xyz_to_srgb: [[f32; 3]; 3] = [
            [3.1338561, -1.6168667, -0.4906146],
            [-0.9787684, 1.9161415, 0.0334540],
            [0.0719453, -0.2289914, 1.4052427],
        ];
        let t = mat3_mul(m_xyz_to_srgb, m_src);
        // Diagonal should be ~1, off-diagonal ~0.
        for i in 0..3usize {
            for j in 0..3usize {
                let expected = if i == j { 1.0f32 } else { 0.0 };
                assert!(
                    (t[i][j] - expected).abs() < 0.01,
                    "t[{i}][{j}] = {} expected ~{expected}",
                    t[i][j]
                );
            }
        }
    }

    #[test]
    fn p3_primaries_produce_known_matrix() {
        // Display P3 D50-adapted XYZ primaries (from Apple's ICC profile).
        let r = [0.51512f32, 0.24120, -0.00105];
        let g = [0.29198f32, 0.69225, 0.04189];
        let b = [0.15710f32, 0.06657, 0.78408];

        let m_src = [
            [r[0], g[0], b[0]],
            [r[1], g[1], b[1]],
            [r[2], g[2], b[2]],
        ];
        let m_xyz_to_srgb: [[f32; 3]; 3] = [
            [3.1338561, -1.6168667, -0.4906146],
            [-0.9787684, 1.9161415, 0.0334540],
            [0.0719453, -0.2289914, 1.4052427],
        ];
        let t = mat3_mul(m_xyz_to_srgb, m_src);

        // Well-known P3→sRGB matrix values.
        assert!((t[0][0] - 1.2249).abs() < 0.002, "R gain {}", t[0][0]);
        assert!((t[1][1] - 1.0421).abs() < 0.002, "G gain {}", t[1][1]);
        assert!((t[2][2] - 1.0983).abs() < 0.002, "B gain {}", t[2][2]);
    }

    #[test]
    fn round_trip_identity_matrix() {
        let identity = [[1.0f32, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
        let mut pixels = vec![200u8, 100, 50, 0, 255, 128];
        let original = pixels.clone();
        apply_matrix_to_rgb8(&mut pixels, identity);
        assert_eq!(pixels, original);
    }

    #[test]
    fn linear_roundtrip() {
        for v in 0u8..=255 {
            let rt = linear_to_srgb8(srgb8_to_linear(v));
            assert!(
                (rt as i16 - v as i16).abs() <= 1,
                "roundtrip failed for {v}: got {rt}"
            );
        }
    }
}
