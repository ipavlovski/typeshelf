//! `thumb://` protocol: `thumb://localhost/<face>/<size>?t=<text>&live=1&w=<max width>`.
//!
//! Jobs run newest-first, so the rows on screen render before ones scrolled past.
//! A job overtaken by many newer requests is dropped with an uncacheable 503; if
//! its row comes back into view the WebView simply asks again.

use std::path::PathBuf;
use std::sync::atomic::Ordering;

use tauri::http::{Request, Response, StatusCode};
use tauri::{Manager, UriSchemeContext, UriSchemeResponder};
use ts_render::{CancelToken, LineStyle};

use crate::state::AppState;

/// Requests older than this many newer ones are no longer on screen.
const STALE_AFTER: u64 = 400;

pub fn handle<R: tauri::Runtime>(
    ctx: UriSchemeContext<'_, R>,
    request: Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let app = ctx.app_handle().clone();
    let state = app.state::<AppState>();
    let Some(job) = parse(&request) else {
        responder.respond(status(StatusCode::BAD_REQUEST));
        return;
    };
    let Some((key, path)) =
        state.snap.read().unwrap().faces.get(job.face as usize).map(|r| (r.key, PathBuf::from(&r.path)))
    else {
        responder.respond(status(StatusCode::NOT_FOUND));
        return;
    };
    let seq = state.thumb_seq.fetch_add(1, Ordering::Relaxed) + 1;
    let latest = state.thumb_seq.clone();
    let renderer = state.renderer.clone();
    state.pool.submit(seq as i64, &CancelToken::default(), move || {
        if latest.load(Ordering::Relaxed) - seq > STALE_AFTER {
            responder.respond(status(StatusCode::SERVICE_UNAVAILABLE));
            return;
        }
        let req = ts_render::Request { key, path: &path, text: &job.text, size: job.size };
        let result =
            if job.live { renderer.live(&req, job.max_width, &LineStyle::default()) } else { renderer.thumbnail(&req) };
        responder.respond(match result {
            Ok(bytes) => Response::builder()
                .header("Content-Type", "image/webp")
                .header("Cache-Control", "max-age=31536000, immutable")
                .header("Access-Control-Allow-Origin", "*")
                .body(bytes)
                .unwrap(),
            Err(_) => status(StatusCode::UNPROCESSABLE_ENTITY),
        });
    });
}

struct Job {
    face: u32,
    size: u32,
    text: String,
    live: bool,
    max_width: u32,
}

fn parse(request: &Request<Vec<u8>>) -> Option<Job> {
    let url = tauri::Url::parse(&request.uri().to_string()).ok()?;
    let mut parts = url.path_segments()?;
    let face = parts.next()?.parse().ok()?;
    let size = parts.next()?.parse::<u32>().ok()?.clamp(8, 400);
    let mut job = Job { face, size, text: ts_index::DEFAULT_SAMPLE.to_owned(), live: false, max_width: 2400 };
    for (k, v) in url.query_pairs() {
        match k.as_ref() {
            "t" if !v.is_empty() => job.text = v.chars().take(200).collect(),
            "live" => job.live = v == "1",
            "w" => job.max_width = v.parse().unwrap_or(2400).clamp(16, 4000),
            _ => {}
        }
    }
    Some(job)
}

fn status(code: StatusCode) -> Response<Vec<u8>> {
    Response::builder().status(code).header("Cache-Control", "no-store").body(Vec::new()).unwrap()
}
