import { useEffect, useState } from "react";
import { api, type ImportPreview } from "./api";

interface Props {
  onClose: () => void;
  onDone: (msg: string) => void;
}

/** Shows what will be imported from FontBase, then runs the import. */
export function ImportDialog({ onClose, onDone }: Props) {
  const [preview, setPreview] = useState<ImportPreview | null | undefined>(undefined);
  const [download, setDownload] = useState(true);
  const [activate, setActivate] = useState<"none" | "session" | "pinned">("session");
  const [running, setRunning] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    api.fontbasePreview().then(setPreview).catch((e) => setError(String(e)));
  }, []);

  const toDownload = preview?.collections.reduce((n, c) => n + c.toDownload, 0) ?? 0;
  const run = async () => {
    setRunning(true);
    try {
      const r = await api.fontbaseImport(download, activate);
      onDone(
        `Imported ${r.created + r.reused} collections with ${(r.members + r.resolved).toLocaleString()} fonts` +
          (r.activated ? `, re-activated ${r.activated}` : "") +
          (r.pending ? `. ${r.pending} fonts are still pending.` : "."),
      );
      onClose();
    } catch (e) {
      setError(String(e));
      setRunning(false);
    }
  };

  return (
    <div className="detail-backdrop" onClick={running ? undefined : onClose}>
      <div className="dialog" onClick={(e) => e.stopPropagation()}>
        <h2>Import from FontBase</h2>
        {preview === undefined && !error && <p className="muted">Reading FontBase's library…</p>}
        {preview === null && <p>No FontBase library was found on this computer.</p>}
        {error && <p className="warn">{error}</p>}
        {preview && (
          <>
            <p>
              Root folder <code>{preview.root ?? "none"}</code> {preview.rootIndexed ? "(already in your library)" : "will be added and indexed."}
            </p>
            <div className="import-tree">
              {preview.collections.map((c, i) => (
                <div key={i} style={{ paddingLeft: c.depth * 16 }}>
                  <span>{c.name}</span>
                  <span className="muted">
                    {c.fonts} fonts{c.toDownload ? ` + ${c.toDownload} to download` : ""}
                    {c.missing ? ` · ${c.missing} not found` : ""}
                  </span>
                </div>
              ))}
            </div>
            <label className="check">
              <input type="checkbox" checked={download} onChange={(e) => setDownload(e.target.checked)} disabled={!toDownload} />
              Download the {toDownload} Google Fonts your collections use but FontBase never downloaded
            </label>
            <label className="check">
              Re-activate FontBase's {preview.active} active fonts:
              <select value={activate} onChange={(e) => setActivate(e.target.value as typeof activate)}>
                <option value="none">no</option>
                <option value="session">for this session</option>
                <option value="pinned">pinned</option>
              </select>
            </label>
            <p className="muted">FontBase is not changed. Running the import again later only adds what is new.</p>
          </>
        )}
        <div className="dialog-actions">
          <button className="ghost" onClick={onClose} disabled={running}>
            Cancel
          </button>
          <button className="primary" onClick={run} disabled={!preview || running}>
            {running ? "Importing…" : "Import"}
          </button>
        </div>
      </div>
    </div>
  );
}
