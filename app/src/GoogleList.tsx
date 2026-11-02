import { useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { api, googleThumbUrl, lineHeight, type ActivationResult, type GoogleFamily, type Settings } from "./api";

interface Props {
  query: string;
  settings: Settings;
  onActivation: (op: Promise<ActivationResult | void>) => Promise<void>;
  onError: (msg: string) => void;
}

const CATEGORIES = ["All", "Sans Serif", "Serif", "Display", "Handwriting", "Monospace"];

/** Browses the Google Fonts catalogue; previews come from tiny subsets, installing downloads the family. */
export function GoogleList({ query, settings, onActivation, onError }: Props) {
  const [families, setFamilies] = useState<GoogleFamily[] | null>(null);
  const [category, setCategory] = useState("All");
  const [busy, setBusy] = useState<Set<string>>(new Set());
  const parent = useRef<HTMLDivElement>(null);

  useEffect(() => {
    api.googleCatalogue().then(setFamilies).catch((e) => onError(`Google Fonts unavailable: ${e}`));
  }, []);

  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    return (families ?? [])
      .filter((f) => (category === "All" || f.category === category) && (!q || f.family.toLowerCase().includes(q)))
      // `popularity` is a rank: 1 is the most popular.
      .sort((a, b) => a.popularity - b.popularity);
  }, [families, query, category]);

  const imgHeight = lineHeight(settings.size);
  const virtualizer = useVirtualizer({ count: shown.length, getScrollElement: () => parent.current, estimateSize: () => imgHeight + 30, overscan: 4 });
  useEffect(() => virtualizer.measure(), [imgHeight, virtualizer]);

  const install = async (f: GoogleFamily, activate: boolean) => {
    setBusy((b) => new Set(b).add(f.family));
    try {
      const faces = await api.googleInstall(f.family);
      setFamilies((all) => all?.map((x) => (x.family === f.family ? { ...x, installed: true } : x)) ?? null);
      if (activate) await onActivation(api.setActive(faces, true));
      else await onActivation(Promise.resolve());
    } catch (e) {
      onError(`Could not install ${f.family}: ${e}`);
    } finally {
      setBusy((b) => {
        const n = new Set(b);
        n.delete(f.family);
        return n;
      });
    }
  };

  if (!families) return <div className="empty">Loading the Google Fonts catalogue…</div>;
  return (
    <>
      <div className="selection-bar">
        {CATEGORIES.map((c) => (
          <button key={c} className={category === c ? "on" : "ghost"} onClick={() => setCategory(c)}>
            {c}
          </button>
        ))}
        <span className="muted">{shown.length.toLocaleString()} families</span>
      </div>
      <div className="list" ref={parent}>
        <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
          {virtualizer.getVirtualItems().map((v) => {
            const f = shown[v.index];
            return (
              <div key={f.family} className="row family-row" style={{ transform: `translateY(${v.start}px)`, height: v.size }}>
                <div className="row-head">
                  <span className="family-name">{f.family}</span>
                  <span>
                    {f.category} · {f.styles} styles{f.variable ? " · variable" : ""}
                  </span>
                  <span style={{ flex: 1 }} />
                  {f.installed ? (
                    <span className="badge">in library</span>
                  ) : busy.has(f.family) ? (
                    <span className="muted">Downloading…</span>
                  ) : (
                    <>
                      <button className="ghost" onClick={() => install(f, false)}>
                        Add to library
                      </button>
                      <button onClick={() => install(f, true)}>Add &amp; activate</button>
                    </>
                  )}
                </div>
                <img className="preview" src={googleThumbUrl(f.family, settings.size, settings.sample)} style={{ height: imgHeight }} alt="" loading="lazy" />
              </div>
            );
          })}
        </div>
      </div>
    </>
  );
}
