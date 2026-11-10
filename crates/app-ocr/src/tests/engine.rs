use super::*;

#[test]
fn decoding_limits_alpha_and_preserves_source_resolution() {
    assert!(decode_image(b"not an image").is_err());
    assert!(decode_image(&[]).is_err());
    let image = image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 0, 0, 0]));
    let mut bytes = Cursor::new(vec![]);
    image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
    // Transparent pixels composite onto white paper.
    assert_eq!(
        decode_image(bytes.get_ref()).unwrap().get_pixel(0, 0).0,
        [255; 3]
    );
    let source = RgbImage::from_pixel(2400, 1200, Rgb([17, 29, 41]));
    let mut png = Cursor::new(vec![]);
    source.write_to(&mut png, image::ImageFormat::Png).unwrap();
    let decoded = decode_image(png.get_ref()).unwrap();
    assert_eq!(decoded.dimensions(), (2400, 1200));
    assert_eq!(decoded.get_pixel(0, 0).0, [17, 29, 41]);
    assert!(matches!(
        decode_image(&vec![0; MAX_IMAGE_BYTES + 1]),
        Err(MatrixOcrError::InvalidImage(_))
    ));
}
