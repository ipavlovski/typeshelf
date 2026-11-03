//! MCP (Model Context Protocol) server, so an assistant such as Claude can browse the library
//! and tag it. Messages are JSON-RPC 2.0 sent as text frames on the local automation endpoint
//! (src/automation.rs). Assistants that speak MCP over stdio start `typeshelf.exe --mcp`
//! (see [`bridge`]): it relays each line to the running app, starting the app if needed.
//!
//! The server is stateless: every request is answered from the current snapshot, so a bridge
//! that reconnects after the app restarts carries on as if nothing happened.

use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use base64::Engine;
use serde_json::{Value, json};
use ts_model::{Block, FaceKey, normalize_tag};
use ts_render::{Image, LineStyle};
use ts_search::{FaceId, FamilyId, Filter, Query, Snapshot};

use crate::state::AppState;

const PROTOCOLS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "typeshelf is the user's font manager. Fonts are grouped into families (identified by \
name); tags are hierarchical paths separated by '/', e.g. 'serif/transitional' or 'display/script/brush'. A family \
tagged 'serif/slab' also matches searches for '#serif', and parent tags exist implicitly. To tag a library: call \
library_overview for the existing taxonomy, page through list_families with query 'is:untagged' (offset 0 each time, \
since tagged families drop out), look at families with preview_families when names and metrics are not enough, then \
record decisions with tag_families in batches. Reuse existing tag paths rather than inventing near-duplicates.";

const QUERY_HELP: &str = "Search syntax: free text (fuzzy family name) plus filters: '#serif' or 'tag:serif' (tag \
and its sub-tags), 'is:tagged', 'is:untagged', 'is:variable', 'is:italic', 'is:active', 'is:fav', \
'script:cyrillic', 'feat:ss01', 'format:otf', 'col:\"Name\"' (collection), 'folder:text', numeric ranges such as \
'wght:>600', 'xheight:0.5..0.6', 'contrast:<0.2', 'width:>1.0', 'glyphs:>500'. Prefix any filter with '-' to \
negate it. Quote values containing spaces: '#\"hand made\"'.";

/// Blocks so common they say nothing about a font.
const COMMON_BLOCKS: &[Block] = &[
    Block::BasicLatin,
    Block::Latin1,
    Block::LatinExtA,
    Block::LatinExtB,
    Block::Punctuation,
    Block::Currency,
    Block::Letterlike,
    Block::NumberForms,
    Block::Arrows,
    Block::Math,
    Block::Technical,
    Block::Shapes,
    Block::PresentationForms,
    Block::Ipa,
];

const MAX_LIST: usize = 200;
const MAX_PREVIEW: usize = 16;
const SHEET_WIDTH: u32 = 1100;
const PANGRAM: &str = "The quick brown fox jumps over the lazy dog 0123456789";

/// Handles one JSON-RPC message. Returns the reply (none for notifications) and whether
/// the library changed, so the caller can tell the UI to refresh.
pub fn handle(state: &AppState, msg: &Value) -> (Option<Value>, bool) {
    let id = msg.get("id").cloned();
    let Some(method) = msg.get("method").and_then(Value::as_str) else {
        // A response to something we never send, or garbage.
        return (id.map(|id| error(id, -32600, "invalid request")), false);
    };
    let Some(id) = id else { return (None, false) }; // notification
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let mut changed = false;
    let result = match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
            let version = PROTOCOLS.iter().find(|&&v| v == asked).unwrap_or(&PROTOCOLS[0]);
            Ok(json!({
                "protocolVersion": version,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": crate::channel::APP, "version": env!("CARGO_PKG_VERSION") },
                "instructions": INSTRUCTIONS,
            }))
        }
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools() })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            Ok(match call(state, name, &args) {
                Ok(out) => {
                    changed = out.changed;
                    json!({ "content": out.content })
                }
                Err(e) => json!({ "content": [text(format!("{e:#}"))], "isError": true }),
            })
        }
        _ => Err((-32601, format!("method not found: {method}"))),
    };
    let reply = match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
        Err((code, message)) => error(id, code, &message),
    };
    (Some(reply), changed)
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn text(s: impl Into<String>) -> Value {
    json!({ "type": "text", "text": s.into() })
}

fn tools() -> Value {
    let families =
        json!({ "type": "array", "items": { "type": "string" }, "description": "Family names, exactly as listed." });
    json!([
        {
            "name": "library_overview",
            "description": "Library size, how many families are still untagged, the full tag tree with family counts, and the user's collections. Start here.",
            "inputSchema": { "type": "object", "properties": {} },
            "annotations": { "readOnlyHint": true },
        },
        {
            "name": "list_families",
            "description": format!("Lists font families matching a query, alphabetically, with what the fonts say about themselves: styles, weight range, italics, variable axes, vendor, non-Latin scripts, measured shape metrics of the regular style (xheight = x-height/cap height, stem = stem thickness/cap height, width = average advance/cap height, contrast = 0 monolinear .. 1 hairline), folder, collections and current tags. {QUERY_HELP}"),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search query; empty lists everything. Use 'is:untagged' to work through what is left." },
                    "offset": { "type": "integer", "minimum": 0, "default": 0 },
                    "limit": { "type": "integer", "minimum": 1, "maximum": MAX_LIST, "default": 50 },
                },
            },
            "annotations": { "readOnlyHint": true },
        },
        {
            "name": "family_details",
            "description": "Everything known about one family: every face with its style, weight, width, PostScript name, axes and file; OpenType features; scripts; vendor; licence text; tags.",
            "inputSchema": { "type": "object", "properties": { "family": { "type": "string" } }, "required": ["family"] },
            "annotations": { "readOnlyHint": true },
        },
        {
            "name": "preview_families",
            "description": format!("Renders a specimen sheet image: for each family in the order given, its name set in itself and a line of sample text, separated by rules. Fonts that can't draw the text show a sample they can. Up to {MAX_PREVIEW} families per call."),
            "inputSchema": {
                "type": "object",
                "properties": {
                    "families": families,
                    "text": { "type": "string", "description": "Sample line instead of the default pangram." },
                },
                "required": ["families"],
            },
            "annotations": { "readOnlyHint": true },
        },
        {
            "name": "tag_families",
            "description": "Changes tags on whole families (every face). Tags are '/'-separated paths; an existing tag keeps its spelling. mode 'add' (default) adds the tags, 'remove' removes exactly those tags, 'replace' makes them the family's only tags. Batch many families per call.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "assignments": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "family": { "type": "string" },
                                "tags": { "type": "array", "items": { "type": "string" } },
                            },
                            "required": ["family", "tags"],
                        },
                    },
                    "mode": { "type": "string", "enum": ["add", "remove", "replace"], "default": "add" },
                },
                "required": ["assignments"],
            },
            "annotations": { "destructiveHint": false, "idempotentHint": true },
        },
        {
            "name": "rename_tag",
            "description": "Renames a tag and everything below it (serif → classic turns serif/slab into classic/slab). Renaming onto an existing tag merges them. Also moves a branch: 'slab' → 'serif/slab'.",
            "inputSchema": {
                "type": "object",
                "properties": { "from": { "type": "string" }, "to": { "type": "string" } },
                "required": ["from", "to"],
            },
        },
        {
            "name": "delete_tag",
            "description": "Removes a tag and all its sub-tags from every font. The fonts stay.",
            "inputSchema": { "type": "object", "properties": { "tag": { "type": "string" } }, "required": ["tag"] },
            "annotations": { "destructiveHint": true },
        },
    ])
}

struct Output {
    content: Vec<Value>,
    changed: bool,
}

impl Output {
    fn json(v: Value) -> Output {
        Output { content: vec![text(v.to_string())], changed: false }
    }

    fn changed(mut self) -> Output {
        self.changed = true;
        self
    }
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str> {
    args.get(key).and_then(Value::as_str).with_context(|| format!("'{key}' is required"))
}

fn call(state: &AppState, name: &str, args: &Value) -> Result<Output> {
    match name {
        "library_overview" => Ok(Output::json(overview(&state.snap.read().unwrap()))),
        "list_families" => {
            let query = args.get("query").and_then(Value::as_str).unwrap_or("");
            let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
            let limit = (args.get("limit").and_then(Value::as_u64).unwrap_or(50) as usize).clamp(1, MAX_LIST);
            Ok(Output::json(list(&state.snap.read().unwrap(), query, offset, limit)))
        }
        "family_details" => {
            let snap = state.snap.read().unwrap();
            let fam = find(&snap, str_arg(args, "family")?)?;
            Ok(Output::json(details(&snap, fam)))
        }
        "preview_families" => preview(state, args),
        "tag_families" => tag_families(state, args).map(|v| Output::json(v).changed()),
        "rename_tag" => {
            let (from, to) = (str_arg(args, "from")?, str_arg(args, "to")?);
            let mut store = state.store.lock().unwrap();
            store.rename_tag(from, to)?;
            state.refresh_tags(&store)?;
            Ok(Output::json(json!({ "renamed": from, "to": normalize_tag(to) })).changed())
        }
        "delete_tag" => {
            let tag = str_arg(args, "tag")?;
            let store = state.store.lock().unwrap();
            let removed = store.delete_tag(tag)?;
            state.refresh_tags(&store)?;
            Ok(Output::json(json!({ "deleted": tag, "taggingsRemoved": removed })).changed())
        }
        _ => bail!("unknown tool: {name}"),
    }
}

fn families_of(snap: &Snapshot, faces: impl IntoIterator<Item = FaceId>) -> usize {
    faces.into_iter().map(|f| snap.family_of(f)).collect::<std::collections::HashSet<_>>().len()
}

fn search(snap: &Snapshot, query: &str) -> Vec<(FamilyId, Vec<FaceId>)> {
    snap.search(&Query::parse(query), &|| false).unwrap_or_default().families
}

fn overview(snap: &Snapshot) -> Value {
    let untagged = snap.search(&Query::default().with(Filter::Not(Box::new(Filter::Tagged))), &|| false);
    let tags: Vec<Value> = snap
        .tag_tree()
        .into_iter()
        .map(|t| json!({ "tag": t.path, "families": families_of(snap, &t.faces) }))
        .collect();
    let collections: Vec<Value> = snap
        .collections()
        .iter()
        .map(
            |c| json!({ "name": collection_path(snap, c), "families": families_of(snap, snap.collection_faces(c.id)) }),
        )
        .collect();
    json!({
        "families": snap.families.len(),
        "faces": snap.faces.len(),
        "untaggedFamilies": untagged.map_or(0, |r| r.families.len()),
        "tags": tags,
        "collections": collections,
        "queryHelp": QUERY_HELP,
    })
}

/// The face standing for a family: the one closest to Regular.
fn regular(snap: &Snapshot, faces: &[FaceId]) -> FaceId {
    faces
        .iter()
        .copied()
        .min_by_key(|&id| {
            let f = &snap.faces[id as usize].info;
            (f.italic, (f.weight as i32 - 400).abs(), f.width.abs_diff(5))
        })
        .unwrap_or(0)
}

fn round(v: Option<f32>) -> Value {
    v.map_or(Value::Null, |v| json!((v as f64 * 100.0).round() / 100.0))
}

fn scripts(snap: &Snapshot, faces: &[FaceId]) -> Vec<&'static str> {
    let cov = faces.iter().fold(0u64, |acc, &f| acc | snap.faces[f as usize].info.coverage.0);
    ts_model::Coverage(cov).blocks().filter(|b| !COMMON_BLOCKS.contains(b)).map(Block::slug).collect()
}

/// A collection's name with its parents', e.g. `1. SERIF/1A. Humanist`.
fn collection_path(snap: &Snapshot, c: &ts_store::Collection) -> String {
    let mut parts = vec![c.name.as_str()];
    let mut parent = c.parent_id;
    // Bounded, in case of a cycle.
    while let Some(p) = parent.and_then(|id| snap.collections().iter().find(|x| x.id == id))
        && parts.len() < 16
    {
        parts.push(&p.name);
        parent = p.parent_id;
    }
    parts.reverse();
    parts.join("/")
}

/// The innermost collections holding any of `faces` (a parent is implied by its path).
fn collections_of(snap: &Snapshot, faces: &[FaceId]) -> Vec<String> {
    let holds = |id: i64| {
        let members = snap.collection_faces(id);
        faces.iter().any(|f| members.contains(*f))
    };
    let all = snap.collections();
    all.iter()
        .filter(|c| holds(c.id) && !all.iter().any(|k| k.parent_id == Some(c.id) && holds(k.id)))
        .map(|c| collection_path(snap, c))
        .collect()
}

fn folder_name(path: &str) -> Option<String> {
    Path::new(path).parent()?.file_name().map(|n| n.to_string_lossy().into_owned())
}

/// Omits empty and null fields to keep long listings short.
fn compact(mut v: Value) -> Value {
    if let Value::Object(m) = &mut v {
        m.retain(|_, x| !(x.is_null() || x == &json!([]) || x == &json!(false) || x == &json!("") || x == &json!({})));
    }
    v
}

fn summary(snap: &Snapshot, fam: FamilyId) -> Value {
    let faces = &snap.families[fam as usize].faces;
    let info = |f: &FaceId| &snap.faces[*f as usize].info;
    let reg = &snap.faces[regular(snap, faces) as usize];
    let (lo, hi) = faces.iter().map(|f| info(f).weight).fold((u16::MAX, 0), |(lo, hi), w| (lo.min(w), hi.max(w)));
    let axes: Vec<String> = reg.info.axes.iter().map(|a| format!("{} {}..{}", a.tag, a.min, a.max)).collect();
    let m = &reg.info.metrics;
    compact(json!({
        "name": snap.families[fam as usize].name,
        "styles": faces.len(),
        "weights": if lo == hi { lo.to_string() } else { format!("{lo}-{hi}") },
        "italic": faces.iter().any(|f| info(f).italic),
        "axes": axes,
        "vendor": reg.info.vendor,
        "scripts": scripts(snap, faces),
        "glyphs": reg.info.glyph_count,
        "metrics": compact(json!({ "xheight": round(m.x_height), "stem": round(m.weight), "width": round(m.width), "contrast": round(m.contrast) })),
        "folder": folder_name(&reg.path),
        "collections": collections_of(snap, faces),
        "tags": snap.family_tags(fam),
    }))
}

fn list(snap: &Snapshot, query: &str, offset: usize, limit: usize) -> Value {
    let results = search(snap, query);
    let families: Vec<Value> = results.iter().skip(offset).take(limit).map(|(fam, _)| summary(snap, *fam)).collect();
    let next = offset + families.len();
    json!({
        "total": results.len(),
        "offset": offset,
        "families": families,
        "next": (next < results.len()).then_some(next),
    })
}

fn details(snap: &Snapshot, fam: FamilyId) -> Value {
    let faces = &snap.families[fam as usize].faces;
    let mut features: Vec<&str> =
        faces.iter().flat_map(|&f| snap.faces[f as usize].info.features.iter().map(String::as_str)).collect();
    features.sort_unstable();
    features.dedup();
    let reg = &snap.faces[regular(snap, faces) as usize].info;
    let rows: Vec<Value> = faces
        .iter()
        .map(|&f| {
            let row = &snap.faces[f as usize];
            let i = &row.info;
            compact(json!({
                "style": i.subfamily,
                "fullName": i.full_name,
                "postscript": i.ps_name,
                "weight": i.weight,
                "width": i.width,
                "italic": i.italic,
                "format": i.format.as_str(),
                "axes": i.axes.iter().map(|a| json!({ "tag": a.tag, "min": a.min, "default": a.default, "max": a.max })).collect::<Vec<_>>(),
                "file": row.path,
            }))
        })
        .collect();
    compact(json!({
        "name": snap.families[fam as usize].name,
        "nameInFont": (reg.family_raw != reg.family).then_some(&reg.family_raw),
        "faces": rows,
        "features": features,
        "scripts": scripts(snap, faces),
        "vendor": reg.vendor,
        "license": reg.license,
        "sample": reg.sample,
        "collections": collections_of(snap, faces),
        "tags": snap.family_tags(fam),
    }))
}

fn family_index(snap: &Snapshot) -> HashMap<String, FamilyId> {
    snap.families.iter().enumerate().map(|(i, f)| (f.name.to_lowercase(), i as FamilyId)).collect()
}

fn find(snap: &Snapshot, name: &str) -> Result<FamilyId> {
    match family_index(snap).get(&name.trim().to_lowercase()) {
        Some(&f) => Ok(f),
        None => bail!("no family named '{name}' (names are as list_families shows them)"),
    }
}

fn names_arg(args: &Value) -> Result<Vec<&str>> {
    let list = args.get("families").and_then(Value::as_array).context("'families' must be a list of names")?;
    list.iter().map(|v| v.as_str().context("family names must be strings")).collect()
}

fn preview(state: &AppState, args: &Value) -> Result<Output> {
    let names = names_arg(args)?;
    if names.is_empty() || names.len() > MAX_PREVIEW {
        bail!("give 1 to {MAX_PREVIEW} families");
    }
    let sample = args.get("text").and_then(Value::as_str).filter(|t| !t.trim().is_empty()).unwrap_or(PANGRAM);
    // Copy out what's needed so rendering doesn't hold the snapshot lock.
    let wanted: Vec<(String, Option<(FaceKey, PathBuf)>)> = {
        let snap = state.snap.read().unwrap();
        let index = family_index(&snap);
        names
            .iter()
            .map(|n| {
                let face = index.get(&n.trim().to_lowercase()).map(|&fam| {
                    let row = &snap.faces[regular(&snap, &snap.families[fam as usize].faces) as usize];
                    (row.key, PathBuf::from(&row.path))
                });
                ((*n).to_owned(), face)
            })
            .collect()
    };
    let style = LineStyle::default();
    let mut lines: Vec<Option<Image>> = Vec::new();
    let mut legend = Vec::new();
    for (i, (name, face)) in wanted.iter().enumerate() {
        let rendered = face.as_ref().context("not in the library").and_then(|(key, path)| {
            let font = state.renderer.font(*key, path)?;
            let title = ts_render::render_line(&font, &font.drawable_text(name), 40.0, SHEET_WIDTH, &style)?;
            let body = ts_render::render_line(&font, &font.drawable_text(sample), 24.0, SHEET_WIDTH, &style)?;
            Ok((title, body))
        });
        match rendered {
            Ok((title, body)) => {
                if !lines.is_empty() {
                    lines.push(None);
                }
                lines.push(Some(title));
                lines.push(Some(body));
                legend.push(format!("{}. {name}", i + 1));
            }
            // Keep the rows in step with the legend.
            Err(e) => legend.push(format!("{}. {name} (not shown: {e:#})", i + 1)),
        }
    }
    let sheet = ts_render::stack(&lines, SHEET_WIDTH, 6);
    let webp = ts_render::encode_webp_ink(&sheet)?;
    Ok(Output {
        content: vec![
            text(format!(
                "Top to bottom, separated by rules (families not shown leave no row):\n{}",
                legend.join("\n")
            )),
            json!({ "type": "image", "mimeType": "image/webp", "data": base64::engine::general_purpose::STANDARD.encode(webp) }),
        ],
        changed: false,
    })
}

fn tag_families(state: &AppState, args: &Value) -> Result<Value> {
    let mode = args.get("mode").and_then(Value::as_str).unwrap_or("add");
    if !["add", "remove", "replace"].contains(&mode) {
        bail!("mode must be add, remove or replace");
    }
    let assignments = args.get("assignments").and_then(Value::as_array).context("'assignments' must be a list")?;
    // Lowercased tag → (spelling, faces), so one transaction per tag.
    let mut adds: BTreeMap<String, (String, Vec<FaceKey>)> = BTreeMap::new();
    let mut removes: BTreeMap<String, (String, Vec<FaceKey>)> = BTreeMap::new();
    let mut unknown = Vec::new();
    let mut families = 0;
    {
        let snap = state.snap.read().unwrap();
        let index = family_index(&snap);
        for a in assignments {
            let name = a.get("family").and_then(Value::as_str).context("each assignment needs a 'family'")?;
            let tags: Vec<String> = a
                .get("tags")
                .and_then(Value::as_array)
                .context("each assignment needs 'tags'")?
                .iter()
                .filter_map(|t| t.as_str().and_then(normalize_tag))
                .collect();
            let Some(&fam) = index.get(&name.trim().to_lowercase()) else {
                unknown.push(name.to_owned());
                continue;
            };
            families += 1;
            let keys: Vec<FaceKey> =
                snap.families[fam as usize].faces.iter().map(|&f| snap.faces[f as usize].key).collect();
            let push = |map: &mut BTreeMap<String, (String, Vec<FaceKey>)>, tag: &str| {
                map.entry(tag.to_lowercase()).or_insert_with(|| (tag.to_owned(), Vec::new())).1.extend(&keys);
            };
            if mode == "replace" {
                for old in snap.family_tags(fam) {
                    if !tags.iter().any(|t| t.to_lowercase() == old.to_lowercase()) {
                        push(&mut removes, &old);
                    }
                }
            }
            for t in &tags {
                push(if mode == "remove" { &mut removes } else { &mut adds }, t);
            }
        }
    }
    let mut store = state.store.lock().unwrap();
    for (tag, keys) in removes.values() {
        store.remove_tag(tag, keys)?;
    }
    let mut stored = Vec::new();
    for (tag, keys) in adds.values() {
        stored.push(store.add_tag(tag, keys)?);
    }
    state.refresh_tags(&store)?;
    Ok(compact(json!({
        "families": families,
        "added": stored,
        "removed": removes.into_values().map(|(t, _)| t).collect::<Vec<_>>(),
        "unknownFamilies": unknown,
    })))
}

// ── stdio bridge ────────────────────────────────────────────────────────────

type Socket = tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<std::net::TcpStream>>;

fn connect(launch: bool) -> Result<Socket> {
    let url = format!("ws://127.0.0.1:{}", crate::channel::PORT);
    if let Ok((ws, _)) = tungstenite::connect(&url) {
        return Ok(ws);
    }
    if !launch {
        bail!("{} is not running", crate::channel::APP);
    }
    eprintln!("starting {}…", crate::channel::APP);
    std::process::Command::new(std::env::current_exe()?)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .context("starting the app")?;
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        std::thread::sleep(Duration::from_millis(500));
        if let Ok((ws, _)) = tungstenite::connect(&url) {
            return Ok(ws);
        }
        if Instant::now() > deadline {
            bail!("{} did not open its local endpoint on port {}", crate::channel::APP, crate::channel::PORT);
        }
    }
}

/// Sends one message and, for a request, waits for its reply.
fn relay(ws: &mut Socket, line: &str, request: bool) -> Result<Option<String>> {
    use tungstenite::Message;
    ws.send(Message::text(line))?;
    if !request {
        return Ok(None);
    }
    loop {
        match ws.read()? {
            Message::Text(t) => return Ok(Some(t.to_string())),
            Message::Close(_) => bail!("the app closed the connection"),
            _ => {}
        }
    }
}

/// `--mcp`: MCP over stdio (newline-delimited JSON-RPC), relayed to the running app.
pub fn bridge() -> Result<()> {
    let mut ws = connect(true)?;
    let mut out = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let request = match serde_json::from_str::<Value>(&line) {
            Ok(v) => v.get("method").is_some() && v.get("id").is_some(),
            Err(e) => {
                writeln!(out, "{}", error(Value::Null, -32700, &format!("parse error: {e}")))?;
                out.flush()?;
                continue;
            }
        };
        let reply = match relay(&mut ws, &line, request) {
            Ok(r) => r,
            // The app restarted (an update, `pnpm preview`): reconnect once and resend.
            Err(_) => {
                ws = connect(true)?;
                relay(&mut ws, &line, request)?
            }
        };
        if let Some(reply) = reply {
            writeln!(out, "{reply}")?;
            out.flush()?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> (tempfile::TempDir, AppState) {
        let dir = tempfile::tempdir().unwrap();
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures");
        let backend: Box<dyn ts_activate::Backend> = Box::new(ts_activate::MemoryBackend::default());
        let state = AppState::open_with(dir.path().to_owned(), Some(backend)).unwrap();
        let folder = state.add_folder(&fixtures.canonicalize().unwrap(), false).unwrap();
        state.index_folder(&folder, &|_, _| {}).unwrap();
        (dir, state)
    }

    fn rpc(s: &AppState, method: &str, params: Value) -> Value {
        let (reply, _) = handle(s, &json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params }));
        reply.unwrap()
    }

    /// Calls a tool and parses its JSON text answer.
    fn tool(s: &AppState, name: &str, args: Value) -> Value {
        let r = rpc(s, "tools/call", json!({ "name": name, "arguments": args }));
        assert_eq!(r["result"]["isError"], Value::Null, "{r}");
        serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap()
    }

    fn names(v: &Value) -> Vec<String> {
        v["families"].as_array().unwrap().iter().map(|f| f["name"].as_str().unwrap().to_owned()).collect()
    }

    #[test]
    fn handshake_and_tool_list() {
        let (_dir, s) = state();
        let init = rpc(&s, "initialize", json!({ "protocolVersion": "2025-03-26", "capabilities": {} }));
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert!(init["result"]["capabilities"]["tools"].is_object());
        let unknown = rpc(&s, "initialize", json!({ "protocolVersion": "1999-01-01" }));
        assert_eq!(unknown["result"]["protocolVersion"], PROTOCOLS[0]);

        let (reply, _) = handle(&s, &json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }));
        assert!(reply.is_none(), "notifications get no reply");

        let tools = rpc(&s, "tools/list", json!({}));
        let names: Vec<&str> =
            tools["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"tag_families") && names.contains(&"preview_families"));
        assert_eq!(rpc(&s, "nope", json!({}))["error"]["code"], -32601);
    }

    #[test]
    fn tags_families_and_lists_what_is_left() {
        let (_dir, s) = state();
        let all = tool(&s, "list_families", json!({}));
        let total = all["total"].as_u64().unwrap();
        assert!(total >= 3);
        let first = names(&all)[0].clone();

        let (reply, changed) = handle(
            &s,
            &json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": { "name": "tag_families", "arguments": {
                "assignments": [{ "family": first.to_uppercase(), "tags": ["Sans/Grotesque", "#mood / neutral"] }, { "family": "Nope", "tags": ["x"] }],
            }}}),
        );
        assert!(changed);
        let out: Value =
            serde_json::from_str(reply.unwrap()["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(out["families"], 1);
        assert_eq!(out["unknownFamilies"], json!(["Nope"]));

        let left = tool(&s, "list_families", json!({ "query": "is:untagged" }));
        assert_eq!(left["total"].as_u64().unwrap(), total - 1);
        assert!(!names(&left).contains(&first));
        let sans = tool(&s, "list_families", json!({ "query": "#sans" }));
        assert_eq!(names(&sans), std::slice::from_ref(&first));
        assert_eq!(sans["families"][0]["tags"], json!(["mood/neutral", "Sans/Grotesque"]));

        let overview = tool(&s, "library_overview", json!({}));
        assert_eq!(overview["untaggedFamilies"].as_u64().unwrap(), total - 1);
        let tags: Vec<&str> = overview["tags"].as_array().unwrap().iter().map(|t| t["tag"].as_str().unwrap()).collect();
        assert_eq!(tags, ["mood", "mood/neutral", "Sans", "Sans/Grotesque"]);

        tool(
            &s,
            "tag_families",
            json!({ "assignments": [{ "family": first, "tags": ["sans/humanist"] }], "mode": "replace" }),
        );
        assert_eq!(tool(&s, "family_details", json!({ "family": first }))["tags"], json!(["sans/humanist"]));

        tool(&s, "rename_tag", json!({ "from": "sans", "to": "sans-serif" }));
        assert_eq!(
            names(&tool(&s, "list_families", json!({ "query": "#sans-serif/humanist" }))),
            std::slice::from_ref(&first)
        );
        tool(&s, "delete_tag", json!({ "tag": "sans-serif" }));
        assert_eq!(tool(&s, "list_families", json!({ "query": "is:tagged" }))["total"], 0);
    }

    #[test]
    fn pages_through_families() {
        let (_dir, s) = state();
        let page = tool(&s, "list_families", json!({ "limit": 2 }));
        assert_eq!(page["families"].as_array().unwrap().len(), 2);
        assert_eq!(page["next"], 2);
        let rest = tool(&s, "list_families", json!({ "offset": 2, "limit": 200 }));
        assert_eq!(rest["next"], Value::Null);
    }

    #[test]
    fn previews_are_images_with_a_legend() {
        let (_dir, s) = state();
        let fams = names(&tool(&s, "list_families", json!({ "limit": 3 })));
        let r = rpc(
            &s,
            "tools/call",
            json!({ "name": "preview_families", "arguments": { "families": [fams[0], "Missing"] } }),
        );
        let content = &r["result"]["content"];
        let legend = content[0]["text"].as_str().unwrap();
        assert!(legend.contains(&format!("1. {}", fams[0])) && legend.contains("2. Missing (not shown"), "{legend}");
        assert_eq!(content[1]["mimeType"], "image/webp");
        let bytes = base64::engine::general_purpose::STANDARD.decode(content[1]["data"].as_str().unwrap()).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");

        let bad = rpc(&s, "tools/call", json!({ "name": "preview_families", "arguments": { "families": [] } }));
        assert_eq!(bad["result"]["isError"], true);
    }
}
