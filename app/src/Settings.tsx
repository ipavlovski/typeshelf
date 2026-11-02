import { useState } from "react";
import { api, DEFAULT_SAMPLE, type AppInfo, type Settings as Prefs, type UpdateStatus } from "./api";
import { versionLabel } from "./versions";

interface Props {
  app: AppInfo | null;
  status: UpdateStatus;
  settings: Prefs;
  onSettings: (s: Prefs) => void;
  onUpdates: () => void;
  onImport: () => void;
  onClose: () => void;
}

/** Settings: preview defaults, library import, and the installed version with its updates. */
export function Settings({ app, status, settings, onSettings, onUpdates, onImport, onClose }: Props) {
  const [checkResult, setCheckResult] = useState<string | null>(null);
  const [checking, setChecking] = useState(false);
  const pending = status.state === "downloading" || status.state === "ready";

  const check = async () => {
    setChecking(true);
    setCheckResult(null);
    try {
      const v = await api.checkForUpdates();
      setCheckResult(v ? `Downloaded ${versionLabel(v)}.` : "You have the latest version.");
    } catch (e) {
      setCheckResult(String(e));
    } finally {
      setChecking(false);
    }
  };

  return (
    <div className="detail-backdrop" onClick={onClose}>
      <div className="dialog settings" onClick={(e) => e.stopPropagation()}>
        <header className="dialog-head">
          <h2>Settings</h2>
          <button className="ghost" onClick={onClose} aria-label="Close">
            ✕
          </button>
        </header>

        <section>
          <h3>Preview</h3>
          <label className="row-setting">
            <span>Sample text</span>
            <input value={settings.sample} onChange={(e) => onSettings({ ...settings, sample: e.target.value || DEFAULT_SAMPLE })} />
          </label>
          <label className="row-setting">
            <span>Size</span>
            <input type="range" min={14} max={120} value={settings.size} onChange={(e) => onSettings({ ...settings, size: Number(e.target.value) })} />
            <code>{settings.size}px</code>
          </label>
        </section>

        <section>
          <h3>Library</h3>
          <div className="row-setting">
            <span>FontBase</span>
            <button onClick={onImport}>Import from FontBase…</button>
          </div>
        </section>

        <section>
          <h3>Updates</h3>
          <div className="row-setting">
            <span>
              Installed {app ? <b>{versionLabel(app.version)}</b> : "…"}
              {app?.channel === "dev" && ` · typeshelfDev${app.label ? ` (${app.label})` : ""}`}
              {pending && <i className="new-dot" title="An update is downloading or ready" />}
            </span>
            <span className="row-actions">
              {app?.updates && (
                <button onClick={check} disabled={checking || status.state === "downloading"}>
                  {checking ? "Checking…" : "Check for updates"}
                </button>
              )}
              <button onClick={onUpdates}>Updates and releases</button>
            </span>
          </div>
          {status.state === "downloading" && <p className="muted">Downloading {versionLabel(status.version)}… {status.percent}%</p>}
          {status.state === "ready" && (
            <div className="install-action">
              <button className="primary" onClick={() => api.installUpdate()}>
                Restart to install {versionLabel(status.version)}
              </button>
              <span className="muted">It also installs when you quit.</span>
            </div>
          )}
          {checkResult && <p className="muted">{checkResult}</p>}
          {app && !app.updates && (
            <p className="muted">{app.channel === "dev" ? "typeshelfDev is updated with pnpm preview." : "This build doesn't update itself."}</p>
          )}
        </section>
      </div>
    </div>
  );
}
