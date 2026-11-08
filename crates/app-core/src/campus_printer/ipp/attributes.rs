use super::*;

pub(super) fn printer(
    response: &mut IppRequestResponse,
    uri: &str,
    uptime: i32,
) -> Result<(), PrintQueueError> {
    let keyword = IppValue::new_keyword;
    let mut values = vec![
        ("printer-uri-supported", IppValue::new_uri(uri)?),
        ("uri-authentication-supported", keyword("none")?),
        ("uri-security-supported", keyword("none")?),
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
        ("charset-supported", IppValue::new_charset("utf-8")?),
        (
            "natural-language-configured",
            IppValue::new_natural_language("en")?,
        ),
        (
            "generated-natural-language-supported",
            IppValue::new_natural_language("en")?,
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
            IppValue::new_mime_media_type("application/pdf")?,
        ),
        ("compression-supported", keyword("none")?),
        ("multiple-document-jobs-supported", IppValue::Boolean(false)),
        (
            "multiple-document-handling-default",
            keyword("separate-documents-uncollated-copies")?,
        ),
        (
            "multiple-document-handling-supported",
            IppValue::Array(vec![
                keyword("separate-documents-uncollated-copies")?,
                keyword("separate-documents-collated-copies")?,
            ]),
        ),
        ("copies-default", IppValue::Integer(1)),
        ("copies-supported", IppValue::new_range_of_integer(1, 1)),
        ("media-default", keyword("iso_a4_210x297mm")?),
        (
            "media-supported",
            IppValue::Array(vec![
                keyword("iso_a4_210x297mm")?,
                keyword("na_letter_8.5x11in")?,
            ]),
        ),
        ("sides-default", keyword("one-sided")?),
        ("sides-supported", keyword("one-sided")?),
        ("print-color-mode-default", keyword("color")?),
        ("print-color-mode-supported", keyword("color")?),
        ("color-supported", IppValue::Boolean(true)),
        (
            "printer-resolution-default",
            IppValue::new_resolution(300, 300, 3),
        ),
        (
            "printer-resolution-supported",
            IppValue::new_resolution(300, 300, 3),
        ),
        ("print-quality-default", IppValue::Enum(4)),
        ("print-quality-supported", IppValue::Enum(4)),
    ];
    values.push((
        "job-creation-attributes-supported",
        IppValue::Array(
            [
                "job-name",
                "document-format",
                "copies",
                "media",
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
