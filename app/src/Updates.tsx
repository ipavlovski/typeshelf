import { useEffect, useState } from "react";
import { api, type AppInfo, type Release, type UpdateStatus } from "./api";
import { ReleaseNotes } from "./ReleaseNotes";
import { compareVersions, versionLabel } from "./versions";

// The Updates window: the app's GitHub releases and dev builds, newest first, with their notes. Any release newer
// than the installed version can be installed from here (dev builds only ever are; see src-tauri/src/updater.rs).

const date = (iso: string) => new Date(iso).toLocaleDateString(undefined, { year: "numeric", month: "short", day: "numeric" });

export function Updates({ app, status, onClose }: { app: AppInfo | null; status: UpdateStatus; onClose: () => void }) {
  const [releases, setReleases] = useState<Release[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [selected, setSelected] = useState<string | null>(null);
  const current = releases?.find((r) => r.tag === selected) ?? releases?.[0];

  const load = () => {
    setLoading(true);
    setError(null);
    api
      .listReleases()
      .then(setReleases)
      .catch((e) => setError(String(e)))
      .finally(() => setLoading(false));
  };
  useEffect(load, []);

  // Start on the newest release that would be an update, else the newest.
  useEffect(() => {
    if (!releases || selected || !app) return;
    setSelected((releases.find((r) => compareVersions(r.version, app.version) > 0) ?? releases[0])?.tag ?? null);
  }, [releases, selected, app]);

  return (
    <div className="detail-backdrop" onClick={onClose}>
      <div className="detail updates" onClick={(e) => e.stopPropagation()}>
        <header className="detail-head">
          <h2>Updates</h2>
          {app && (
            <span className="muted">
              Installed <b>{versionLabel(app.version)}</b>
              {app.channel === "dev" && " · typeshelfDev"}
            </span>
          )}
          <span style={{ flex: 1 }} />
          <button onClick={load} disabled={loading}>
            {loading ? "Loading…" : "Refresh"}
          </button>
          <button className="ghost" onClick={onClose} aria-label="Close">
            ✕
          </button>
        </header>
        <div className="detail-body">
          <nav className="styles-list" aria-label="Releases">
            {releases?.map((r) => {
              const cmp = app ? compareVersions(r.version, app.version) : 0;
              return (
                <div key={r.tag} className={`style-item release-item ${r.tag === current?.tag ? "on" : ""}`} onClick={() => setSelected(r.tag)}>
                  <span>
                    {versionLabel(r.version)}
                    {r.dev && <i className="chip">dev</i>}
                    {app && cmp === 0 && <i className="chip installed">installed</i>}
                    {app && cmp > 0 && <i className="new-dot" title="Newer than the installed version" />}
                  </span>
                  <small>{date(r.publishedAt)}</small>
                </div>
              );
            })}
          </nav>
          <article className="detail-main">
            {error ? (
              <p className="muted">Couldn’t load the releases: {error}</p>
            ) : !releases ? null : !current ? (
              <p className="muted">No releases are published yet.</p>
            ) : (
              <>
                <h2>{current.title}</h2>
                <p className="muted">
                  {date(current.publishedAt)} · {current.dev ? "Dev build (pre-release)" : "Release"} ·{" "}
                  <a href={current.url} target="_blank" rel="noreferrer">
                    View on GitHub ↗
                  </a>
                </p>
                <InstallAction release={current} app={app} status={status} />
                <ReleaseNotes source={current.notes} />
              </>
            )}
          </article>
        </div>
      </div>
    </div>
  );
}

/** Update / progress / restart for a release, depending on the installed version and the updater's status. */
function InstallAction({ release, app, status }: { release: Release; app: AppInfo | null; status: UpdateStatus }) {
  const [requested, setRequested] = useState<string | null>(null);
  if (!app) return null;
  if (app.channel === "dev") return <p className="note">This is typeshelfDev, which is updated by pnpm preview. Install releases in typeshelf.</p>;
  if (!app.updates) return <p className="note">Updates are installed by the released app.</p>;

  const cmp = compareVersions(release.version, app.version);
  if (cmp === 0) return <p className="note">This is the installed version.</p>;
  if (cmp < 0) return <p className="note">Older than the installed version {versionLabel(app.version)}.</p>;

  const label = versionLabel(release.version);
  const mine = requested === release.tag;
  if (status.state === "ready" && status.version === release.version)
    return (
      <div className="install-action">
        <button className="primary" onClick={() => api.installUpdate()}>
          Restart to install {label}
        </button>
        <span className="muted">Downloaded. It also installs when you quit.</span>
      </div>
    );
  const busy = mine && (status.state === "checking" || status.state === "downloading");
  return (
    <div className="install-action">
      <button
        className="primary"
        disabled={busy}
        onClick={() => {
          setRequested(release.tag);
          void api.installRelease(release.tag).catch(() => {}); // reported through the status
        }}
      >
        {busy ? (status.state === "downloading" ? `Downloading… ${status.percent}%` : "Starting…") : `Update to ${label}`}
      </button>
      {mine && status.state === "error" ? (
        <span className="warn">{status.message}</span>
      ) : (
        <span className="muted">Newer than the installed {versionLabel(app.version)}. Downloads in the background, then asks to restart.</span>
      )}
    </div>
  );
}
