//! Local WebSocket endpoint for design-app plugins (Adobe UXP): a plugin sends the
//! PostScript names a document needs, typeshelf activates them as session fonts.
//!
//! Bound to 127.0.0.1 only, on a fixed port (src/channel.rs) because UXP manifests must declare it.
//! It can only activate fonts already in the library.
//!
//! ```json
//! → {"type":"activate","postscriptNames":["Garamond-Regular"]}
//! ← {"type":"result","activated":1,"alreadyActive":0,"missing":[],"restartApps":[]}
//! → {"type":"ping"}
//! ← {"type":"pong","app":"typeshelf","version":"0.1.0"}
//! ```

use std::collections::HashSet;
use std::net::{TcpListener, TcpStream};

use serde::Deserialize;
use serde_json::{Value, json};
use tauri::{AppHandle, Emitter, Manager};
use tungstenite::Message;

use crate::state::AppState;

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Request {
    Ping,
    /// typeshelfDev only: `pnpm preview` quits the running preview cleanly before replacing it.
    Quit,
    #[serde(rename_all = "camelCase")]
    Activate {
        postscript_names: Vec<String>,
        #[serde(default)]
        pin: bool,
    },
}

pub fn start(app: AppHandle) {
    let listener = match TcpListener::bind(("127.0.0.1", crate::channel::PORT)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("automation endpoint unavailable on port {}: {e}", crate::channel::PORT);
            return;
        }
    };
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let app = app.clone();
            std::thread::spawn(move || serve(&app, stream));
        }
    });
}

fn serve(app: &AppHandle, stream: TcpStream) {
    let Ok(mut ws) = tungstenite::accept(stream) else { return };
    while let Ok(msg) = ws.read() {
        let reply = match msg {
            Message::Text(text)
                if crate::channel::IS_DEV && matches!(serde_json::from_str(&text), Ok(Request::Quit)) =>
            {
                let _ = ws.send(Message::text(json!({ "type": "bye" }).to_string()));
                app.exit(0);
                break;
            }
            Message::Text(text) => handle(&app.state::<AppState>(), &text),
            Message::Close(_) => break,
            _ => continue,
        };
        if reply.get("activated").and_then(Value::as_u64).unwrap_or(0) > 0 {
            let _ = app.emit("library-changed", ());
        }
        if ws.send(Message::text(reply.to_string())).is_err() {
            break;
        }
    }
}

pub fn handle(state: &AppState, text: &str) -> Value {
    let request = match serde_json::from_str::<Request>(text) {
        Ok(r) => r,
        Err(e) => return json!({ "type": "error", "message": e.to_string() }),
    };
    match request {
        Request::Ping => json!({ "type": "pong", "app": crate::channel::APP, "version": env!("CARGO_PKG_VERSION") }),
        Request::Quit => json!({ "type": "error", "message": "quit is only accepted by typeshelfDev" }),
        Request::Activate { postscript_names, pin } => {
            let wanted: HashSet<&str> = postscript_names.iter().map(String::as_str).collect();
            // Documents use system fonts too; a library copy of Arial must not shadow Windows' own.
            let system = state.backend.as_ref().and_then(|b| b.system_font_names().ok()).unwrap_or_default();
            let (faces, found, already) = {
                let snap = state.snap.read().unwrap();
                let mut faces = Vec::new();
                let mut found = HashSet::new();
                let mut already = 0;
                for (id, row) in snap.faces.iter().enumerate() {
                    if wanted.contains(row.info.ps_name.as_str()) {
                        found.insert(row.info.ps_name.as_str().to_owned());
                        if system.contains(&row.info.full_name.to_lowercase()) {
                            continue;
                        }
                        if snap.is_active(id as u32) {
                            already += 1;
                        } else {
                            faces.push(id as u32);
                        }
                    }
                }
                (faces, found, already)
            };
            let missing: Vec<&String> = postscript_names.iter().filter(|n| !found.contains(*n)).collect();
            if faces.is_empty() {
                return json!({ "type": "result", "activated": 0, "alreadyActive": already, "missing": missing, "restartApps": [] });
            }
            match state.set_active(&faces, true, pin) {
                Ok(out) => json!({
                    "type": "result",
                    "activated": out.changed.len(),
                    "alreadyActive": already,
                    "missing": missing,
                    "restartApps": [],
                }),
                Err(e) => json!({ "type": "error", "message": format!("{e:#}"), "missing": missing }),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> (tempfile::TempDir, AppState) {
        let dir = tempfile::tempdir().unwrap();
        let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
        // Never touch the real OS font registration from tests.
        let backend: Box<dyn ts_activate::Backend> = Box::new(ts_activate::MemoryBackend::default());
        let state = AppState::open_with(dir.path().to_owned(), Some(backend)).unwrap();
        let folder = state.add_folder(&fixtures.canonicalize().unwrap(), false).unwrap();
        state.index_folder(&folder, &|_, _| {}).unwrap();
        (dir, state)
    }

    #[test]
    fn ping() {
        let (_dir, s) = state();
        assert_eq!(handle(&s, r#"{"type":"ping"}"#)["type"], "pong");
    }

    #[test]
    fn reports_missing_fonts_and_bad_requests() {
        let (_dir, s) = state();
        let r = handle(&s, r#"{"type":"activate","postscriptNames":["Nope-Regular"]}"#);
        assert_eq!(r["missing"], json!(["Nope-Regular"]));
        assert_eq!(r["activated"], 0);
        assert_eq!(handle(&s, "{}")["type"], "error");
    }

    #[test]
    fn activates_known_fonts() {
        let (_dir, s) = state();
        let r = handle(&s, r#"{"type":"activate","postscriptNames":["FixtureSans-Light","Nope"]}"#);
        assert_eq!(r["activated"], 1);
        assert_eq!(r["missing"], json!(["Nope"]));
        let again = handle(&s, r#"{"type":"activate","postscriptNames":["FixtureSans-Light"]}"#);
        assert_eq!((again["activated"].as_u64(), again["alreadyActive"].as_u64()), (Some(0), Some(1)));
    }
}
