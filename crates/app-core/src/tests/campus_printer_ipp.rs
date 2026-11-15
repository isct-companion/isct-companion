use super::*;
use crate::{
    data::DataStore,
    ports::{SecretError, SecretVault},
    school_auth::SchoolAuthService,
};

const KEYWORD: u8 = 0x44;
const MIME: u8 = 0x49;
const INTEGER: u8 = 0x21;
const NAME: u8 = 0x42;

/// A request as Windows sends it on the wire: `operation`, its attributes,
/// then the document.
fn wire(operation: u16, attributes: &[(u8, &str, Vec<u8>)], document: &[u8]) -> Bytes {
    let mut bytes = vec![0x02, 0x00];
    bytes.extend_from_slice(&operation.to_be_bytes());
    bytes.extend_from_slice(&[0, 0, 0, 1, 0x01]);
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
    bytes.extend_from_slice(document);
    bytes.into()
}

/// The job options a Print-Job with `attributes` asks for, read exactly as
/// `handle` reads them.
fn options_of(attributes: &[(u8, &str, Vec<u8>)]) -> Result<PrintOptions, StatusCode> {
    options(&parse(wire(2, attributes, b"")).unwrap().attributes)
}

fn keyword<'a>(name: &'a str, value: &str) -> (u8, &'a str, Vec<u8>) {
    (KEYWORD, name, value.as_bytes().to_vec())
}

fn copies(n: i32) -> (u8, &'static str, Vec<u8>) {
    (INTEGER, "copies", n.to_be_bytes().to_vec())
}

fn settings(sides: Sides, color: ColorMode) -> Settings {
    Settings { sides, color }
}

// A job with no settings prints like the printer's defaults: one collated
// copy, long-edge duplex, monochrome.
#[test]
fn a_job_without_settings_gets_the_printer_defaults() {
    let options = options_of(&[]).unwrap();
    assert_eq!(options.copies, 1);
    assert_eq!(options.settings, settings(Sides::LongEdge, ColorMode::Mono));
}

// What the dialog asked for is honoured where the printer offers it, in the
// one sides and color model every platform maps into.
#[test]
fn offered_settings_are_honoured() {
    let options = options_of(&[
        copies(3),
        keyword("sides", "two-sided-short-edge"),
        keyword("print-color-mode", "color"),
    ])
    .unwrap();
    assert_eq!(options.copies, 3);
    assert_eq!(
        options.settings,
        settings(Sides::ShortEdge, ColorMode::Color)
    );
    let one_sided = options_of(&[keyword("sides", "one-sided")]).unwrap();
    assert_eq!(one_sided.settings.sides, Sides::OneSided);
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
    ])
    .unwrap();
    assert_eq!(options.copies, CAPABILITIES.max_copies);
    assert_eq!(options.settings, settings(Sides::LongEdge, ColorMode::Mono));
    assert_eq!(options_of(&[copies(0)]).unwrap().copies, 1);
    assert_eq!(
        options_of(&[keyword("print-color-mode", "bi-level")])
            .unwrap()
            .settings
            .color,
        ColorMode::Mono
    );
}

// Only a PDF (or bytes a client did not name) is accepted; another format
// or a compressed upload is refused before a job exists.
#[test]
fn only_uncompressed_pdf_documents_are_accepted() {
    for format in ["application/pdf", "application/octet-stream"] {
        assert!(options_of(&[(MIME, "document-format", format.into())]).is_ok());
    }
    assert!(matches!(
        options_of(&[(MIME, "document-format", b"image/pwg-raster".to_vec())]),
        Err(StatusCode::ClientErrorDocumentFormatNotSupported)
    ));
    assert!(matches!(
        options_of(&[keyword("compression", "gzip")]),
        Err(StatusCode::ClientErrorCompressionNotSupported)
    ));
}

// The document is exactly the bytes after the attributes; a wrong offset
// would send a corrupt PDF.
#[test]
fn the_document_starts_after_the_attributes() {
    let request = parse(wire(2, &[copies(2)], b"%PDF-1.7 body")).unwrap();
    assert_eq!(&request.document[..], b"%PDF-1.7 body");
}

struct NoAccounts;

impl SecretVault for NoAccounts {
    fn store(&self, _: &str, _: &str) -> Result<(), SecretError> {
        Ok(())
    }
    fn load(&self, _: &str) -> Result<Option<String>, SecretError> {
        Ok(None)
    }
    fn delete(&self, _: &str) -> Result<(), SecretError> {
        Ok(())
    }
}

fn job_attribute<'a>(reply: &'a IppRequestResponse, name: &str) -> Option<&'a IppValue> {
    reply
        .attributes()
        .groups_of(DelimiterTag::JobAttributes)
        .find_map(|group| group.get(name))
        .map(IppAttribute::value)
}

// Print-Job answers once the send has ended: a job that did not print is
// aborted with Core's words for why, stays so for Windows' polls, and is
// handed back for the person's notification.
#[tokio::test]
async fn a_failed_send_aborts_the_job_with_cores_message() {
    let path = std::env::temp_dir().join(format!("campus-ipp-{}", uuid::Uuid::new_v4()));
    let store = DataStore::initialize_dir(path).unwrap().0;
    let core = Arc::new(Core::new(
        store,
        Arc::new(SchoolAuthService::new(NoAccounts)),
    ));
    let queue = CampusPrintQueue::new(
        core,
        "ipp://127.0.0.1:21000/ipp/test".into(),
        "urn:uuid:test".into(),
        Box::new(|message| message.key.to_string()),
    );
    let reply = queue
        .handle(wire(
            2,
            &[(NAME, "job-name", b"report".to_vec())],
            b"%PDF-1.7 body",
        ))
        .await
        .unwrap();
    let saved = "tools.campusPrinter.saveTheTokyoTechAccount";
    assert_eq!(
        reply.failed,
        Some(Failure {
            title: "report".into(),
            text: saved.into()
        })
    );
    let answer = IppParser::new(Cursor::new(reply.body)).parse().unwrap();
    assert_eq!(
        answer.header().operation_or_status,
        StatusCode::SuccessfulOk as i16
    );
    assert_eq!(
        job_attribute(&answer, "job-state"),
        Some(&IppValue::Enum(JobState::Aborted.code()))
    );
    let Some(IppValue::Integer(id)) = job_attribute(&answer, "job-id").cloned() else {
        panic!("no job-id");
    };

    let poll = queue
        .handle(wire(
            9,
            &[(INTEGER, "job-id", id.to_be_bytes().to_vec())],
            b"",
        ))
        .await
        .unwrap();
    assert_eq!(poll.failed, None);
    let poll = IppParser::new(Cursor::new(poll.body)).parse().unwrap();
    assert_eq!(
        job_attribute(&poll, "job-state"),
        Some(&IppValue::Enum(JobState::Aborted.code()))
    );
    assert_eq!(
        job_attribute(&poll, "job-state-message").map(ToString::to_string),
        Some(saved.to_string())
    );
}
