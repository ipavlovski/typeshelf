# typeshelf for Adobe apps (UXP)

Activates the fonts an open document needs, from your typeshelf library, when the
document opens or when you press the button in the panel.

- Photoshop 24+: fonts of all text layers.
- InDesign 18.5+: fonts the document reports as not installed.
- Illustrator: not yet; its UXP API cannot list a document's fonts.

The plugin talks to the typeshelf app on `ws://127.0.0.1:47215` (local only). The app
must be running; fonts are activated for the session.

## Build and load

```sh
node build.mjs
```

Load `dist/photoshop` or `dist/indesign` with Adobe's UXP Developer Tool
(Add Plugin → select manifest.json → Load). Status: written against the UXP APIs but
not yet tested inside Photoshop or InDesign.
