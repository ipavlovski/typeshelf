import { useState, type ReactNode } from "react";
import { ask, open } from "@tauri-apps/plugin-dialog";
import { api, type ActivationResult, type Collection, type Library, type Scope } from "./api";
import { Dot } from "./FontList";

interface Props {
  library: Library | null;
  scope: Scope;
  onScope: (s: Scope) => void;
  onChanged: () => void;
  onActivation: (op: Promise<ActivationResult | void>) => Promise<void>;
  onError: (msg: string) => void;
  dropTarget: (collectionId: number) => void;
}

function Item({
  active,
  label,
  count,
  onClick,
  children,
  depth = 0,
}: {
  active: boolean;
  label: ReactNode;
  count?: number;
  onClick: () => void;
  children?: ReactNode;
  depth?: number;
}) {
  return (
    <div className={`nav-item ${active ? "active" : ""}`} style={{ paddingLeft: 12 + depth * 14 }} onClick={onClick}>
      {children}
      <span className="nav-label">{label}</span>
      {count != null && <span className="count">{count.toLocaleString()}</span>}
    </div>
  );
}

function InlineInput({ initial, onDone }: { initial: string; onDone: (v: string | null) => void }) {
  const [value, setValue] = useState(initial);
  return (
    <input
      className="inline-input"
      autoFocus
      value={value}
      onChange={(e) => setValue(e.target.value)}
      onClick={(e) => e.stopPropagation()}
      onBlur={() => onDone(value.trim() || null)}
      onKeyDown={(e) => {
        if (e.key === "Enter") onDone(value.trim() || null);
        if (e.key === "Escape") onDone(null);
      }}
    />
  );
}

export function Sidebar({ library, scope, onScope, onChanged, onActivation, onError, dropTarget }: Props) {
  const [collapsed, setCollapsed] = useState<Set<number>>(new Set());
  const [editing, setEditing] = useState<{ id: number | null; parent: number | null; name: string } | null>(null);
  const [menu, setMenu] = useState<{ x: number; y: number; c: Collection } | null>(null);
  const is = (kind: Scope["kind"], id?: number) => scope.kind === kind && scope.id === id;

  const addFolder = async () => {
    const path = await open({ directory: true, title: "Add a font folder" });
    if (typeof path === "string") api.addFolder(path, false).catch((e) => onError(String(e)));
  };

  const saveEdit = async (name: string | null) => {
    const e = editing;
    setEditing(null);
    if (!e || !name) return;
    try {
      if (e.id == null) await api.collectionCreate(e.parent, name);
      else await api.collectionRename(e.id, name);
      onChanged();
    } catch (err) {
      onError(String(err));
    }
  };

  const tree = (parent: number | null, depth: number): ReactNode[] =>
    (library?.collections ?? [])
      .filter((c) => c.parentId === parent)
      .flatMap((c) => {
        const children = library!.collections.some((x) => x.parentId === c.id);
        const open = !collapsed.has(c.id);
        const node =
          editing?.id === c.id ? (
            <div key={c.id} className="nav-item" style={{ paddingLeft: 12 + depth * 14 }}>
              <InlineInput initial={c.name} onDone={saveEdit} />
            </div>
          ) : (
            <div
              key={c.id}
              onContextMenu={(e) => {
                e.preventDefault();
                setMenu({ x: e.clientX, y: e.clientY, c });
              }}
              onDragOver={(e) => e.preventDefault()}
              onDrop={() => dropTarget(c.id)}
            >
              <Item active={is("collection", c.id)} label={c.name} count={c.faces} depth={depth} onClick={() => onScope({ kind: "collection", id: c.id })}>
                <span
                  className={`caret ${children ? "" : "hidden"}`}
                  onClick={(e) => {
                    e.stopPropagation();
                    const next = new Set(collapsed);
                    if (open) next.add(c.id);
                    else next.delete(c.id);
                    setCollapsed(next);
                  }}
                >
                  {open ? "▾" : "▸"}
                </span>
                <Dot
                  active={c.active}
                  total={c.faces}
                  pinned={false}
                  disabled={!library?.canActivate || c.faces === 0}
                  onToggle={() => onActivation(api.setCollectionActive(c.id, c.active < c.faces))}
                />
              </Item>
            </div>
          );
        const newChild =
          editing && editing.id == null && editing.parent === c.id ? (
            <div key={`new${c.id}`} className="nav-item" style={{ paddingLeft: 12 + (depth + 1) * 14 }}>
              <InlineInput initial="" onDone={saveEdit} />
            </div>
          ) : null;
        return [node, ...(open ? tree(c.id, depth + 1) : []), newChild];
      });

  return (
    <aside className="sidebar" onClick={() => setMenu(null)}>
      <div className="brand">typeshelf</div>
      <section>
        <h3>Library</h3>
        <Item active={is("all")} label="All fonts" count={library?.all} onClick={() => onScope({ kind: "all" })} />
        <Item active={is("active")} label="Active" count={library?.active} onClick={() => onScope({ kind: "active" })} />
        <Item active={is("inactive")} label="Inactive" count={library ? library.all - library.active : undefined} onClick={() => onScope({ kind: "inactive" })} />
        <Item active={is("pinned")} label="Pinned" count={library?.pinned} onClick={() => onScope({ kind: "pinned" })} />
        <Item active={is("favourites")} label="Favourites" count={library?.favourites} onClick={() => onScope({ kind: "favourites" })} />
      </section>
      <section>
        <h3>
          Collections
          <button className="add" title="New collection" onClick={() => setEditing({ id: null, parent: null, name: "" })}>
            +
          </button>
        </h3>
        {tree(null, 0)}
        {editing && editing.id == null && editing.parent == null && (
          <div className="nav-item">
            <InlineInput initial="" onDone={saveEdit} />
          </div>
        )}
      </section>
      <section>
        <h3>
          Folders
          <button className="add" title="Add folder" onClick={addFolder}>
            +
          </button>
        </h3>
        {library?.folders.map((f) => (
          <div
            key={f.id}
            onContextMenu={async (e) => {
              e.preventDefault();
              if (await ask(`Stop watching ${f.path}? The files stay on disk.`, { title: "Remove folder", kind: "warning" })) {
                api.removeFolder(f.id).then(onChanged);
              }
            }}
          >
            <Item active={is("folder", f.id)} label={f.path.split(/[\\/]/).pop() || f.path} count={f.faces} onClick={() => onScope({ kind: "folder", id: f.id })} />
          </div>
        ))}
        {library && library.folders.length === 0 && (
          <button className="primary wide" onClick={addFolder}>
            Add a font folder
          </button>
        )}
      </section>
      {menu && (
        <div className="menu" style={{ left: menu.x, top: menu.y }}>
          <button onClick={() => setEditing({ id: null, parent: menu.c.id, name: "" })}>New sub-collection</button>
          <button onClick={() => setEditing({ id: menu.c.id, parent: menu.c.parentId, name: menu.c.name })}>Rename</button>
          <button onClick={() => onActivation(api.setCollectionActive(menu.c.id, true))}>Activate all</button>
          <button onClick={() => onActivation(api.setCollectionActive(menu.c.id, false))}>Deactivate all</button>
          <button
            className="danger"
            onClick={async () => {
              const c = menu.c;
              if (await ask(`Delete "${c.name}" and its sub-collections? Fonts are not deleted.`, { title: "Delete collection", kind: "warning" })) {
                await api.collectionDelete(c.id);
                if (is("collection", c.id)) onScope({ kind: "all" });
                onChanged();
              }
            }}
          >
            Delete
          </button>
        </div>
      )}
    </aside>
  );
}
