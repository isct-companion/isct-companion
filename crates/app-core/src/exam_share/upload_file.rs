use super::{ExamError, ExamUploadFile};
use image::{codecs::jpeg::JpegEncoder, DynamicImage, ImageDecoder, ImageFormat, ImageReader};
use pdf_writer::{Content, Filter, Finish, Name, Pdf, Rect, Ref};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Cursor, Read},
    path::Path,
};
use tokio_util::sync::CancellationToken;

use super::client::FILE_LIMIT as LIMIT;

pub(super) struct Prepared {
    pub name: String,
    pub content_type: &'static str,
    pub hash: String,
    pub bytes: Vec<u8>,
}

pub(super) fn prepare(
    files: &[ExamUploadFile],
    cancel: &CancellationToken,
) -> Result<Prepared, ExamError> {
    let (name, content_type, bytes, hash) = match files {
        [] => return Err(ExamError::UploadFile),
        [file] => single(file, cancel)?,
        files => {
            let bytes = image_pdf(files, cancel)?;
            ("materials.pdf".into(), "application/pdf", bytes, None)
        }
    };
    if cancel.is_cancelled() {
        return Err(ExamError::Cancelled);
    }
    check_size(bytes.len())?;
    let hash = hash.unwrap_or_else(|| data_encoding::HEXLOWER.encode(&Sha256::digest(&bytes)));
    Ok(Prepared {
        name,
        content_type,
        hash,
        bytes,
    })
}

fn read(file: &ExamUploadFile) -> Result<Vec<u8>, ExamError> {
    let mut data = Vec::new();
    File::open(&file.path)
        .and_then(|file| file.take(LIMIT as u64 + 1).read_to_end(&mut data))
        .map_err(|_| ExamError::UploadFile)?;
    check_size(data.len())?;
    Ok(data)
}

fn check_size(size: usize) -> Result<(), ExamError> {
    match size {
        0 => Err(ExamError::UploadFile),
        size if size > LIMIT => Err(ExamError::UploadSize),
        _ => Ok(()),
    }
}

type SingleFile = (String, &'static str, Vec<u8>, Option<String>);

fn single(file: &ExamUploadFile, cancel: &CancellationToken) -> Result<SingleFile, ExamError> {
    let data = read(file)?;
    let name = file.name.trim();
    if name.is_empty() || name.encode_utf16().count() > 255 {
        return Err(ExamError::UploadFile);
    }
    if data.starts_with(b"%PDF-") {
        return Ok((name.into(), "application/pdf", data, None));
    }
    if [Path::new(name), Path::new(&file.path)]
        .into_iter()
        .any(|source| {
            source
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| {
                    ["txt", "md", "markdown"]
                        .iter()
                        .any(|allowed| ext.eq_ignore_ascii_case(allowed))
                })
        })
        && std::str::from_utf8(&data).is_ok()
    {
        return Ok((name.into(), "text/plain", data, None));
    }
    if photo_format(&data).is_none() {
        return Err(ExamError::UploadFile);
    }
    // The site stores a photo only as WebP, which its own uploader encodes in
    // the browser; a JPEG is refused. One photo goes up as a one-page PDF,
    // keyed by the original file's hash as the site's uploader keys it.
    let hash = data_encoding::HEXLOWER.encode(&Sha256::digest(&data));
    let stem = Path::new(name)
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or(ExamError::UploadFile)?;
    Ok((
        format!("{stem}.pdf"),
        "application/pdf",
        image_pdf(std::slice::from_ref(file), cancel)?,
        Some(hash),
    ))
}

/// The image formats the service stores, and so the ones a photo may be in.
fn photo_format(data: &[u8]) -> Option<ImageFormat> {
    image::guess_format(data).ok().filter(|format| {
        matches!(
            format,
            ImageFormat::Jpeg | ImageFormat::Png | ImageFormat::WebP
        )
    })
}

fn decode(data: &[u8]) -> Result<DynamicImage, ExamError> {
    let format = photo_format(data).ok_or(ExamError::UploadFile)?;
    let mut decoder = ImageReader::with_format(Cursor::new(data), format)
        .into_decoder()
        .map_err(|_| ExamError::UploadFile)?;
    let orientation = decoder.orientation().map_err(|_| ExamError::UploadFile)?;
    let mut image = DynamicImage::from_decoder(decoder).map_err(|_| ExamError::UploadFile)?;
    image.apply_orientation(orientation);
    Ok(image)
}

fn jpeg(image: &DynamicImage) -> Result<Vec<u8>, ExamError> {
    let rgba = image.to_rgba8();
    let mut rgb = image::RgbImage::new(rgba.width(), rgba.height());
    for (source, target) in rgba.pixels().zip(rgb.pixels_mut()) {
        let alpha = u16::from(source[3]);
        *target = image::Rgb(std::array::from_fn(|i| {
            ((u16::from(source[i]) * alpha + 255 * (255 - alpha)) / 255) as u8
        }));
    }
    let mut data = Vec::new();
    JpegEncoder::new_with_quality(&mut data, 90)
        .encode_image(&rgb)
        .map_err(|_| ExamError::UploadFile)?;
    Ok(data)
}

fn image_pdf(files: &[ExamUploadFile], cancel: &CancellationToken) -> Result<Vec<u8>, ExamError> {
    let mut pdf = Pdf::new();
    let catalog = Ref::new(1);
    let tree = Ref::new(2);
    let mut next = Ref::new(3);
    let mut pages = Vec::new();
    let mut image_bytes = 0;
    pdf.catalog(catalog).pages(tree);
    for file in files {
        if cancel.is_cancelled() {
            return Err(ExamError::Cancelled);
        }
        let data = read(file)?;
        // Several files become one PDF of their pages, which only photos give.
        if photo_format(&data).is_none() {
            return Err(ExamError::UploadMixed);
        }
        let image = decode(&data)?;
        let width = image.width().min(1600);
        let source_height = (2048_u64 * u64::from(image.width()) / u64::from(width))
            .min(u64::from(image.height())) as u32;
        for top in (0..image.height()).step_by(source_height as usize) {
            if cancel.is_cancelled() {
                return Err(ExamError::Cancelled);
            }
            let image = image
                .crop_imm(
                    0,
                    top,
                    image.width(),
                    source_height.min(image.height() - top),
                )
                .thumbnail(width, 2048);
            let encoded = jpeg(&image)?;
            image_bytes += encoded.len();
            check_size(image_bytes)?;
            let page_id = next.bump();
            let image_id = next.bump();
            let stream_id = next.bump();
            pages.push(page_id);
            let width = image.width() as f32;
            let height = image.height() as f32;
            let name = Name(b"Image");
            let mut page = pdf.page(page_id);
            page.parent(tree)
                .media_box(Rect::new(0.0, 0.0, width, height))
                .contents(stream_id);
            page.resources().x_objects().pair(name, image_id);
            page.finish();
            let mut object = pdf.image_xobject(image_id, &encoded);
            object.filter(Filter::DctDecode);
            object
                .width(image.width() as i32)
                .height(image.height() as i32)
                .bits_per_component(8);
            object.color_space().device_rgb();
            object.finish();
            let mut content = Content::new();
            content
                .transform([width, 0.0, 0.0, height, 0.0, 0.0])
                .x_object(name);
            pdf.stream(stream_id, &content.finish());
        }
    }
    pdf.pages(tree).count(pages.len() as i32).kids(pages);
    Ok(pdf.finish())
}

#[cfg(test)]
#[path = "../tests/exam_share__upload_file.rs"]
mod tests;
