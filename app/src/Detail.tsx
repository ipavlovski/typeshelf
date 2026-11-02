import { useEffect, useMemo, useState } from "react";
import { api, thumbUrl, type ActivationResult, type Face, type FamilyRow, type Settings } from "./api";
import { Dot } from "./FontList";

interface Props {
  family: FamilyRow;
  settings: Settings;
  onClose: () => void;
  onActivation: (op: Promise<ActivationResult | void>) => Promise<void>;
  onError: (msg: string) => void;
}

const WATERFALL = [12, 16, 20, 24, 32, 40, 56, 72, 96];

/** Named Unicode ranges for grouping the glyph grid. */
const BLOCKS: [number, number, string][] = [
  [0x0020, 0x007f, "Basic Latin"],
  [0x00a0, 0x00ff, "Latin-1"],
  [0x0100, 0x024f, "Latin Extended"],
  [0x0370, 0x03ff, "Greek"],
  [0x0400, 0x052f, "Cyrillic"],
  [0x0590, 0x05ff, "Hebrew"],
  [0x0600, 0x06ff, "Arabic"],
  [0x0900, 0x097f, "Devanagari"],
  [0x0e00, 0x0e7f, "Thai"],
  [0x1e00, 0x1eff, "Latin Extended Additional"],
  [0x2000, 0x206f, "Punctuation"],
  [0x20a0, 0x20cf, "Currency"],
  [0x2100, 0x214f, "Letterlike"],
  [0x2150, 0x218f, "Number forms"],
  [0x2190, 0x21ff, "Arrows"],
  [0x2200, 0x22ff, "Math"],
  [0x2500, 0x25ff, "Box drawing & shapes"],
  [0x2600, 0x27bf, "Symbols & dingbats"],
  [0x3000, 0x30ff, "CJK symbols & kana"],
  [0x4e00, 0x9fff, "CJK"],
  [0xac00, 0xd7af, "Hangul"],
  [0xe000, 0xf8ff, "Private use"],
  [0xfb00, 0xfdff, "Presentation forms"],
];
const blockOf = (cp: number) => BLOCKS.find(([a, b]) => cp >= a && cp <= b)?.[2] ?? "Other";

/** Feature tags worth a toggle; kern/mark positioning features are always on. */
const SKIP_FEATURES = new Set(["kern", "mark", "mkmk", "abvm", "blwm", "ccmp", "locl", "rlig", "rvrn", "curs", "dist"]);
const GRID_LIMIT = 4000;

export function Detail({ family, settings, onClose, onActivation, onError }: Props) {
  const [faces, setFaces] = useState<Face[]>([]);
  const [face, setFace] = useState<Face | null>(null);
  const [fontName, setFontName] = useState<string | null>(null);
  const [tab, setTab] = useState<"specimen" | "glyphs" | "info">("specimen");
  const [text, setText] = useState(settings.sample);
  const [features, setFeatures] = useState<Record<string, boolean>>({});
  const [axes, setAxes] = useState<Record<string, number>>({});
  const [codepoints, setCodepoints] = useState<number[]>([]);
  const [copied, setCopied] = useState<string | null>(null);

  const reload = () =>
    api.familyFaces(family.id).then((fs) => {
      setFaces(fs);
      setFace((cur) => fs.find((f) => f.id === cur?.id) ?? fs.find((f) => f.id === family.preview) ?? fs[0] ?? null);
    });

  useEffect(() => {
    reload().catch((e) => onError(String(e)));
  }, [family.id]);

  // One face at a time through the FontFace API: cheap, and CSS gives OpenType and variable controls.
  useEffect(() => {
    if (!face) return;
    setFeatures({});
    setAxes(Object.fromEntries(face.axes.map((a) => [a.tag, a.default])));
    setFontName(null);
    if (face.format === "ttc") return; // FontFace cannot load a collection; fall back to rendered previews.
    let cancelled = false;
    let loaded: FontFace | null = null;
    api
      .fontBytes(face.id)
      .then(async (bytes) => {
        const name = `ts-${face.hash}-${face.index}`;
        loaded = new FontFace(name, bytes);
        await loaded.load();
        if (cancelled) return;
        document.fonts.add(loaded);
        setFontName(name);
      })
      .catch((e) => onError(`Could not load ${face.fullName}: ${e}`));
    api.faceCodepoints(face.id).then((cps) => !cancelled && setCodepoints(cps));
    return () => {
      cancelled = true;
      if (loaded) document.fonts.delete(loaded);
    };
  }, [face?.id]);

  const fontStyle = useMemo(
    () =>
      fontName
        ? {
            fontFamily: `"${fontName}"`,
            fontFeatureSettings: Object.entries(features).map(([t, on]) => `"${t}" ${on ? 1 : 0}`).join(", ") || "normal",
            fontVariationSettings: Object.entries(axes).map(([t, v]) => `"${t}" ${v}`).join(", ") || "normal",
          }
        : undefined,
    [fontName, features, axes],
  );

  const groups = useMemo(() => {
    const out = new Map<string, number[]>();
    for (const cp of codepoints.slice(0, GRID_LIMIT)) {
      if (cp < 0x20 || (cp >= 0x7f && cp < 0xa0)) continue;
      const b = blockOf(cp);
      if (!out.has(b)) out.set(b, []);
      out.get(b)!.push(cp);
    }
    return [...out.entries()];
  }, [codepoints]);

  const copy = (ch: string) => {
    navigator.clipboard.writeText(ch);
    setCopied(ch);
    setTimeout(() => setCopied(null), 1200);
  };

  const togglable = face?.features.filter((t) => !SKIP_FEATURES.has(t)) ?? [];
  const activeCount = faces.filter((f) => f.active).length;

  return (
    <div className="detail-backdrop" onClick={onClose}>
      <div className="detail" onClick={(e) => e.stopPropagation()}>
        <header className="detail-head">
          <Dot
            active={activeCount}
            total={faces.length}
            pinned={faces.some((f) => f.pinned)}
            onToggle={() => onActivation(api.setActive(faces.map((f) => f.id), activeCount < faces.length)).then(reload)}
          />
          <h2>{family.name}</h2>
          <nav className="tabs">
            {(["specimen", "glyphs", "info"] as const).map((t) => (
              <button key={t} className={tab === t ? "on" : ""} onClick={() => setTab(t)}>
                {t[0].toUpperCase() + t.slice(1)}
              </button>
            ))}
          </nav>
          <button className="ghost close" onClick={onClose} aria-label="Close">
            ✕
          </button>
        </header>
        <div className="detail-body">
          <aside className="styles-list">
            {faces.map((f) => (
              <div key={f.id} className={`style-item ${f.id === face?.id ? "on" : ""}`} onClick={() => setFace(f)}>
                <Dot
                  active={f.active ? 1 : 0}
                  total={1}
                  pinned={f.pinned}
                  onToggle={(e) => onActivation(e.altKey ? api.setActive([f.id], true, !f.pinned) : api.setActive([f.id], !f.active)).then(reload)}
                />
                <span>{f.style}</span>
                <small>{f.weight}</small>
              </div>
            ))}
          </aside>
          <section className="detail-main">
            {tab === "specimen" && (
              <>
                <input className="sample wide" value={text} onChange={(e) => setText(e.target.value)} />
                {face && face.axes.length > 0 && fontName && (
                  <div className="controls">
                    {face.axes.map((a) => (
                      <label key={a.tag}>
                        <span>{a.tag}</span>
                        <input
                          type="range"
                          min={a.min}
                          max={a.max}
                          step={(a.max - a.min) / 200}
                          value={axes[a.tag] ?? a.default}
                          onChange={(e) => setAxes({ ...axes, [a.tag]: Number(e.target.value) })}
                        />
                        <code>{Math.round(axes[a.tag] ?? a.default)}</code>
                      </label>
                    ))}
                  </div>
                )}
                {togglable.length > 0 && fontName && (
                  <div className="features">
                    {togglable.map((t) => (
                      <button key={t} className={features[t] ? "on" : ""} onClick={() => setFeatures({ ...features, [t]: !features[t] })}>
                        {t}
                      </button>
                    ))}
                  </div>
                )}
                <div className="waterfall">
                  {WATERFALL.map((size) => (
                    <div key={size} className="wf-row">
                      <small>{size}</small>
                      {fontStyle ? (
                        <div style={{ ...fontStyle, fontSize: size }}>{text}</div>
                      ) : face ? (
                        <img src={thumbUrl(face.id, size, text, 2000)} style={{ height: size * 1.4 }} alt="" />
                      ) : null}
                    </div>
                  ))}
                </div>
                {fontStyle && (
                  <p className="paragraph" style={{ ...fontStyle, fontSize: 18 }}>
                    {text} {text} Sphinx of black quartz, judge my vow. Pack my box with five dozen liquor jugs. 0123456789 &amp;?!
                  </p>
                )}
              </>
            )}
            {tab === "glyphs" &&
              (fontStyle ? (
                <div className="glyphs">
                  {codepoints.length > GRID_LIMIT && <p className="muted">Showing the first {GRID_LIMIT.toLocaleString()} of {codepoints.length.toLocaleString()} characters.</p>}
                  {groups.map(([name, cps]) => (
                    <div key={name}>
                      <h4>
                        {name} <span className="muted">{cps.length}</span>
                      </h4>
                      <div className="glyph-grid" style={fontStyle}>
                        {cps.map((cp) => {
                          const ch = String.fromCodePoint(cp);
                          return (
                            <button key={cp} title={`U+${cp.toString(16).toUpperCase().padStart(4, "0")} — click to copy`} onClick={() => copy(ch)}>
                              {ch}
                            </button>
                          );
                        })}
                      </div>
                    </div>
                  ))}
                  {copied && <div className="toast">Copied {copied}</div>}
                </div>
              ) : (
                <p className="muted">Glyphs are not available for font collections (.ttc) yet.</p>
              ))}
            {tab === "info" && face && (
              <table className="info">
                <tbody>
                  {(
                    [
                      ["Full name", face.fullName],
                      ["PostScript", face.psName],
                      ["Style", `${face.style} · weight ${face.weight} · width ${face.width}${face.italic ? " · italic" : ""}`],
                      ["Format", `${face.format.toUpperCase()} · ${face.glyphs} glyphs`],
                      ["Scripts", face.scripts.join(", ")],
                      ["Features", face.features.join(" ")],
                      ["Axes", face.axes.map((a) => `${a.tag} ${a.min}–${a.max}`).join(", ")],
                      ["Vendor", face.vendor],
                      ["Licence", face.license],
                      ["File", face.path],
                    ] as const
                  )
                    .filter(([, v]) => v)
                    .map(([k, v]) => (
                      <tr key={k}>
                        <th>{k}</th>
                        <td>{v}</td>
                      </tr>
                    ))}
                </tbody>
              </table>
            )}
            {tab === "info" && face && (
              <button className="ghost" onClick={() => api.reveal(face.id)}>
                Show in folder
              </button>
            )}
          </section>
        </div>
      </div>
    </div>
  );
}
