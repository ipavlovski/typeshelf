import type { ReactNode } from "react";

// Release notes as scripts/lib/notes.ts writes them (and as GitHub release bodies usually are): headings,
// nested "- " bullets and paragraphs, with `code`, **bold** and [links](url) inline.

interface Item {
  text: string;
  children: Item[];
}

type Block = { t: "h"; text: string } | { t: "p"; text: string } | { t: "ul"; items: Item[] };

export function parseNotes(md: string): Block[] {
  const blocks: Block[] = [];
  for (const line of md.replace(/\r\n/g, "\n").split("\n")) {
    const bullet = /^(\s*)[-*] (.*)$/.exec(line);
    const last = blocks.at(-1);
    if (/^#{1,6} /.test(line)) blocks.push({ t: "h", text: line.replace(/^#+ /, "") });
    else if (bullet) {
      const list = last?.t === "ul" ? last : (blocks.push({ t: "ul", items: [] }), blocks.at(-1) as Block & { t: "ul" });
      const item = { text: bullet[2]!, children: [] };
      const parent = bullet[1]!.length >= 2 ? list.items.at(-1) : undefined;
      (parent ? parent.children : list.items).push(item);
    } else if (line.trim()) {
      if (last?.t === "p") last.text = last.text ? `${last.text} ${line.trim()}` : line.trim();
      else blocks.push({ t: "p", text: line.trim() });
    } else if (last?.t === "p") blocks.push({ t: "p", text: "" }); // paragraph break
  }
  return blocks.filter((b) => b.t !== "p" || b.text);
}

function inline(text: string): ReactNode[] {
  return text.split(/(`[^`]+`|\*\*[^*]+\*\*|\[[^\]]+\]\([^)\s]+\))/).map((part, i) => {
    if (/^`.+`$/.test(part)) return <code key={i}>{part.slice(1, -1)}</code>;
    if (/^\*\*.+\*\*$/.test(part)) return <b key={i}>{part.slice(2, -2)}</b>;
    const link = /^\[([^\]]+)\]\(([^)\s]+)\)$/.exec(part);
    if (link && /^https?:\/\//.test(link[2]!))
      return (
        <a key={i} href={link[2]} target="_blank" rel="noreferrer">
          {link[1]}
        </a>
      );
    return part;
  });
}

function List({ items }: { items: Item[] }) {
  return (
    <ul>
      {items.map((it, i) => (
        <li key={i}>
          {inline(it.text)}
          {it.children.length > 0 && <List items={it.children} />}
        </li>
      ))}
    </ul>
  );
}

export function ReleaseNotes({ source }: { source: string }) {
  const blocks = parseNotes(source);
  if (!blocks.length) return <p className="muted">No release notes.</p>;
  return (
    <div className="notes">
      {blocks.map((b, i) => (b.t === "h" ? <h3 key={i}>{inline(b.text)}</h3> : b.t === "p" ? <p key={i}>{inline(b.text)}</p> : <List key={i} items={b.items} />))}
    </div>
  );
}
