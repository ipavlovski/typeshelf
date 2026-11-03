import { useRef, useState, type KeyboardEvent } from "react";
import type { Tag } from "./api";

/** `#serif/slab`, quoted when the tag has spaces. */
export const tagToken = (path: string) => (/\s/.test(path) ? `#"${path}"` : `#${path}`);

/** Splits a query into tokens, keeping `"quoted parts"` (quotes included) together. */
function tokens(query: string): string[] {
  return query.match(/(?:[^\s"]+|"[^"]*"?)+/g) ?? [];
}

const tagOf = (token: string) => token.replace(/^#/, "").replaceAll('"', "").toLowerCase();

/** Whether the query filters by exactly this tag. */
export const queryHasTag = (query: string, path: string) => tokens(query).some((t) => t.startsWith("#") && tagOf(t) === path.toLowerCase());

/** Adds `#tag` to the query, or takes it out if it is there. */
export function toggleTag(query: string, path: string): string {
  const all = tokens(query);
  const kept = all.filter((t) => !(t.startsWith("#") && tagOf(t) === path.toLowerCase()));
  return (kept.length === all.length ? [...kept, tagToken(path)] : kept).join(" ");
}

/** Tags whose path contains `typed`, best matches (prefix of the path or a segment) first. */
function suggest(tags: Tag[], typed: string, limit = 8): Tag[] {
  const q = typed.toLowerCase();
  const rank = (p: string) => (p.startsWith(q) ? 0 : p.split("/").some((s) => s.startsWith(q)) ? 1 : 2);
  return tags
    .filter((t) => t.path.toLowerCase().includes(q))
    .sort((a, b) => rank(a.path.toLowerCase()) - rank(b.path.toLowerCase()))
    .slice(0, limit);
}

function Suggestions({ items, index, onPick }: { items: Tag[]; index: number; onPick: (t: Tag) => void }) {
  if (!items.length) return null;
  return (
    <div className="suggest">
      {items.map((t, i) => (
        <button
          key={t.path}
          className={i === index ? "on" : ""}
          // mousedown, so the input keeps focus.
          onMouseDown={(e) => {
            e.preventDefault();
            onPick(t);
          }}
        >
          <TagPath path={t.path} />
          <span className="count">{t.faces.toLocaleString()}</span>
        </button>
      ))}
    </div>
  );
}

/** Arrow keys move, Enter/Tab picks, Escape closes. Returns true when it handled the key. */
function navigate(e: KeyboardEvent, items: Tag[], index: number, setIndex: (i: number) => void, pick: (t: Tag) => void, close: () => void) {
  if (!items.length) return false;
  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    setIndex((index + (e.key === "ArrowDown" ? 1 : items.length - 1)) % items.length);
  } else if ((e.key === "Enter" || e.key === "Tab") && index >= 0) {
    pick(items[index]);
  } else if (e.key === "Escape") {
    close();
  } else {
    return false;
  }
  e.preventDefault();
  return true;
}

/** `serif/slab` with dimmed separators. */
export function TagPath({ path }: { path: string }) {
  const parts = path.split("/");
  return (
    <span className="tag-path">
      {parts.map((p, i) => (
        <span key={i}>
          {i > 0 && <i>/</i>}
          {p}
        </span>
      ))}
    </span>
  );
}

/** The search box: completes `#tag` tokens from the library's tags. */
export function SearchBox({ value, onChange, tags }: { value: string; onChange: (v: string) => void; tags: Tag[] }) {
  const input = useRef<HTMLInputElement>(null);
  const [cursor, setCursor] = useState(0);
  const [index, setIndex] = useState(0);
  const [closed, setClosed] = useState(false);

  // The token being typed: from the last space before the cursor. `-#tag` negates.
  const start = value.lastIndexOf(" ", cursor - 1) + 1;
  const token = value.slice(start, cursor);
  const hash = token.startsWith("-#") ? 2 : token.startsWith("#") ? 1 : 0;
  const items = hash && !closed && !token.includes('"') ? suggest(tags, token.slice(hash)) : [];

  const pick = (t: Tag) => {
    const end = value.indexOf(" ", cursor);
    const rest = end < 0 ? "" : value.slice(end);
    const head = `${value.slice(0, start)}${token.slice(0, hash - 1)}${tagToken(t.path)} `;
    onChange(head + rest.trimStart());
    setClosed(true);
    requestAnimationFrame(() => input.current?.setSelectionRange(head.length, head.length));
  };

  const track = () => setCursor(input.current?.selectionStart ?? value.length);

  return (
    <div className="search-box">
      <input
        ref={input}
        className="search"
        placeholder="Search — try #tag script:cyrillic wght:>600 feat:ss01 is:active"
        value={value}
        onChange={(e) => {
          onChange(e.target.value);
          setCursor(e.target.selectionStart ?? e.target.value.length);
          setIndex(0);
          setClosed(false);
        }}
        onKeyDown={(e) => navigate(e, items, index, setIndex, pick, () => setClosed(true))}
        onKeyUp={track}
        onClick={track}
        onBlur={() => setClosed(true)}
        spellCheck={false}
        autoFocus
      />
      <Suggestions items={items} index={index} onPick={pick} />
    </div>
  );
}

/** A one-off input for naming a tag, suggesting existing ones. Enter applies, Escape cancels. */
export function TagInput({ tags, onDone, placeholder = "tag/subtag" }: { tags: Tag[]; onDone: (tag: string | null) => void; placeholder?: string }) {
  const [value, setValue] = useState("");
  const [index, setIndex] = useState(-1);
  const typed = value.replace(/^#/, "").trim();
  const items = typed ? suggest(tags, typed).filter((t) => t.path.toLowerCase() !== typed.toLowerCase()) : [];
  return (
    <div className="search-box tag-input">
      <input
        autoFocus
        value={value}
        placeholder={placeholder}
        spellCheck={false}
        onChange={(e) => {
          setValue(e.target.value);
          setIndex(-1);
        }}
        onBlur={() => onDone(null)}
        onKeyDown={(e) => {
          if (navigate(e, items, index, setIndex, (t) => onDone(t.path), () => setIndex(-1))) return;
          if (e.key === "Enter") onDone(typed || null);
          if (e.key === "Escape") onDone(null);
        }}
      />
      <Suggestions items={items} index={index} onPick={(t) => onDone(t.path)} />
    </div>
  );
}

/** Tag chips on a font row: click filters by the tag, × removes it from the family. */
export function TagChips({ tags, query, onFilter, onRemove }: { tags: string[]; query: string; onFilter: (t: string) => void; onRemove: (t: string) => void }) {
  return (
    <span className="tags">
      {tags.map((t) => (
        <span
          key={t}
          className={`tag ${queryHasTag(query, t) ? "on" : ""}`}
          title={`Click to search ${tagToken(t)}`}
          onClick={(e) => {
            e.stopPropagation();
            onFilter(t);
          }}
          onDoubleClick={(e) => e.stopPropagation()}
        >
          <TagPath path={t} />
          <button
            className="untag"
            title={`Remove ${tagToken(t)} from this family`}
            onClick={(e) => {
              e.stopPropagation();
              onRemove(t);
            }}
          >
            ×
          </button>
        </span>
      ))}
    </span>
  );
}
