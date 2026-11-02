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
    if let Some((family, size, text)) = parse_google(&request) {
        let renderer = state.renderer.clone();
        let seq = state.thumb_seq.fetch_add(1, Ordering::Relaxed) + 1;
        let latest = state.thumb_seq.clone();
        state.pool.submit(seq as i64, &CancelToken::default(), move || {
            if latest.load(Ordering::Relaxed) - seq > STALE_AFTER {
                responder.respond(status(StatusCode::SERVICE_UNAVAILABLE));
                return;
            }
            let result = renderer.thumbnail_of(&format!("google:{family}"), &text, size, || {
                ts_providers::preview_subset(&family, &text)
            });
            responder.respond(match result {
                Ok(bytes) => image(bytes),
                Err(_) => status(StatusCode::UNPROCESSABLE_ENTITY),
            });
        });
        return;
    }
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
            Ok(bytes) => image(bytes),
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

/// `thumb://localhost/g/<family>/<size>?t=<text>`: a Google family not yet downloaded.
fn parse_google(request: &Request<Vec<u8>>) -> Option<(String, u32, String)> {
    let url = tauri::Url::parse(&request.uri().to_string()).ok()?;
    let mut parts = url.path_segments()?;
    if parts.next()? != "g" {
        return None;
    }
    let family = percent_decode(parts.next()?);
    let size = parts.next()?.parse::<u32>().ok()?.clamp(8, 400);
    let text = url
        .query_pairs()
        .find(|(k, _)| k == "t")
        .map(|(_, v)| v.chars().take(200).collect())
        .unwrap_or_else(|| ts_index::DEFAULT_SAMPLE.to_owned());
    Some((family, size, text))
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && let Some(v) = s.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn image(bytes: Vec<u8>) -> Response<Vec<u8>> {
    Response::builder()
        .header("Content-Type", "image/webp")
        .header("Cache-Control", "max-age=31536000, immutable")
        .header("Access-Control-Allow-Origin", "*")
        .body(bytes)
        .unwrap()
}

fn status(code: StatusCode) -> Response<Vec<u8>> {
    Response::builder().status(code).header("Cache-Control", "no-store").body(Vec::new()).unwrap()
}
