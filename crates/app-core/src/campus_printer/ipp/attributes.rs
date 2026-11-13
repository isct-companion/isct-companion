//! The printer's IPP description, built from `CAPABILITIES`.
use super::*;
use crate::campus_printer::Media;
use std::collections::BTreeMap;

const MARGIN: i32 = CAPABILITIES.margin;

fn collection(members: Vec<(&str, IppValue)>) -> Result<IppValue, PrintQueueError> {
    let mut map = BTreeMap::new();
    for (name, value) in members {
        map.insert(name.try_into()?, value);
    }
    Ok(IppValue::Collection(map))
}

fn media_size(media: Media) -> Result<IppValue, PrintQueueError> {
    let (x, y) = media.size();
    collection(vec![
        ("x-dimension", IppValue::Integer(x)),
        ("y-dimension", IppValue::Integer(y)),
    ])
}

/// One sheet as Windows' IPP class driver builds its paper list from it:
/// without `media-col-database` it has no page size to render to, and a job
/// fails in the spooler with nothing sent.
fn media_col(media: Media) -> Result<IppValue, PrintQueueError> {
    collection(vec![
        ("media-size", media_size(media)?),
        ("media-top-margin", IppValue::Integer(MARGIN)),
        ("media-bottom-margin", IppValue::Integer(MARGIN)),
        ("media-left-margin", IppValue::Integer(MARGIN)),
        ("media-right-margin", IppValue::Integer(MARGIN)),
        ("media-source", IppValue::new_keyword("auto")?),
        ("media-type", IppValue::new_keyword("stationery")?),
    ])
}

fn keywords<'a>(names: impl IntoIterator<Item = &'a str>) -> Result<IppValue, PrintQueueError> {
    Ok(IppValue::Array(
        names
            .into_iter()
            .map(IppValue::new_keyword)
            .collect::<Result<_, _>>()?,
    ))
}

pub(super) fn printer(
    response: &mut IppRequestResponse,
    uri: &str,
    printer_uuid: &str,
    uptime: i32,
) -> Result<(), PrintQueueError> {
    let keyword = IppValue::new_keyword;
    let media = CAPABILITIES.media;
    let defaults = CAPABILITIES.defaults;
    let mut values = vec![
        ("printer-uri-supported", IppValue::new_uri(uri)?),
        ("uri-authentication-supported", keyword("none")?),
        ("uri-security-supported", keyword("none")?),
        ("printer-uuid", IppValue::new_uri(printer_uuid.to_string())?),
        (
            "printer-location",
            IppValue::new_text_without_language("Local computer")?,
        ),
        (
            "printer-name",
            IppValue::new_name_without_language("IsctCompanion Campus Printer")?,
        ),
        (
            "printer-info",
            IppValue::new_text_without_language("Campus printer via IsctCompanion")?,
        ),
        (
            "printer-make-and-model",
            IppValue::new_text_without_language("IsctCompanion PDF printer")?,
        ),
        ("printer-state", IppValue::Enum(3)),
        ("printer-state-reasons", keyword("none")?),
        ("printer-is-accepting-jobs", IppValue::Boolean(true)),
        ("printer-up-time", IppValue::Integer(uptime)),
        ("charset-configured", IppValue::new_charset("utf-8")?),
        (
            "charset-supported",
            IppValue::Array(vec![IppValue::new_charset("utf-8")?]),
        ),
        (
            "natural-language-configured",
            IppValue::new_natural_language("en")?,
        ),
        (
            "generated-natural-language-supported",
            IppValue::Array(vec![IppValue::new_natural_language("en")?]),
        ),
        (
            "ipp-versions-supported",
            IppValue::Array(vec![keyword("1.1")?, keyword("2.0")?]),
        ),
        (
            "operations-supported",
            IppValue::Array(
                [2, 4, 5, 6, 8, 9, 10, 11]
                    .into_iter()
                    .map(IppValue::Enum)
                    .collect(),
            ),
        ),
        (
            "document-format-default",
            IppValue::new_mime_media_type("application/pdf")?,
        ),
        (
            "document-format-supported",
            IppValue::Array(vec![IppValue::new_mime_media_type("application/pdf")?]),
        ),
        (
            "compression-supported",
            IppValue::Array(vec![keyword("none")?]),
        ),
        ("multiple-document-jobs-supported", IppValue::Boolean(false)),
        (
            "multiple-document-handling-default",
            keyword("separate-documents-collated-copies")?,
        ),
        (
            "multiple-document-handling-supported",
            IppValue::Array(vec![keyword("separate-documents-collated-copies")?]),
        ),
        ("copies-default", IppValue::Integer(1)),
        (
            "copies-supported",
            IppValue::new_range_of_integer(1, CAPABILITIES.max_copies as i32),
        ),
        ("media-default", keyword(media[0].keyword())?),
        ("media-supported", keywords(media.map(Media::keyword))?),
        (
            "sides-default",
            keyword(super::keyword(&SIDES, defaults.sides))?,
        ),
        ("sides-supported", keywords(SIDES.map(|(_, name)| name))?),
        (
            "print-color-mode-default",
            keyword(super::keyword(&COLORS, defaults.color))?,
        ),
        (
            "print-color-mode-supported",
            keywords(COLORS.map(|(_, name)| name))?,
        ),
        ("color-supported", IppValue::Boolean(true)),
        (
            "printer-resolution-default",
            IppValue::new_resolution(300, 300, 3),
        ),
        (
            "printer-resolution-supported",
            IppValue::Array(vec![IppValue::new_resolution(300, 300, 3)]),
        ),
        // Windows builds the queue's capabilities from these. Without a PDF
        // version list or device ID it falls back to its most limited set.
        (
            "pdf-versions-supported",
            IppValue::Array(vec![keyword("adobe-1.7")?, keyword("iso-32000-1_2008")?]),
        ),
        (
            "printer-device-id",
            IppValue::new_text_without_language(
                "MFG:IsctCompanion;MDL:Campus PDF Printer;CMD:PDF;CLS:PRINTER;",
            )?,
        ),
        ("orientation-requested-default", IppValue::Enum(3)),
        (
            "orientation-requested-supported",
            IppValue::Array([3, 4, 5, 6].into_iter().map(IppValue::Enum).collect()),
        ),
        (
            "media-col-database",
            IppValue::Array(media.into_iter().map(media_col).collect::<Result<_, _>>()?),
        ),
        ("media-col-default", media_col(media[0])?),
        (
            "media-col-ready",
            IppValue::Array(vec![media_col(media[0])?]),
        ),
        (
            "media-ready",
            IppValue::Array(vec![keyword(media[0].keyword())?]),
        ),
        (
            "media-size-supported",
            IppValue::Array(
                media
                    .into_iter()
                    .map(media_size)
                    .collect::<Result<_, _>>()?,
            ),
        ),
        (
            "media-col-supported",
            IppValue::Array(
                [
                    "media-size",
                    "media-top-margin",
                    "media-bottom-margin",
                    "media-left-margin",
                    "media-right-margin",
                    "media-source",
                    "media-type",
                ]
                .into_iter()
                .map(keyword)
                .collect::<Result<_, _>>()?,
            ),
        ),
        (
            "media-top-margin-supported",
            IppValue::Array(vec![IppValue::Integer(MARGIN)]),
        ),
        (
            "media-bottom-margin-supported",
            IppValue::Array(vec![IppValue::Integer(MARGIN)]),
        ),
        (
            "media-left-margin-supported",
            IppValue::Array(vec![IppValue::Integer(MARGIN)]),
        ),
        (
            "media-right-margin-supported",
            IppValue::Array(vec![IppValue::Integer(MARGIN)]),
        ),
        (
            "media-source-supported",
            IppValue::Array(vec![keyword("auto")?]),
        ),
        (
            "media-type-supported",
            IppValue::Array(vec![keyword("stationery")?]),
        ),
        ("output-bin-default", keyword("face-down")?),
        (
            "output-bin-supported",
            IppValue::Array(vec![keyword("face-down")?]),
        ),
        (
            "document-format-preferred",
            IppValue::new_mime_media_type("application/pdf")?,
        ),
        ("print-quality-default", IppValue::Enum(4)),
        (
            "print-quality-supported",
            IppValue::Array(vec![IppValue::Enum(4)]),
        ),
    ];
    values.push((
        "job-creation-attributes-supported",
        IppValue::Array(
            [
                "job-name",
                "document-format",
                "copies",
                "media",
                "media-col",
                "sides",
                "print-color-mode",
            ]
            .into_iter()
            .map(keyword)
            .collect::<Result<_, _>>()?,
        ),
    ));
    for (name, value) in values {
        response.attributes_mut().add(
            DelimiterTag::PrinterAttributes,
            IppAttribute::with_name(name, value)?,
        );
    }
    Ok(())
}
