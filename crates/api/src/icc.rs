/// ICC profile helpers.

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

pub fn profile_description(icc: &[u8]) -> Option<String> {
    let tag_data = find_tag(icc, b"desc")?;
    if tag_data.len() < 8 {
        return None;
    }

    match &tag_data[0..4] {
        b"desc" => {
            if tag_data.len() < 12 {
                return None;
            }
            let count = u32::from_be_bytes(tag_data[8..12].try_into().ok()?) as usize;
            if count == 0 || tag_data.len() < 12 + count {
                return None;
            }
            let raw = &tag_data[12..12 + count];
            let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
            Some(String::from_utf8_lossy(&raw[..end]).trim().to_owned())
        }
        b"mluc" => {
            if tag_data.len() < 16 {
                return None;
            }
            let record_count = u32::from_be_bytes(tag_data[8..12].try_into().ok()?) as usize;
            let mut chosen: Option<(usize, usize)> = None;
            for i in 0..record_count {
                let base = 16 + i * 12;
                if base + 12 > tag_data.len() {
                    break;
                }
                let lang = &tag_data[base..base + 2];
                let len =
                    u32::from_be_bytes(tag_data[base + 4..base + 8].try_into().ok()?) as usize;
                let offset =
                    u32::from_be_bytes(tag_data[base + 8..base + 12].try_into().ok()?) as usize;
                if offset + len > tag_data.len() {
                    continue;
                }
                if chosen.is_none() {
                    chosen = Some((offset, len));
                }
                if lang == b"en" {
                    chosen = Some((offset, len));
                    break;
                }
            }
            let (offset, len) = chosen?;
            let utf16: Vec<u16> = tag_data[offset..offset + len]
                .chunks_exact(2)
                .map(|c| u16::from_be_bytes([c[0], c[1]]))
                .collect();
            Some(
                String::from_utf16_lossy(&utf16)
                    .trim_matches('\0')
                    .trim()
                    .to_owned(),
            )
        }
        _ => None,
    }
}

pub fn apply_icc_to_srgb_rgb8(pixels: &mut [u8], icc: &[u8]) -> bool {
    let Ok(src_profile) = lcms2::Profile::new_icc(icc) else {
        return false;
    };
    let dst_profile = lcms2::Profile::new_srgb();
    let Ok(transform) = lcms2::Transform::new(
        &src_profile,
        lcms2::PixelFormat::RGB_8,
        &dst_profile,
        lcms2::PixelFormat::RGB_8,
        lcms2::Intent::Perceptual,
    ) else {
        return false;
    };
    transform.transform_in_place(pixels);
    true
}
