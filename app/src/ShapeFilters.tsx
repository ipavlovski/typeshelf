import { useEffect, useState } from "react";
import { api, type Histogram } from "./api";

export type Ranges = Record<string, [number, number]>;

/** Range filters as query tokens, e.g. `contrast:0.5..1`. */
export function rangesToQuery(ranges: Ranges): string {
  return Object.entries(ranges)
    .map(([k, [lo, hi]]) => `${k}:${lo.toFixed(3)}..${hi.toFixed(3)}`)
    .join(" ");
}

/** SuperSearch: filter by measured shape, with each metric's distribution behind its slider. */
export function ShapeFilters({ ranges, onChange, version }: { ranges: Ranges; onChange: (r: Ranges) => void; version: number }) {
  const [hists, setHists] = useState<Histogram[]>([]);
  useEffect(() => {
    api.metricHistograms().then(setHists);
  }, [version]);

  return (
    <div className="shape-filters">
      {hists.map((h) => {
        const [lo, hi] = ranges[h.key] ?? [h.min, h.max];
        const peak = Math.max(...h.bins, 1);
        const step = (h.max - h.min) / 200;
        const at = (v: number) => ((v - h.min) / (h.max - h.min)) * 100;
        const set = (a: number, b: number) => {
          const next = { ...ranges };
          if (a <= h.min + step && b >= h.max - step) delete next[h.key];
          else next[h.key] = [Math.min(a, b), Math.max(a, b)];
          onChange(next);
        };
        return (
          <div key={h.key} className="metric">
            <div className="metric-head">
              <span>{h.label}</span>
              {ranges[h.key] && (
                <button className="ghost" onClick={() => set(h.min, h.max)}>
                  reset
                </button>
              )}
            </div>
            <div className="hist">
              {h.bins.map((n, i) => {
                const binLo = h.min + ((h.max - h.min) * i) / h.bins.length;
                const inside = binLo >= lo - step && binLo <= hi;
                return <div key={i} className={inside ? "in" : ""} style={{ height: `${(Math.sqrt(n) / Math.sqrt(peak)) * 100}%` }} />;
              })}
              <div className="hist-range" style={{ left: `${at(lo)}%`, right: `${100 - at(hi)}%` }} />
            </div>
            <div className="dual">
              <input type="range" min={h.min} max={h.max} step={step} value={lo} onChange={(e) => set(Number(e.target.value), hi)} />
              <input type="range" min={h.min} max={h.max} step={step} value={hi} onChange={(e) => set(lo, Number(e.target.value))} />
            </div>
          </div>
        );
      })}
    </div>
  );
}
