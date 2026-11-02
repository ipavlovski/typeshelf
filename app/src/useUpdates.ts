import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { api, type AppInfo, type UpdateStatus } from "./api";

/** The app's version and the updater's status (src-tauri/src/updater.rs), kept live. */
export function useUpdates() {
  const [app, setApp] = useState<AppInfo | null>(null);
  const [status, setStatus] = useState<UpdateStatus>({ state: "none" });
  useEffect(() => {
    api.appInfo().then(setApp);
    api.updateStatus().then(setStatus);
    const unlisten = listen<UpdateStatus>("update-status", (e) => setStatus(e.payload));
    return () => void unlisten.then((f) => f());
  }, []);
  return { app, status };
}
