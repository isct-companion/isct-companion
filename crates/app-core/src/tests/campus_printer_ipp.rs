use super::*;

const KEYWORD: u8 = 0x44;
const MIME: u8 = 0x49;
const INTEGER: u8 = 0x21;

/// A request as Windows sends it on the wire, parsed by the same parser the
/// receiver uses, so the tests read attributes exactly as `handle` does.
fn request(attributes: &[(u8, &str, Vec<u8>)]) -> IppRequestResponse {
    let mut bytes = vec![0x02, 0x00, 0x00, 0x05, 0, 0, 0, 1, 0x01];
    let preamble = [
        (0x47, "attributes-charset", b"utf-8".to_vec()),
        (0x48, "attributes-natural-language", b"en".to_vec()),
    ];
    for (tag, name, value) in preamble.iter().chain(attributes) {
        bytes.push(*tag);
        bytes.extend_from_slice(&(name.len() as u16).to_be_bytes());
        bytes.extend_from_slice(name.as_bytes());
        bytes.extend_from_slice(&(value.len() as u16).to_be_bytes());
        bytes.extend_from_slice(value);
    }
    bytes.push(0x03);
    IppParser::new(Cursor::new(bytes)).parse().unwrap()
}

fn keyword<'a>(name: &'a str, value: &str) -> (u8, &'a str, Vec<u8>) {
    (KEYWORD, name, value.as_bytes().to_vec())
}

fn copies(n: i32) -> (u8, &'static str, Vec<u8>) {
    (INTEGER, "copies", n.to_be_bytes().to_vec())
}

fn settings(duplex: Duplex, color: ColorMode) -> Settings {
    Settings {
        duplex: Some(duplex),
        color: Some(color),
    }
}

// A job with no settings prints like the queue's defaults: one collated
// copy, long-edge duplex, monochrome.
#[test]
fn a_job_without_settings_gets_the_queue_defaults() {
    let options = options(&request(&[])).unwrap();
    assert_eq!(options.copies, 1);
    assert_eq!(
        options.settings,
        settings(Duplex::LongEdge, ColorMode::Mono)
    );
}

// What the dialog asked for is honoured where the printer offers it.
#[test]
fn offered_settings_are_honoured() {
    let options = options(&request(&[
        copies(3),
        keyword("sides", "two-sided-short-edge"),
        keyword("print-color-mode", "color"),
    ]))
    .unwrap();
    assert_eq!(options.copies, 3);
    assert_eq!(
        options.settings,
        settings(Duplex::ShortEdge, ColorMode::Color)
    );
    let one_sided = options_of(&[keyword("sides", "one-sided")]);
    assert_eq!(one_sided.settings.duplex, Some(Duplex::Off));
}

fn options_of(attributes: &[(u8, &str, Vec<u8>)]) -> PrintOptions {
    options(&request(attributes)).unwrap()
}

// A value the printer does not offer never fails the job (Windows would
// show it as an error and the person would lose it): it falls back to the
// default, and copies are clamped to what LPD repeats. Copies are always
// collated, whatever the dialog's collate box said.
#[test]
fn unoffered_settings_fall_back_instead_of_failing_the_job() {
    let options = options_of(&[
        copies(250),
        keyword("sides", "tumble"),
        keyword("print-color-mode", "auto"),
        keyword(
            "multiple-document-handling",
            "separate-documents-uncollated-copies",
        ),
    ]);
    assert_eq!(options.copies, MAX_COPIES);
    assert_eq!(
        options.settings,
        settings(Duplex::LongEdge, ColorMode::Mono)
    );
    assert_eq!(options_of(&[copies(0)]).copies, 1);
    assert_eq!(
        options_of(&[keyword("print-color-mode", "bi-level")])
            .settings
            .color,
        Some(ColorMode::Mono)
    );
}

// Only a PDF (or bytes a client did not name) is accepted; another format
// or a compressed upload is refused before a job exists.
#[test]
fn only_uncompressed_pdf_documents_are_accepted() {
    for format in ["application/pdf", "application/octet-stream"] {
        assert!(options(&request(&[(MIME, "document-format", format.into())])).is_ok());
    }
    assert!(matches!(
        options(&request(&[(
            MIME,
            "document-format",
            b"image/pwg-raster".to_vec()
        )])),
        Err(StatusCode::ClientErrorDocumentFormatNotSupported)
    ));
    assert!(matches!(
        options(&request(&[keyword("compression", "gzip")])),
        Err(StatusCode::ClientErrorCompressionNotSupported)
    ));
}
