use std::path::PathBuf;

use ts_model::{ActivationMode::*, FileHash};
use ts_store::{JournalState, Store};

use super::*;

struct Env {
    store: Store,
    backend: MemoryBackend,
    dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        Self {
            store: Store::open_in_memory().unwrap(),
            backend: MemoryBackend::default(),
            dir: tempfile::tempdir().unwrap(),
        }
    }

    /// Creates a real file so reconcile sees it as present.
    fn req(&self, n: u64) -> Request {
        let path = self.file(&format!("f{n}.ttf"));
        std::fs::write(&path, b"font").unwrap();
        Request { hash: FileHash(n), path: path.to_string_lossy().into_owned(), name: format!("Font {n}") }
    }

    fn file(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn os_count(&self) -> usize {
        self.backend.entries.lock().unwrap().len()
    }
}

#[test]
fn activates_a_batch_with_one_notification() {
    let mut env = Env::new();
    let reqs: Vec<_> = (1..=300).map(|n| env.req(n)).collect();
    let out = activate(&mut env.store, &env.backend, &reqs, Session).unwrap();
    assert_eq!(out.changed.len(), 300);
    assert_eq!(env.os_count(), 300);
    assert_eq!(env.backend.notifications.load(std::sync::atomic::Ordering::Relaxed), 1);
    assert!(env.store.activations().unwrap().iter().all(|a| a.state == JournalState::Active));
}

#[test]
fn holders_are_counted() {
    let mut env = Env::new();
    let r = env.req(1);
    activate(&mut env.store, &env.backend, std::slice::from_ref(&r), Session).unwrap();
    let out = activate(&mut env.store, &env.backend, std::slice::from_ref(&r), Session).unwrap();
    assert_eq!(out.held, [r.hash]);
    assert_eq!(
        env.backend.notifications.load(std::sync::atomic::Ordering::Relaxed),
        1,
        "second holder changes nothing in the OS"
    );

    deactivate(&mut env.store, &env.backend, &[r.hash], false).unwrap();
    assert_eq!(env.os_count(), 1, "one holder left");
    deactivate(&mut env.store, &env.backend, &[r.hash], false).unwrap();
    assert_eq!(env.os_count(), 0);
    assert!(env.store.activations().unwrap().is_empty());
}

#[test]
fn force_ignores_holders() {
    let mut env = Env::new();
    let r = env.req(1);
    for _ in 0..3 {
        activate(&mut env.store, &env.backend, std::slice::from_ref(&r), Session).unwrap();
    }
    deactivate(&mut env.store, &env.backend, &[r.hash], true).unwrap();
    assert_eq!(env.os_count(), 0);
}

#[test]
fn failed_registration_is_rolled_back() {
    let mut env = Env::new();
    let ok = env.req(1);
    let bad = env.req(2);
    env.backend.fail.lock().unwrap().insert(bad.path.clone());
    let out = activate(&mut env.store, &env.backend, &[ok.clone(), bad.clone()], Session).unwrap();
    assert_eq!(out.changed, [ok.hash]);
    assert_eq!(out.failed.len(), 1);
    assert!(env.store.activation(bad.hash).unwrap().is_none());
}

#[test]
fn pinning_wins_and_survives_logon() {
    let mut env = Env::new();
    let session = env.req(1);
    let pinned = env.req(2);
    activate(&mut env.store, &env.backend, &[session.clone(), pinned.clone()], Session).unwrap();
    set_mode(&mut env.store, &env.backend, &[pinned.hash], Pinned).unwrap();
    // Activating again as session must not downgrade a pinned font.
    activate(&mut env.store, &env.backend, std::slice::from_ref(&pinned), Session).unwrap();
    assert_eq!(env.store.activation(pinned.hash).unwrap().unwrap().mode, Pinned);

    let rep = reconcile(&mut env.store, &env.backend, true).unwrap();
    assert_eq!(rep.session_cleared, 1);
    assert!(env.store.activation(session.hash).unwrap().is_none());
    assert_eq!(env.os_count(), 1);
}

#[test]
fn launch_reconcile_keeps_session_fonts() {
    let mut env = Env::new();
    let r = env.req(1);
    activate(&mut env.store, &env.backend, std::slice::from_ref(&r), Session).unwrap();
    let rep = reconcile(&mut env.store, &env.backend, false).unwrap();
    assert_eq!(rep, ReconcileReport::default());
    assert_eq!(env.os_count(), 1);
}

#[test]
fn crash_mid_activation_is_rolled_back() {
    let mut env = Env::new();
    let r = env.req(1);
    activate(&mut env.store, &env.backend, std::slice::from_ref(&r), Session).unwrap();
    // Simulate a crash between the journal write and the OS change.
    let mut a = env.store.activation(r.hash).unwrap().unwrap();
    a.state = JournalState::Activating;
    env.store.put_activations(&[a]).unwrap();

    let rep = reconcile(&mut env.store, &env.backend, false).unwrap();
    assert_eq!(rep.rolled_back, 1);
    assert_eq!(env.os_count(), 0);
}

#[test]
fn crash_mid_deactivation_is_finished() {
    let mut env = Env::new();
    let r = env.req(1);
    activate(&mut env.store, &env.backend, std::slice::from_ref(&r), Pinned).unwrap();
    let mut a = env.store.activation(r.hash).unwrap().unwrap();
    a.state = JournalState::Deactivating;
    env.store.put_activations(&[a]).unwrap();

    let rep = reconcile(&mut env.store, &env.backend, false).unwrap();
    assert_eq!(rep.finished, 1);
    assert_eq!(env.os_count(), 0);
}

#[test]
fn orphans_are_removed() {
    let mut env = Env::new();
    env.backend.entries.lock().unwrap().insert("Lost [typeshelf 00]".into(), ("/gone.ttf".into(), Session));
    let rep = reconcile(&mut env.store, &env.backend, false).unwrap();
    assert_eq!(rep.orphans, 1);
    assert_eq!(env.os_count(), 0);
}

#[test]
fn moved_files_are_repointed() {
    let mut env = Env::new();
    let r = env.req(1);
    activate(&mut env.store, &env.backend, std::slice::from_ref(&r), Pinned).unwrap();

    // The file moves and the indexer records its new path under the same hash.
    let new = env.file("moved.ttf");
    std::fs::rename(&r.path, &new).unwrap();
    let folder = env.store.add_folder(&env.dir.path().to_string_lossy(), ts_store::FolderKind::Watched).unwrap();
    let rec = ts_store::FileRecord {
        path: new.to_string_lossy().into_owned(),
        size: 4,
        mtime: 0,
        hash: Some(r.hash),
        status: ts_store::FileStatus::Ok,
        faces: vec![],
    };
    env.store.write_files(folder, &[rec]).unwrap();

    let rep = reconcile(&mut env.store, &env.backend, false).unwrap();
    assert_eq!(rep.repaired, 1);
    let entries = env.backend.entries.lock().unwrap();
    assert_eq!(entries.values().next().unwrap().0, new.to_string_lossy());
}

#[test]
fn missing_files_are_dropped() {
    let mut env = Env::new();
    let r = env.req(1);
    activate(&mut env.store, &env.backend, std::slice::from_ref(&r), Pinned).unwrap();
    std::fs::remove_file(&r.path).unwrap();
    let rep = reconcile(&mut env.store, &env.backend, false).unwrap();
    assert_eq!(rep.missing, 1);
    assert_eq!(env.os_count(), 0);
}

#[test]
fn flags_system_duplicates() {
    let mut env = Env::new();
    let r = env.req(1);
    env.backend.system.lock().unwrap().insert("font 1".into());
    let out = activate(&mut env.store, &env.backend, std::slice::from_ref(&r), Session).unwrap();
    assert_eq!(out.shadowed, [r.hash]);
}

#[test]
fn quitting_clears_only_session_fonts() {
    let mut env = Env::new();
    let s = env.req(1);
    let p = env.req(2);
    activate(&mut env.store, &env.backend, std::slice::from_ref(&s), Session).unwrap();
    activate(&mut env.store, &env.backend, std::slice::from_ref(&p), Pinned).unwrap();
    deactivate_session(&mut env.store, &env.backend).unwrap();
    assert_eq!(env.store.activations().unwrap().len(), 1);
    assert_eq!(env.os_count(), 1);
}
