use std::io::Write;

fn crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    for (n, slot) in t.iter_mut().enumerate() {
        let mut c = n as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *slot = c;
    }
    t
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8], table: &[u32; 256]) {
    out.extend((data.len() as u32).to_be_bytes());
    out.extend(kind);
    out.extend(data);
    let mut c = 0xFFFF_FFFFu32;
    for &b in kind.iter().chain(data) {
        c = table[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
    }
    out.extend((c ^ 0xFFFF_FFFF).to_be_bytes());
}

pub fn rgba(width: usize, height: usize, pixels: &[u8]) -> Result<Vec<u8>, String> {
    if pixels.len() != width * height * 4 || width == 0 || height == 0 {
        return Err(format!("{} bytes do not make a {width}x{height} RGBA image", pixels.len()));
    }
    let mut raw = Vec::with_capacity((width * 4 + 1) * height);
    for row in pixels.chunks(width * 4) {
        raw.push(0);
        raw.extend(row);
    }
    let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(&raw).map_err(|e| e.to_string())?;
    let data = z.finish().map_err(|e| e.to_string())?;
    let table = crc_table();
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend((width as u32).to_be_bytes());
    ihdr.extend((height as u32).to_be_bytes());
    ihdr.extend([8, 6, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr, &table);
    chunk(&mut out, b"IDAT", &data, &table);
    chunk(&mut out, b"IEND", &[], &table);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_crc_matches_the_png_reference_value() {
        let table = crc_table();
        let mut out = Vec::new();
        chunk(&mut out, b"IEND", &[], &table);
        assert_eq!(&out[8..], &[0xAE, 0x42, 0x60, 0x82]);
    }

    #[test]
    fn an_image_round_trips_through_zlib() {
        let px: Vec<u8> = (0..2 * 3 * 4).map(|i| i as u8).collect();
        let png = rgba(2, 3, &px).unwrap();
        assert_eq!(&png[..8], &[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]);
        let idat = png.windows(4).position(|w| w == b"IDAT").unwrap();
        let len = u32::from_be_bytes(png[idat - 4..idat].try_into().unwrap()) as usize;
        let mut back = Vec::new();
        std::io::Read::read_to_end(&mut flate2::read::ZlibDecoder::new(&png[idat + 4..idat + 4 + len]), &mut back).unwrap();
        assert_eq!(back.len(), 3 * (2 * 4 + 1));
        assert_eq!(&back[1..9], &px[..8]);
        assert!(rgba(2, 2, &px).is_err());
    }
}
