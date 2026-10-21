pub fn decode_rgba(bytes: &[u8]) -> Result<(usize, usize, Vec<u8>), String> {
    let mut decoder = jpeg_decoder::Decoder::new(std::io::Cursor::new(bytes));
    let pixels = decoder.decode().map_err(|e| e.to_string())?;
    let info = decoder.info().ok_or("the JPEG has no image header")?;
    let (w, h) = (info.width as usize, info.height as usize);
    let rgba: Vec<u8> = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => pixels.chunks_exact(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        jpeg_decoder::PixelFormat::L8 => pixels.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        jpeg_decoder::PixelFormat::L16 => pixels.chunks_exact(2).flat_map(|c| [c[0], c[0], c[0], 255]).collect(),
        jpeg_decoder::PixelFormat::CMYK32 => return Err("a CMYK JPEG is not read (convert it to RGB)".into()),
    };
    if rgba.len() != w * h * 4 {
        return Err(format!("the JPEG decoded to {} bytes for {w}x{h}", rgba.len()));
    }
    Ok((w, h, rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(img: &(usize, usize, Vec<u8>), x: usize, y: usize) -> [u8; 3] {
        let i = (y * img.0 + x) * 4;
        [img.2[i], img.2[i + 1], img.2[i + 2]]
    }

    fn near(a: [u8; 3], b: [u8; 3]) -> bool {
        a.iter().zip(b).all(|(&p, q)| (p as i32 - q as i32).abs() <= 24)
    }

    #[test]
    fn colour_and_grey_jpegs_decode_to_rgba_the_right_way_up() {
        let img = decode_rgba(include_bytes!("../testdata/quadrants.jpg")).unwrap();
        assert_eq!((img.0, img.1, img.2.len()), (16, 16, 16 * 16 * 4));
        assert!(near(at(&img, 3, 3), [255, 0, 0]), "top left red: {:?}", at(&img, 3, 3));
        assert!(near(at(&img, 12, 3), [0, 255, 0]), "top right green: {:?}", at(&img, 12, 3));
        assert!(near(at(&img, 3, 12), [0, 0, 255]), "bottom left blue: {:?}", at(&img, 3, 12));
        assert!(near(at(&img, 12, 12), [255, 255, 255]), "bottom right white: {:?}", at(&img, 12, 12));
        assert!(img.2.chunks_exact(4).all(|p| p[3] == 255));
        let grey = decode_rgba(include_bytes!("../testdata/grey.jpg")).unwrap();
        assert_eq!((grey.0, grey.1), (8, 8));
        assert!(near(at(&grey, 4, 4), [128, 128, 128]));
        assert!(decode_rgba(b"not a jpeg").is_err());
    }
}
