//! Sharing a past exam, the way ScienceTokyoExam's own uploader does it:
//! the file's SHA-256 is checked against the library first, the site hands
//! out a one-time storage URL, the file goes straight to that storage, and
//! only then is the exam registered. A registration that fails rolls the
//! stored file back so nothing is left without an owner. Several photos
//! become one PDF first, sized like the site's own merge, so a sitting
//! photographed page by page reads as one document.
use super::*;
use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageDecoder, ImageReader, Rgb, RgbImage};
use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Document, Object, Stream};
use sha2::{Digest, Sha256};
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The site's limit per stored file.
pub(super) const LIMIT: usize = 25 * 1024 * 1024;
/// The site's merge: photos scale down to this width and are cut into pages
/// this tall, so a long scroll of a whiteboard stays readable.
const PAGE_WIDTH: u32 = 1600;
const PAGE_HEIGHT: u32 = 2048;
const JPEG_QUALITY: u8 = 90;

const ROLES: &[&str] = &["question", "answer", "both", "notes", "other"];
const OCCASIONS: &[&str] = &[
    "midterm", "final", "makeup", "quiz", "check", "report", "exercise", "textbook", "mock",
    "unknown",
];
/// Occasions held more than once a term, so their number matters.
const NUMBERED: &[&str] = &["quiz", "check", "report"];
const PROVENANCES: &[&str] = &["original", "reconstructed"];
const ANSWER_SOURCES: &[&str] = &["official", "student", "unknown"];
const MAX_SUBJECTS: usize = 10;
const MAX_TEACHERS: usize = 40;
const MAX_NOTE: usize = 40;
const MAX_VARIANT: usize = 100;

/// The file to upload, read and merged.
#[derive(Debug)]
pub(super) struct Prepared {
    pub bytes: Vec<u8>,
    pub name: String,
    pub content_type: &'static str,
    /// What the site deduplicates on: the file as picked for one file, the
    /// merged PDF for several, as its own uploader does.
    pub hash: String,
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Kind {
    Pdf,
    Photo,
    Text,
}

/// The files the site takes, by extension: the shell's picker offers only
/// these.
pub(super) fn kind(path: &Path) -> Option<Kind> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    Some(match extension.as_str() {
        "pdf" => Kind::Pdf,
        "jpg" | "jpeg" | "png" | "webp" => Kind::Photo,
        "txt" => Kind::Text,
        _ => return None,
    })
}

/// A PDF or text file goes up as it is. Photos become a PDF, one or many:
/// the site stores no JPEG or PNG (its uploader re-encodes them to WebP,
/// which this build cannot write lossily), and a PDF reads the same on
/// every platform.
pub(super) fn prepare(paths: &[PathBuf]) -> Result<Prepared, ExamError> {
    let read = |path: &PathBuf| std::fs::read(path).map_err(|_| ExamError::Storage);
    let prepared = match paths {
        [] => return Err(ExamError::InvalidInput),
        [path] => {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or(ExamError::InvalidInput)?;
            let bytes = read(path)?;
            if bytes.is_empty() {
                return Err(ExamError::InvalidInput);
            }
            let hash = sha256_hex(&bytes);
            match kind(path).ok_or(ExamError::InvalidInput)? {
                Kind::Pdf => Prepared {
                    bytes,
                    name: name.to_owned(),
                    content_type: "application/pdf",
                    hash,
                },
                Kind::Text => Prepared {
                    bytes,
                    name: name.to_owned(),
                    content_type: "text/plain",
                    hash,
                },
                Kind::Photo => Prepared {
                    bytes: merge(&[bytes])?,
                    name: format!(
                        "{}.pdf",
                        Path::new(name)
                            .file_stem()
                            .and_then(|stem| stem.to_str())
                            .unwrap_or("photo")
                    ),
                    content_type: "application/pdf",
                    hash,
                },
            }
        }
        many => {
            if !many.iter().all(|path| kind(path) == Some(Kind::Photo)) {
                return Err(ExamError::InvalidInput);
            }
            let photos = many.iter().map(read).collect::<Result<Vec<_>, _>>()?;
            let bytes = merge(&photos)?;
            Prepared {
                hash: sha256_hex(&bytes),
                bytes,
                name: "merged_document.pdf".to_owned(),
                content_type: "application/pdf",
            }
        }
    };
    if prepared.bytes.is_empty() {
        return Err(ExamError::InvalidInput);
    }
    if prepared.bytes.len() > LIMIT {
        return Err(ExamError::TooLarge);
    }
    Ok(prepared)
}

/// Reads a photo upright: phones store the camera's rotation as EXIF
/// orientation, which a browser applies when it draws the photo.
fn upright(bytes: &[u8]) -> Result<DynamicImage, ExamError> {
    let mut decoder = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| ExamError::InvalidInput)?
        .into_decoder()
        .map_err(|_| ExamError::InvalidInput)?;
    let orientation = decoder.orientation().map_err(|_| ExamError::InvalidInput)?;
    let mut image = DynamicImage::from_decoder(decoder).map_err(|_| ExamError::InvalidInput)?;
    image.apply_orientation(orientation);
    Ok(image)
}

/// Photos on white (transparent PNGs too), each scaled to at most
/// `PAGE_WIDTH` and cut into pages at most `PAGE_HEIGHT` tall, one JPEG per
/// page sized in pixels, as the site's merge does.
pub(super) fn merge(photos: &[Vec<u8>]) -> Result<Vec<u8>, ExamError> {
    let mut document = Document::with_version("1.5");
    let pages_id = document.new_object_id();
    let mut kids = Vec::new();
    for photo in photos {
        let image = upright(photo)?;
        let (width, height) = (image.width(), image.height());
        if width == 0 || height == 0 {
            return Err(ExamError::InvalidInput);
        }
        let scale = f64::from(PAGE_WIDTH.min(width)) / f64::from(width);
        let slice = ((f64::from(PAGE_HEIGHT) / scale).floor() as u32).max(1);
        let page_width = ((f64::from(width) * scale).round() as u32).max(1);
        let rgba = image.to_rgba8();
        let mut top = 0;
        while top < height {
            let rows = slice.min(height - top);
            let page_height = ((f64::from(rows) * scale).round() as u32).max(1);
            let part = image::imageops::crop_imm(&rgba, 0, top, width, rows).to_image();
            let part = image::imageops::resize(
                &part,
                page_width,
                page_height,
                image::imageops::FilterType::Triangle,
            );
            let mut flat = RgbImage::from_pixel(page_width, page_height, Rgb([255, 255, 255]));
            for (x, y, pixel) in part.enumerate_pixels() {
                let alpha = u16::from(pixel[3]);
                let blend =
                    |channel: u8| ((u16::from(channel) * alpha + 255 * (255 - alpha)) / 255) as u8;
                flat.put_pixel(
                    x,
                    y,
                    Rgb([blend(pixel[0]), blend(pixel[1]), blend(pixel[2])]),
                );
            }
            let mut jpeg = Vec::new();
            JpegEncoder::new_with_quality(&mut jpeg, JPEG_QUALITY)
                .encode_image(&flat)
                .map_err(|_| ExamError::InvalidInput)?;
            let mut picture = Stream::new(
                dictionary! {
                    "Type" => "XObject",
                    "Subtype" => "Image",
                    "Width" => page_width,
                    "Height" => page_height,
                    "ColorSpace" => "DeviceRGB",
                    "BitsPerComponent" => 8,
                    "Filter" => "DCTDecode",
                },
                jpeg,
            );
            // Already JPEG; deflating it again only costs time.
            picture.allows_compression = false;
            let picture_id = document.add_object(picture);
            let content = Content {
                operations: vec![
                    Operation::new("q", vec![]),
                    Operation::new(
                        "cm",
                        vec![
                            page_width.into(),
                            0.into(),
                            0.into(),
                            page_height.into(),
                            0.into(),
                            0.into(),
                        ],
                    ),
                    Operation::new("Do", vec!["Im0".into()]),
                    Operation::new("Q", vec![]),
                ],
            };
            let content_id = document.add_object(Stream::new(
                dictionary! {},
                content.encode().map_err(|_| ExamError::InvalidInput)?,
            ));
            kids.push(Object::from(document.add_object(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "MediaBox" => vec![0.into(), 0.into(), page_width.into(), page_height.into()],
                "Resources" => dictionary! { "XObject" => dictionary! { "Im0" => picture_id } },
                "Contents" => content_id,
            })));
            top += rows;
        }
    }
    let count = kids.len() as i64;
    document.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => count }),
    );
    let catalog = document.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
    document.trailer.set("Root", catalog);
    let mut bytes = Vec::new();
    document
        .save_to(&mut bytes)
        .map_err(|_| ExamError::InvalidInput)?;
    Ok(bytes)
}

/// The registration fields the site expects, checked with its own rules:
/// notes and other material belong to no sitting, only quizzes, checks and
/// reports are numbered, a question says where it came from and an answer
/// who wrote it.
pub(super) fn fields(draft: &ExamUploadDraft) -> Result<Value, ExamError> {
    let invalid = || ExamError::InvalidInput;
    let role = draft.role.as_str();
    if !ROLES.contains(&role) {
        return Err(invalid());
    }
    let subjects: Vec<&str> = draft
        .subject_codes
        .iter()
        .map(|code| code.trim())
        .filter(|code| !code.is_empty())
        .collect();
    if subjects.is_empty() || subjects.len() > MAX_SUBJECTS {
        return Err(invalid());
    }
    let mut teachers: Vec<&str> = Vec::new();
    for id in &draft.teacher_ids {
        if !teachers.contains(&id.as_str()) {
            teachers.push(id);
        }
    }
    if teachers.len() > MAX_TEACHERS {
        return Err(invalid());
    }
    let sitting = !matches!(role, "notes" | "other");
    let occasion = if sitting {
        let occasion = draft.occasion.as_deref().ok_or_else(invalid)?;
        if !OCCASIONS.contains(&occasion) {
            return Err(invalid());
        }
        occasion
    } else {
        "none"
    };
    let occasion_number = if NUMBERED.contains(&occasion) {
        draft.occasion_number
    } else {
        0
    };
    let variant = if sitting { draft.variant.trim() } else { "" };
    let note = draft.note.trim();
    if variant.chars().count() > MAX_VARIANT || note.chars().count() > MAX_NOTE {
        return Err(invalid());
    }
    let provenance = if matches!(role, "question" | "both") {
        let provenance = draft.provenance.as_deref().ok_or_else(invalid)?;
        if !PROVENANCES.contains(&provenance) {
            return Err(invalid());
        }
        provenance
    } else {
        "original"
    };
    let answer_source = if matches!(role, "answer" | "both") {
        let source = draft.answer_source.as_deref().unwrap_or("unknown");
        if !ANSWER_SOURCES.contains(&source) {
            return Err(invalid());
        }
        Some(source)
    } else {
        None
    };
    Ok(json!({
        "subjectCodes": subjects,
        "teacherIds": teachers,
        "newTeachers": [],
        "academicYear": draft.academic_year,
        "occasion": occasion,
        "occasionNumber": occasion_number,
        "variant": variant,
        "role": role,
        "provenance": provenance,
        "answerSource": answer_source,
        "note": (!note.is_empty()).then_some(note),
    }))
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Only the site's own storage receives the file.
fn storage_url(value: &str) -> Result<url::Url, ExamError> {
    let url = url::Url::parse(value).map_err(|_| ExamError::Response)?;
    let trusted = url.scheme() == "https"
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url
            .host_str()
            .is_some_and(|host| host.ends_with(".r2.cloudflarestorage.com"));
    trusted.then_some(url).ok_or(ExamError::Response)
}

impl ExamShare {
    /// Shares the draft's files; answers the new exam's ID.
    pub(super) async fn upload(
        &self,
        session: &mut Session,
        draft: ExamUploadDraft,
    ) -> Result<String, ExamError> {
        let fields = fields(&draft)?;
        let paths: Vec<PathBuf> = draft.files.iter().map(PathBuf::from).collect();
        let file = tokio::task::spawn_blocking(move || prepare(&paths))
            .await
            .map_err(|_| ExamError::Storage)??;
        let hash = file.hash.clone();

        #[derive(Deserialize)]
        struct Exists {
            exists: bool,
        }
        let exists: Exists = self
            .call(
                session,
                self.service(Method::POST, &["exams", "hash-exists"])?
                    .json(&json!({ "fileHash": hash })),
            )
            .await?;
        if exists.exists {
            return Err(ExamError::Duplicate);
        }

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Authorized {
            upload_url: String,
            content_type: String,
            storage_path: String,
        }
        let size = file.bytes.len();
        let authorized: Authorized = self
            .call(
                session,
                self.service(Method::POST, &["exams", "uploads", "authorize"])?
                    .json(&json!({
                        "fileName": file.name,
                        "contentType": file.content_type,
                        "size": size,
                    })),
            )
            .await?;
        let response = self
            .api
            .http
            .put(storage_url(&authorized.upload_url)?)
            .header(reqwest::header::CONTENT_TYPE, &authorized.content_type)
            // The site's uploader never overwrites a stored file.
            .header(reqwest::header::IF_NONE_MATCH, "*")
            .timeout(Duration::from_secs(300))
            .body(file.bytes)
            .send()
            .await
            .map_err(client::network)?;
        if !response.status().is_success() {
            log::warn!("[exam-share] file upload: {}", response.status());
            return Err(ExamError::Unavailable);
        }

        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Created {
            exam_id: String,
        }
        let mut body = fields;
        if let Value::Object(map) = &mut body {
            map.insert("storagePath".into(), json!(authorized.storage_path));
            map.insert("fileName".into(), json!(file.name));
            map.insert("fileSize".into(), json!(size));
            map.insert("fileType".into(), json!(file.content_type));
            map.insert("fileHash".into(), json!(hash));
            map.insert("requestId".into(), json!(draft.request_id));
        }
        let created: Result<Created, _> = self
            .call(
                session,
                self.service(Method::POST, &["exams"])?
                    .timeout(Duration::from_secs(60))
                    .json(&body),
            )
            .await;
        match created {
            Ok(created) => Ok(created.exam_id),
            Err(error) => {
                let rollback = self
                    .call::<Value>(
                        session,
                        self.service(Method::POST, &["exams", "uploads", "rollback"])?
                            .json(&json!({ "storagePath": authorized.storage_path })),
                    )
                    .await;
                if let Err(rollback) = rollback {
                    log::warn!("[exam-share] upload rollback: {rollback}");
                }
                Err(error)
            }
        }
    }
}

#[cfg(test)]
#[path = "../tests/exam_share__upload.rs"]
mod tests;
