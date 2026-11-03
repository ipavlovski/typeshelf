import { useCallback, useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, type ActivationResult, type FaceId, type FamilyRow, type Library, type Scope, type SearchResult, type Settings, DEFAULT_SETTINGS } from "./api";
import { Sidebar } from "./Sidebar";
import { FontList } from "./FontList";
import { Detail } from "./Detail";
import { GoogleList } from "./GoogleList";
import { ImportDialog } from "./ImportDialog";
import { ShapeFilters, rangesToQuery, type Ranges } from "./ShapeFilters";
import { Settings as SettingsWindow } from "./Settings";
import { Updates } from "./Updates";
import { useUpdates } from "./useUpdates";
import { versionLabel } from "./versions";
import { SearchBox, TagInput, toggleTag } from "./Tags";

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
  const [importing, setImporting] = useState(false);
  const [showShape, setShowShape] = useState(false);
  const [ranges, setRanges] = useState<Ranges>({});
  const [libraryVersion, setLibraryVersion] = useState(0);
  const [tagging, setTagging] = useState(false);
  const [panel, setPanel] = useState<"settings" | "updates" | null>(null);
  const { app, status } = useUpdates();
  const toastTimer = useRef<number>(undefined);

  const notify = useCallback((msg: string) => {
    setToast(msg);
    window.clearTimeout(toastTimer.current);
    toastTimer.current = window.setTimeout(() => setToast(null), 6000);
  }, []);

  const refreshLibrary = useCallback(() => api.library().then(setLibrary).catch((e) => notify(String(e))), [notify]);

  const runSearch = useCallback(async () => {
    try {
      const res = await api.search(`${query} ${rangesToQuery(ranges)}`, scope);
      // null = superseded by a newer keystroke.
      if (res) {
        setResult(res);
        if (!performance.getEntriesByName("first-results").length) performance.mark("first-results");
      }
    } catch (e) {
      notify(String(e));
    }
  }, [query, ranges, scope, notify]);

  useEffect(() => {
    api.getSettings().then(setSettings);
    refreshLibrary();
    const unlisten = [
      listen<Progress>("index-progress", (e) => setProgress(e.payload.done < e.payload.total ? e.payload : null)),
      listen("library-changed", () => {
        setProgress(null);
        setLibraryVersion((v) => v + 1);
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

  /** Tags or untags faces, then refreshes chips and counts. */
  const tag = async (tag: string, faces: FaceId[], add: boolean) => {
    try {
      const stored = await api.tagSet(tag, faces, add);
      if (add) notify(`Tagged ${faces.length.toLocaleString()} ${faces.length === 1 ? "style" : "styles"} #${stored}`);
    } catch (e) {
      notify(String(e));
    }
    await activation(Promise.resolve());
  };
  const tagFilter = (t: string) => setQuery((q) => toggleTag(q, t));

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
        onImport={() => setImporting(true)}
        query={query}
        onTagFilter={tagFilter}
        onTagsChanged={() => activation(Promise.resolve())}
        onSettings={() => setPanel("settings")}
        updatePending={status.state === "ready"}
        dropTarget={(collectionId) => {
          const faces = selectedFaces();
          if (faces.length) api.collectionAdd(collectionId, faces, true).then(refreshLibrary);
        }}
      />
      <main className="main">
        <header className="toolbar">
          <SearchBox value={query} onChange={setQuery} tags={library?.tags ?? []} />
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
          <button
            className={showShape || Object.keys(ranges).length ? "on" : "ghost"}
            onClick={() => setShowShape(!showShape)}
            title="Filter by measured x-height, weight, width and contrast"
          >
            Shape{Object.keys(ranges).length ? ` (${Object.keys(ranges).length})` : ""}
          </button>
          {library?.active ? (
            <button className="ghost" onClick={() => activation(api.deactivateAll())} title="Deactivate every session font">
              Deactivate all
            </button>
          ) : null}
        </header>
        {showShape && scope.kind !== "google" && <ShapeFilters ranges={ranges} onChange={setRanges} version={libraryVersion} />}
        {selected.size > 0 && (
          <div className="selection-bar">
            <span>{selected.size} families selected</span>
            <button onClick={() => activation(api.setActive(selectedFaces(), true))}>Activate</button>
            <button onClick={() => activation(api.setActive(selectedFaces(), false))}>Deactivate</button>
            <button onClick={() => activation(api.setActive(selectedFaces(), true, true))}>Pin</button>
            <button onClick={() => api.setFavourite(selectedFaces(), true).then(() => activation(Promise.resolve()))}>Favourite</button>
            {tagging ? (
              <TagInput
                tags={library?.tags ?? []}
                onDone={(t) => {
                  setTagging(false);
                  if (t) tag(t, selectedFaces(), true);
                }}
              />
            ) : (
              <button onClick={() => setTagging(true)} title="Tag the selected families; use / for sub-tags, e.g. serif/slab">
                Tag…
              </button>
            )}
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
        {scope.kind === "google" ? (
          <GoogleList query={query} settings={settings} onActivation={activation} onError={notify} />
        ) : (
        <FontList
          result={result}
          settings={settings}
          selected={selected}
          onSelect={setSelected}
          onOpen={setDetail}
          onActivation={activation}
          canActivate={library?.canActivate ?? false}
          query={query}
          onTagFilter={tagFilter}
          onUntag={async (fam, t) => tag(t, (await api.familyFaces(fam.id)).map((f) => f.id), false)}
        />
        )}
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
          {status.state === "ready" && (
            <button className="link" onClick={() => api.installUpdate()}>
              Update {versionLabel(status.version)} is ready: restart to install
            </button>
          )}
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
      {importing && (
        <ImportDialog
          onClose={() => setImporting(false)}
          onDone={(msg) => {
            notify(msg);
            refreshLibrary();
            runSearchRef.current();
          }}
        />
      )}
      {panel === "settings" && (
        <SettingsWindow
          app={app}
          status={status}
          settings={settings}
          onSettings={updateSettings}
          onUpdates={() => setPanel("updates")}
          onImport={() => {
            setPanel(null);
            setImporting(true);
          }}
          onClose={() => setPanel(null)}
        />
      )}
      {panel === "updates" && <Updates app={app} status={status} onClose={() => setPanel("settings")} />}
      {toast && (
        <div className="toast" onClick={() => setToast(null)}>
          {toast}
        </div>
      )}
    </div>
  );
}
