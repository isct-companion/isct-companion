//! What a school view does with a page the server sends as an attachment.
//!
//! Moodle and other school services mark many files `Content-Disposition:
//! attachment` (every assignment submission, `?forcedownload=1` links). When
//! the engine can display the type, WebKit commits such a response as an
//! empty page unless the shell turns it into a download, so shells ask here
//! before letting a main-frame response through.
use serde::Serialize;

/// How the app shows an attachment instead of the school view.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AttachmentPreview {
    Pdf,
    Image,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttachmentAction {
    /// Fetch it through the view's session, then show it in the app's viewer.
    Preview(AttachmentPreview),
    /// Save it to the download folder.
    Download,
}

/// The action for a main-frame response, or `None` when it is not an
/// attachment and the view should display it as usual.
pub fn attachment_action(
    disposition: Option<&str>,
    mime_type: Option<&str>,
    file_name: &str,
) -> Option<AttachmentAction> {
    let kind = disposition?.split(';').next()?.trim();
    if !kind.eq_ignore_ascii_case("attachment") {
        return None;
    }
    let mime = mime_type
        .and_then(|mime| mime.split(';').next())
        .map(|mime| mime.trim().to_ascii_lowercase())
        .unwrap_or_default();
    let preview = match mime.as_str() {
        "application/pdf" => Some(AttachmentPreview::Pdf),
        "image/png" | "image/jpeg" | "image/gif" | "image/webp" | "image/bmp" | "image/avif"
        | "image/svg+xml" => Some(AttachmentPreview::Image),
        // Forced downloads may hide the real type; the file name still has it.
        "" | "application/octet-stream" | "application/x-forcedownload" | "binary/octet-stream" => {
            by_extension(file_name)
        }
        _ => None,
    };
    Some(preview.map_or(AttachmentAction::Download, AttachmentAction::Preview))
}

fn by_extension(file_name: &str) -> Option<AttachmentPreview> {
    let (_, extension) = file_name.rsplit_once('.')?;
    match extension.to_ascii_lowercase().as_str() {
        "pdf" => Some(AttachmentPreview::Pdf),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "avif" | "svg" => {
            Some(AttachmentPreview::Image)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use AttachmentAction::{Download, Preview};
    use AttachmentPreview::{Image, Pdf};

    #[test]
    fn only_attachments_leave_the_view() {
        for disposition in [None, Some("inline; filename=\"a.pdf\""), Some("")] {
            assert_eq!(
                attachment_action(disposition, Some("application/pdf"), "a.pdf"),
                None
            );
        }
    }

    #[test]
    fn pdfs_and_images_preview_and_everything_else_downloads() {
        let attachment = Some("Attachment; filename=\"report.pdf\"");
        for (mime, name, action) in [
            (Some("application/pdf"), "report.pdf", Preview(Pdf)),
            (Some("image/JPEG; charset=binary"), "photo", Preview(Image)),
            (
                Some("application/x-forcedownload"),
                "Scan.PDF",
                Preview(Pdf),
            ),
            (Some("application/octet-stream"), "plot.png", Preview(Image)),
            (None, "slides.pdf", Preview(Pdf)),
            (Some("text/plain"), "main.py", Download),
            (Some("text/html"), "page.html", Download),
            (Some("application/zip"), "code.zip", Download),
            (Some("application/octet-stream"), "notes.docx", Download),
            (Some("application/octet-stream"), "README", Download),
        ] {
            assert_eq!(
                attachment_action(attachment, mime, name),
                Some(action),
                "{mime:?} {name}"
            );
        }
    }
}
