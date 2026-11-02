import { invoke } from "@tauri-apps/api/core";

export type FaceId = number;

export interface Scope {
  kind: "all" | "active" | "inactive" | "pinned" | "favourites" | "collection" | "folder";
  id?: number;
}

export interface FamilyRow {
  id: number;
  name: string;
  faces: number;
  active: number;
  pinned: number;
  favourites: number;
  preview: FaceId;
  faceIds: FaceId[];
}

export interface SearchResult {
  families: FamilyRow[];
  faces: number;
  micros: number;
}

export interface Axis {
  tag: string;
  min: number;
  default: number;
  max: number;
}

export interface Face {
  id: FaceId;
  style: string;
  fullName: string;
  psName: string;
  weight: number;
  width: number;
  italic: boolean;
  format: string;
  glyphs: number;
  path: string;
  active: boolean;
  pinned: boolean;
  favourite: boolean;
  axes: Axis[];
  features: string[];
  scripts: string[];
  vendor: string;
  license: string;
  hash: string;
  index: number;
}

export interface Folder {
  id: number;
  path: string;
  root: boolean;
  faces: number;
}

export interface Collection {
  id: number;
  parentId: number | null;
  name: string;
  faces: number;
  active: number;
}

export interface Library {
  folders: Folder[];
  collections: Collection[];
  all: number;
  active: number;
  pinned: number;
  favourites: number;
  canActivate: boolean;
}

export interface ActivationResult {
  changed: number;
  failed: string[];
  shadowed: number;
  restartApps: string[];
}

export interface Settings {
  sample: string;
  size: number;
}

export const DEFAULT_SAMPLE = "The quick brown fox jumps over the lazy dog";
export const DEFAULT_SETTINGS: Settings = { sample: DEFAULT_SAMPLE, size: 42 };

export const api = {
  search: (query: string, scope: Scope) => invoke<SearchResult | null>("search", { query, scope }),
  familyFaces: (family: number) => invoke<Face[]>("family_faces", { family }),
  setActive: (faces: FaceId[], on: boolean, pin = false) => invoke<ActivationResult>("set_active", { faces, on, pin }),
  setPinned: (faces: FaceId[], pinned: boolean) => invoke<ActivationResult>("set_pinned", { faces, pinned }),
  setCollectionActive: (id: number, on: boolean) => invoke<ActivationResult>("set_collection_active", { id, on }),
  deactivateAll: () => invoke<void>("deactivate_all"),
  setFavourite: (faces: FaceId[], on: boolean) => invoke<void>("set_favourite", { faces, on }),
  library: () => invoke<Library>("library"),
  addFolder: (path: string, root: boolean) => invoke<void>("add_folder", { path, root }),
  removeFolder: (id: number) => invoke<void>("remove_folder", { id }),
  collectionCreate: (parent: number | null, name: string) => invoke<number>("collection_create", { parent, name }),
  collectionRename: (id: number, name: string) => invoke<void>("collection_rename", { id, name }),
  collectionDelete: (id: number) => invoke<void>("collection_delete", { id }),
  collectionAdd: (id: number, faces: FaceId[], add: boolean) => invoke<void>("collection_add", { id, faces, add }),
  getSettings: async (): Promise<Settings> => ({ ...DEFAULT_SETTINGS, ...(await invoke<Partial<Settings>>("get_settings")) }),
  setSettings: (settings: Settings) => invoke<void>("set_settings", { settings }),
  fontBytes: (face: FaceId) => invoke<ArrayBuffer>("font_bytes", { face }),
  faceCodepoints: (face: FaceId) => invoke<number[]>("face_codepoints", { face }),
  reveal: (face: FaceId) => invoke<void>("reveal", { face }),
};

// Custom protocols are served as http://<scheme>.localhost on Windows.
const THUMB_BASE = navigator.userAgent.includes("Windows") ? "http://thumb.localhost/" : "thumb://localhost/";

/** Preview image URL. `size` is the CSS font size; the image is rendered for the screen's pixel density. */
export function thumbUrl(face: FaceId, size: number, text: string, maxWidth?: number): string {
  const px = Math.round(size * window.devicePixelRatio);
  const params = new URLSearchParams();
  if (text !== DEFAULT_SAMPLE) {
    params.set("t", text);
    params.set("live", "1");
  }
  if (maxWidth) params.set("w", String(Math.round(maxWidth * window.devicePixelRatio)));
  const q = params.toString();
  return `${THUMB_BASE}${face}/${px}${q ? `?${q}` : ""}`;
}

/** Row image height in CSS pixels for a font size (matches the renderer's line height). */
export const lineHeight = (size: number) => Math.ceil(Math.round(size * window.devicePixelRatio) * 1.4) / window.devicePixelRatio;
