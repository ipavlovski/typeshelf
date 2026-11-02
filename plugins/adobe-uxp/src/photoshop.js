// Photoshop: collects the fonts used by text layers and asks typeshelf to activate them.
const { app, action } = require("photoshop");

function textLayerFonts(layers, out) {
  for (const layer of layers) {
    if (layer.layers) textLayerFonts(layer.layers, out);
    if (layer.kind === "text") {
      try {
        out.push(layer.textItem.characterStyle.font);
      } catch {
        // Layers with mixed styles may not expose a single font; skip them.
      }
    }
  }
  return out;
}

function documentFonts() {
  const doc = app.activeDocument;
  return doc ? textLayerFonts(doc.layers, []) : [];
}

function onDocumentOpened(callback) {
  action.addNotificationListener(["open"], () => callback());
}

module.exports = { documentFonts, onDocumentOpened };
