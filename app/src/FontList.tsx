import { useEffect, useRef, useState, type MouseEvent } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { TagChips } from "./Tags";
import { api, lineHeight, thumbUrl, type ActivationResult, type Face, type FamilyRow, type SearchResult, type Settings } from "./api";

interface Props {
  result: SearchResult | null;
  settings: Settings;
  selected: Set<number>;
  onSelect: (s: Set<number>) => void;
  onOpen: (f: FamilyRow) => void;
  onActivation: (op: Promise<ActivationResult | void>) => Promise<void>;
  canActivate: boolean;
  query: string;
  /** Toggles `#tag` in the search. */
  onTagFilter: (tag: string) => void;
  onUntag: (fam: FamilyRow, tag: string) => void;
}

const ROW_CHROME = 30; // name line + padding

/** Green when every face is active, half when some are. */
export function Dot({
  active,
  total,
  pinned,
  disabled,
  onToggle,
}: {
  active: number;
  total: number;
  pinned: boolean;
  disabled?: boolean;
  onToggle: (e: MouseEvent) => void;
}) {
  const state = active === 0 ? "off" : active === total ? "on" : "some";
  return (
    <button
      className={`dot ${state} ${pinned ? "pinned" : ""}`}
      disabled={disabled}
      title={
        disabled
          ? "Activation unavailable"
          : `${state === "on" ? "Deactivate" : "Activate"} · Alt+click to ${pinned ? "unpin" : "pin"} (survives reboots)`
      }
      onClick={(e) => {
        e.stopPropagation();
        onToggle(e);
      }}
    />
  );
}

export function FontList({ result, settings, selected, onSelect, onOpen, onActivation, canActivate, query, onTagFilter, onUntag }: Props) {
  const parent = useRef<HTMLDivElement>(null);
  const [expanded, setExpanded] = useState<Map<number, Face[]>>(new Map());
  const anchor = useRef<number | null>(null);
  const families = result?.families ?? [];
  const imgHeight = lineHeight(settings.size);

  // Expanded families add one row per style below them.
  type Item = { kind: "family"; fam: FamilyRow; index: number } | { kind: "face"; face: Face; fam: FamilyRow };
  const items: Item[] = [];
  families.forEach((fam, index) => {
    items.push({ kind: "family", fam, index });
    const faces = expanded.get(fam.id);
    if (faces) faces.filter((f) => fam.faceIds.includes(f.id)).forEach((face) => items.push({ kind: "face", face, fam }));
  });

  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => parent.current,
    estimateSize: (i) => (items[i].kind === "family" ? imgHeight + ROW_CHROME : imgHeight + 8),
    overscan: 6,
  });

  useEffect(() => virtualizer.measure(), [imgHeight, expanded, virtualizer]);
  useEffect(() => {
    parent.current?.scrollTo({ top: 0 });
  }, [result?.families.length === 0]);

  // Refresh expanded rows' activation state after any change.
  useEffect(() => {
    if (expanded.size === 0) return;
    Promise.all([...expanded.keys()].map(async (id) => [id, await api.familyFaces(id)] as const)).then((pairs) =>
      setExpanded(new Map(pairs)),
    );
  }, [result]);

  const toggleExpand = async (fam: FamilyRow) => {
    const next = new Map(expanded);
    if (next.has(fam.id)) next.delete(fam.id);
    else next.set(fam.id, await api.familyFaces(fam.id));
    setExpanded(next);
  };

  const click = (e: MouseEvent, fam: FamilyRow, index: number) => {
    const next = new Set(e.ctrlKey || e.metaKey ? selected : []);
    if (e.shiftKey && anchor.current != null) {
      const [a, b] = [Math.min(anchor.current, index), Math.max(anchor.current, index)];
      for (let i = a; i <= b; i++) next.add(families[i].id);
    } else if ((e.ctrlKey || e.metaKey) && next.has(fam.id)) {
      next.delete(fam.id);
    } else {
      next.add(fam.id);
      anchor.current = index;
    }
    onSelect(next);
  };

  const toggleFamily = (fam: FamilyRow, e: MouseEvent) => {
    if (e.altKey) onActivation(api.setActive(fam.faceIds, true, fam.pinned === 0));
    else onActivation(api.setActive(fam.faceIds, fam.active < fam.faces));
  };

  if (result && families.length === 0) {
    return <div className="empty">No fonts match. Add a folder in the sidebar, or loosen the search.</div>;
  }

  return (
    <div className="list" ref={parent}>
      <div style={{ height: virtualizer.getTotalSize(), position: "relative" }}>
        {virtualizer.getVirtualItems().map((v) => {
          const item = items[v.index];
          const style = { transform: `translateY(${v.start}px)`, height: v.size };
          if (item.kind === "face") {
            const f = item.face;
            return (
              <div key={`f${f.id}`} className="row face-row" style={style} onDoubleClick={() => onOpen(item.fam)}>
                <Dot
                  active={f.active ? 1 : 0}
                  total={1}
                  pinned={f.pinned}
                  disabled={!canActivate}
                  onToggle={(e) => onActivation(e.altKey ? api.setActive([f.id], true, !f.pinned) : api.setActive([f.id], !f.active))}
                />
                <span className="face-style">{f.style}</span>
                <img className="preview" src={thumbUrl(f.id, settings.size, settings.sample)} style={{ height: imgHeight }} alt="" draggable={false} />
              </div>
            );
          }
          const fam = item.fam;
          const isOpen = expanded.has(fam.id);
          return (
            <div
              key={`F${fam.id}`}
              className={`row family-row ${selected.has(fam.id) ? "selected" : ""}`}
              style={style}
              onClick={(e) => click(e, fam, item.index)}
              onDoubleClick={() => onOpen(fam)}
            >
              <div className="row-head">
                <Dot active={fam.active} total={fam.faces} pinned={fam.pinned > 0} disabled={!canActivate} onToggle={(e) => toggleFamily(fam, e)} />
                <span className="family-name">{fam.name}</span>
                {fam.faces > 1 && (
                  <button
                    className="styles"
                    onClick={(e) => {
                      e.stopPropagation();
                      toggleExpand(fam);
                    }}
                  >
                    {isOpen ? "▾" : "▸"} {fam.faces} styles
                  </button>
                )}
                {fam.favourites > 0 && <span className="fav">★</span>}
                {fam.pinned > 0 && <span className="badge" title="Pinned: survives reboots">pinned</span>}
                {fam.tags.length > 0 && <TagChips tags={fam.tags} query={query} onFilter={onTagFilter} onRemove={(t) => onUntag(fam, t)} />}
              </div>
              <img className="preview" src={thumbUrl(fam.preview, settings.size, settings.sample)} style={{ height: imgHeight }} alt="" draggable={false} />
            </div>
          );
        })}
      </div>
    </div>
  );
}
