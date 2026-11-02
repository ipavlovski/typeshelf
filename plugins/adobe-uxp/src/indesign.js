// InDesign: collects fonts the document lists as missing and asks typeshelf to activate them.
const { app, FontStatus } = require("indesign");

function documentFonts() {
  if (app.documents.length === 0) return [];
  const fonts = app.activeDocument.fonts;
  const names = [];
  for (let i = 0; i < fonts.length; i++) {
    const font = fonts.item(i);
    if (font.status !== FontStatus.INSTALLED) names.push(font.postscriptName);
  }
  return names;
}

function onDocumentOpened(callback) {
  app.addEventListener("afterOpen", () => callback());
}

module.exports = { documentFonts, onDocumentOpened };
