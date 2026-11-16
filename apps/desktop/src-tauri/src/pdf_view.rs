//! The desktop PDF reader's host (slotledger aqchay): `pdf-view` renders
//! tiles with PDFium; the view asks for a plan per viewport and loads each
//! tile from `pdftile://` as an image.
use crate::{command_error::shell, require_local_main, CommandError};
use pdf_view::layout::{Layout, Plan, Viewport};
use pdf_view::{DocId, Engine, PdfError, SearchHit, TextRun};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::http::{header, Response, StatusCode};
use tauri::{Manager, Webview};
use tokio::sync::oneshot;

/// PDFium is bundled beside the app: in Contents/Frameworks, which the
/// executable's rpath names (build.rs), and beside `cargo run`'s binary.
#[cfg(target_os = "macos")]
const LIBRARY: &str = "@rpath/libpdfium.dylib";
/// Beside the executable, where Windows looks first.
#[cfg(target_os = "windows")]
const LIBRARY: &str = "pdfium.dll";

pub(crate) struct PdfViews {
    engine: Result<Engine, PdfError>,
    layouts: Mutex<HashMap<DocId, Layout>>,
}

impl PdfViews {
    pub(crate) fn start(cache: PathBuf) -> Self {
        let engine = Engine::start(Path::new(LIBRARY), cache);
        if let Err(error) = &engine {
            log::error!("[pdf-view] {error}");
        }
        Self {
            engine,
            layouts: Mutex::new(HashMap::new()),
        }
    }

    fn engine(&self) -> Result<&Engine, CommandError> {
        self.engine.as_ref().map_err(shell)
    }
}

/// Answers a render-thread reply on the async side.
async fn ask<T: Send + 'static>(start: impl FnOnce(pdf_view::Reply<T>)) -> Result<T, PdfError> {
    let (sender, receiver) = oneshot::channel();
    start(Box::new(move |result| {
        let _ = sender.send(result);
    }));
    receiver.await.map_err(|_| PdfError::Stopped)?
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OpenedPdf {
    id: DocId,
    page_count: usize,
}

#[tauri::command]
pub(crate) async fn pdf_open(
    webview: Webview,
    views: tauri::State<'_, PdfViews>,
    path: PathBuf,
) -> Result<OpenedPdf, CommandError> {
    require_local_main(&webview)?;
    // Only files the shell already let this page read: course files and
    // past exams it opened.
    if !webview
        .app_handle()
        .asset_protocol_scope()
        .is_allowed(&path)
    {
        return Err("This file is not open in the app.".into());
    }
    let engine = views.engine()?.clone();
    let opened = ask(|reply| engine.open(path, reply)).await.map_err(shell)?;
    let page_count = opened.pages.len();
    views
        .layouts
        .lock()
        .map_err(shell)?
        .insert(opened.id, Layout::new(opened.pages));
    Ok(OpenedPdf {
        id: opened.id,
        page_count,
    })
}

/// What to draw for a viewport; the engine starts on its tiles at once.
#[tauri::command]
pub(crate) fn pdf_plan(
    webview: Webview,
    views: tauri::State<'_, PdfViews>,
    id: DocId,
    viewport: Viewport,
) -> Result<Plan, CommandError> {
    require_local_main(&webview)?;
    let plan = views
        .layouts
        .lock()
        .map_err(shell)?
        .get(&id)
        .ok_or("This PDF is no longer open.")?
        .plan(&viewport);
    views.engine()?.want(id, plan.wanted.clone());
    Ok(plan)
}

/// The scroll position that shows `page` (0-based) at the top.
#[tauri::command]
pub(crate) fn pdf_page_top(
    webview: Webview,
    views: tauri::State<'_, PdfViews>,
    id: DocId,
    viewport: Viewport,
    page: u32,
) -> Result<f64, CommandError> {
    require_local_main(&webview)?;
    Ok(views
        .layouts
        .lock()
        .map_err(shell)?
        .get(&id)
        .ok_or("This PDF is no longer open.")?
        .page_top(&viewport, page))
}

#[tauri::command]
pub(crate) async fn pdf_text(
    webview: Webview,
    views: tauri::State<'_, PdfViews>,
    id: DocId,
    page: u32,
) -> Result<Vec<TextRun>, CommandError> {
    require_local_main(&webview)?;
    let engine = views.engine()?.clone();
    ask(|reply| engine.text(id, page, reply))
        .await
        .map_err(shell)
}

#[tauri::command]
pub(crate) async fn pdf_search(
    webview: Webview,
    views: tauri::State<'_, PdfViews>,
    id: DocId,
    query: String,
) -> Result<Vec<SearchHit>, CommandError> {
    require_local_main(&webview)?;
    let engine = views.engine()?.clone();
    ask(|reply| engine.search(id, query, reply))
        .await
        .map_err(shell)
}

#[tauri::command]
pub(crate) fn pdf_close(
    webview: Webview,
    views: tauri::State<'_, PdfViews>,
    id: DocId,
) -> Result<(), CommandError> {
    require_local_main(&webview)?;
    views.layouts.lock().map_err(shell)?.remove(&id);
    views.engine()?.close(id);
    Ok(())
}

/// `pdftile://localhost/<doc>/<page>/<level>/<col>/<row>` → a PNG tile. A
/// tile the view stopped needing answers 410, which the view ignores.
pub(crate) fn tile(
    app: &tauri::AppHandle,
    request: tauri::http::Request<Vec<u8>>,
    responder: tauri::UriSchemeResponder,
) {
    let respond = move |status: StatusCode, body: Vec<u8>| {
        responder.respond(
            Response::builder()
                .status(status)
                .header(header::CONTENT_TYPE, "image/png")
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .header(header::CACHE_CONTROL, "max-age=3600, immutable")
                .body(body)
                .unwrap_or_else(|_| Response::new(Vec::new())),
        )
    };
    let parts: Vec<&str> = request.uri().path().trim_matches('/').split('/').collect();
    let parsed = match parts.as_slice() {
        [doc, page, level, col, row] => (|| {
            Some((
                doc.parse::<DocId>().ok()?,
                pdf_view::layout::TileKey {
                    page: page.parse().ok()?,
                    level: level.parse().ok()?,
                    col: col.parse().ok()?,
                    row: row.parse().ok()?,
                },
            ))
        })(),
        _ => None,
    };
    let (Some((doc, key)), Ok(engine)) = (parsed, app.state::<PdfViews>().engine.clone()) else {
        return respond(StatusCode::BAD_REQUEST, Vec::new());
    };
    engine.tile(
        doc,
        key,
        Box::new(move |result| match result {
            Ok(bytes) => respond(StatusCode::OK, bytes.as_ref().clone()),
            Err(PdfError::Cancelled) => respond(StatusCode::GONE, Vec::new()),
            Err(PdfError::Closed) => respond(StatusCode::NOT_FOUND, Vec::new()),
            Err(error) => {
                log::warn!("[pdf-view] tile {key:?}: {error}");
                respond(StatusCode::INTERNAL_SERVER_ERROR, Vec::new())
            }
        }),
    );
}
