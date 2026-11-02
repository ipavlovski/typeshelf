// Assembles one UXP plugin per host into dist/<host>, since a manifest targets one app.
// Load dist/photoshop or dist/indesign with the UXP Developer Tool.
import { cpSync, mkdirSync, rmSync, writeFileSync } from "node:fs";

const hosts = {
  photoshop: { app: "PS", minVersion: "24.0.0", module: "photoshop.js" },
  indesign: { app: "ID", minVersion: "18.5.0", module: "indesign.js" },
};

rmSync("dist", { recursive: true, force: true });
for (const [name, h] of Object.entries(hosts)) {
  const out = `dist/${name}`;
  mkdirSync(out, { recursive: true });
  for (const f of ["index.html", "main.js", "client.js"]) cpSync(`src/${f}`, `${out}/${f}`);
  cpSync(`src/${h.module}`, `${out}/host.js`);
  const manifest = {
    manifestVersion: 5,
    id: `app.typeshelf.autoactivate.${name}`,
    name: "typeshelf",
    version: "0.1.0",
    main: "index.html",
    host: { app: h.app, minVersion: h.minVersion },
    entrypoints: [
      {
        type: "panel",
        id: "typeshelf",
        label: { default: "typeshelf" },
        minimumSize: { width: 220, height: 120 },
        preferredDockedSize: { width: 260, height: 160 },
      },
    ],
    requiredPermissions: { network: { domains: ["ws://127.0.0.1:47215"] } },
  };
  writeFileSync(`${out}/manifest.json`, JSON.stringify(manifest, null, 2) + "\n");
}
console.log("built", Object.keys(hosts).map((h) => `dist/${h}`).join(", "));
