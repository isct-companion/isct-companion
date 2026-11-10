use super::*;

#[test]
fn detect_recognizes_pdf_and_postscript() {
    assert_eq!(Language::detect(b"%PDF-1.7\n"), Some(Language::Pdf));
    assert_eq!(
        Language::detect(b"%!PS-Adobe-3.0\n"),
        Some(Language::PostScript)
    );
    assert_eq!(Language::detect(b"PK\x03\x04"), None);
    assert_eq!(Language::detect(b""), None);
}

// Unset settings send nothing, so the printer's defaults apply; set ones
// use the HP PJL variables, binding only when duplex is on.
#[test]
fn header_sends_only_the_settings_given() {
    assert_eq!(
        header(Language::Pdf, &Settings::default()),
        b"\x1b%-12345X@PJL JOB\n@PJL ENTER LANGUAGE=PDF\n".to_vec()
    );
    let settings = Settings {
        duplex: Some(Duplex::ShortEdge),
        color: Some(ColorMode::Mono),
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
        duplex: Some(Duplex::Off),
        color: Some(ColorMode::Color),
    };
    let text = String::from_utf8(header(Language::Pdf, &settings)).unwrap();
    assert!(text.contains("@PJL SET DUPLEX=OFF\n"));
    assert!(!text.contains("BINDING"));
    assert!(text.contains("@PJL SET RENDERMODE=COLOR\n"));
    let long = Settings {
        duplex: Some(Duplex::LongEdge),
        color: None,
    };
    let text = String::from_utf8(header(Language::Pdf, &long)).unwrap();
    assert!(text.contains("@PJL SET DUPLEX=ON\n@PJL SET BINDING=LONGEDGE\n"));
    assert!(!text.contains("RENDERMODE"));
}
