import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, type ActivationResult, type FamilyRow, type Library, type Scope, type SearchResult, type Settings, DEFAULT_SETTINGS } from "./api";
import { Sidebar } from "./Sidebar";
import { FontList } from "./FontList";
import { Detail } from "./Detail";

interface Progress {
  folder: string;
  done: number;
  total: number;
}

export function App() {
  const [library, setLibrary] = useState<Library | null>(null);
  const [scope, setScope] = useState<Scope>({ kind: "all" });
  const [query, setQuery] = useState("");
  const [result, setResult] = useState<SearchResult | null>(null);
  const [settings, setSettings] = useState<Settings>(DEFAULT_SETTINGS);
  const [selected, setSelected] = useState<Set<number>>(new Set());
  const [detail, setDetail] = useState<FamilyRow | null>(null);
  const [toast, setToast] = useState<string | null>(null);
  const [progress, setProgress] = useState<Progress | null>(null);
  const toastTimer = useRef<number>(undefined);

  const notify = useCallback((msg: string) => {
    setToast(msg);
    window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setToast(null), 6000);
  }, []);

  const refreshLibrary = useCallback(() => api.library().then(setLibrary).catch((e) => notify(String(e))), [notify]);

  const runSearch = useCallback(async () => {
    try {
      const res = await api.search(query, scope);
      // null = superseded by a newer keystroke.
      if (res) {
        setResult(res);
        if (!performance.getEntriesByName("first-results").length) performance.mark("first-results");
      }
    } catch (e) {
      notify(String(e));
    }
  }, [query, scope, notify]);

  useEffect(() => {
    api.getSettings().then(setSettings);
    refreshLibrary();
    const unlisten = [
      listen<Progress>("index-progress", (e) => setProgress(e.payload.done < e.payload.total ? e.payload : null)),
      listen("library-changed", () => {
        setProgress(null);
        refreshLibrary();
        runSearchRef.current();
      }),
    ];
    return () => unlisten.forEach((p) => p.then((f) => f()));
  }, [refreshLibrary]);

  const runSearchRef = useRef(runSearch);
  runSearchRef.current = runSearch;
  useEffect(() => {
    runSearch();
  }, [runSearch]);

  const updateSettings = (s: Settings) => {
    setSettings(s);
    api.setSettings(s);
  };

  /** Runs an activation command, then refreshes counts and dots. */
  const activation = useCallback(
    async (op: Promise<ActivationResult | void>) => {
      try {
        const res = await op;
        if (res) {
          if (res.failed.length) notify(`${res.failed.length} fonts failed: ${res.failed[0]}`);
          else if (res.shadowed) notify(`${res.shadowed} fonts are also installed system-wide; deactivating them here won't hide them.`);
          else if (res.restartApps.length) notify(`Restart ${res.restartApps.join(", ")} to see the changes.`);
        }
      } catch (e) {
        notify(String(e));
      }
      await Promise.all([runSearchRef.current(), refreshLibrary()]);
    },
    [notify, refreshLibrary],
  );

  const selectedFaces = () =>
    (result?.families ?? []).filter((f) => selected.has(f.id)).flatMap((f) => f.faceIds);

  return (
    <div className="app">
      <Sidebar
        library={library}
        scope={scope}
        onScope={(s) => {
          setScope(s);
          setSelected(new Set());
        }}
        onChanged={refreshLibrary}
        onActivation={activation}
        onError={notify}
        dropTarget={(collectionId) => {
          const faces = selectedFaces();
          if (faces.length) api.collectionAdd(collectionId, faces, true).then(refreshLibrary);
        }}
      />
      <main className="main">
        <header className="toolbar">
          <input
            className="search"
            placeholder="Search — try script:cyrillic wght:>600 feat:ss01 is:active"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            spellCheck={false}
            autoFocus
          />
          <input
            className="sample"
            placeholder="Sample text"
            value={settings.sample}
            onChange={(e) => updateSettings({ ...settings, sample: e.target.value || DEFAULT_SETTINGS.sample })}
          />
          <label className="size">
            <input
              type="range"
              min={14}
              max={120}
              value={settings.size}
              onChange={(e) => updateSettings({ ...settings, size: Number(e.target.value) })}
            />
            <span>{settings.size}px</span>
          </label>
          {library?.active ? (
            <button className="ghost" onClick={() => activation(api.deactivateAll())} title="Deactivate every session font">
              Deactivate all
            </button>
          ) : null}
        </header>
        {selected.size > 0 && (
          <div className="selection-bar">
            <span>{selected.size} families selected</span>
            <button onClick={() => activation(api.setActive(selectedFaces(), true))}>Activate</button>
            <button onClick={() => activation(api.setActive(selectedFaces(), false))}>Deactivate</button>
            <button onClick={() => activation(api.setActive(selectedFaces(), true, true))}>Pin</button>
            <button onClick={() => api.setFavourite(selectedFaces(), true).then(() => activation(Promise.resolve()))}>Favourite</button>
            <select
              value=""
              onChange={(e) => {
                const id = Number(e.target.value);
                if (id) api.collectionAdd(id, selectedFaces(), true).then(refreshLibrary).then(() => notify("Added to collection"));
              }}
            >
              <option value="">Add to collection…</option>
              {library?.collections.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.name}
                </option>
              ))}
            </select>
            {scope.kind === "collection" && scope.id != null && (
              <button onClick={() => api.collectionAdd(scope.id!, selectedFaces(), false).then(() => activation(Promise.resolve()))}>
                Remove from collection
              </button>
            )}
            <button className="ghost" onClick={() => setSelected(new Set())}>
              Clear
            </button>
          </div>
        )}
        <FontList
          result={result}
          settings={settings}
          selected={selected}
          onSelect={setSelected}
          onOpen={setDetail}
          onActivation={activation}
          canActivate={library?.canActivate ?? false}
        />
        <footer className="status">
          {progress ? (
            <span>
              Indexing {progress.folder}: {progress.done.toLocaleString()} / {progress.total.toLocaleString()}
            </span>
          ) : result ? (
            <span>
              {result.families.length.toLocaleString()} families · {result.faces.toLocaleString()} styles · {(result.micros / 1000).toFixed(1)} ms
            </span>
          ) : null}
          {library && !library.canActivate && <span className="warn">Activation is not available on this OS yet</span>}
        </footer>
      </main>
      {detail && (
        <Detail
          family={detail}
          settings={settings}
          onClose={() => setDetail(null)}
          onActivation={activation}
          onError={notify}
        />
      )}
      {toast && (
        <div className="toast" onClick={() => setToast(null)}>
          {toast}
        </div>
      )}
    </div>
  );
}
