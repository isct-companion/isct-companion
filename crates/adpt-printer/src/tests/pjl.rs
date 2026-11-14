use super::*;

#[test]
fn detect_recognizes_pdf_and_postscript() {
    assert_eq!(Language::detect(b"%PDF-1.7\n"), Some(Language::Pdf));
    assert_eq!(
        Language::detect(b"%!PS-Adobe-3.0\n"),
        Some(Language::PostScript)
    );
    assert_eq!(Language::detect(b"PK\x03\x04"), None);
    // A prefix before the header is tolerated up to the 1024-byte window.
    assert_eq!(
        Language::detect(b"\xef\xbb\xbf%PDF-1.4"),
        Some(Language::Pdf)
    );
    let mut late = vec![b' '; 1020];
    late.extend_from_slice(b"%PDF-1.4");
    assert_eq!(Language::detect(&late), None);
    assert_eq!(Language::detect(b""), None);
}

// The HP PJL variables, binding only when duplex is on.
#[test]
fn header_states_sides_and_color() {
    let settings = Settings {
        sides: Sides::ShortEdge,
        color: ColorMode::Mono,
    };
    assert_eq!(
        String::from_utf8(header(Language::PostScript, &settings)).unwrap(),
        "\x1b%-12345X@PJL JOB\n\
         @PJL SET DUPLEX=ON\n\
         @PJL SET BINDING=SHORTEDGE\n\
         @PJL SET RENDERMODE=GRAYSCALE\n\
         @PJL ENTER LANGUAGE=POSTSCRIPT\n"
    );
    let settings = Settings {
        sides: Sides::OneSided,
        color: ColorMode::Color,
    };
    let text = String::from_utf8(header(Language::Pdf, &settings)).unwrap();
    assert!(text.contains("@PJL SET DUPLEX=OFF\n"));
    assert!(!text.contains("BINDING"));
    assert!(text.contains("@PJL SET RENDERMODE=COLOR\n"));
    let long = Settings {
        sides: Sides::LongEdge,
        color: ColorMode::Color,
    };
    let text = String::from_utf8(header(Language::Pdf, &long)).unwrap();
    assert!(text.contains("@PJL SET DUPLEX=ON\n@PJL SET BINDING=LONGEDGE\n"));
}
