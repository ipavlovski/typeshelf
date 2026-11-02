// Talks to the typeshelf app over its local WebSocket endpoint (see app/src-tauri/src/automation.rs).

const ENDPOINT = "ws://127.0.0.1:47215";

/** Sends one request and resolves with the reply, or rejects if typeshelf is not running. */
function request(message, timeoutMs = 8000) {
  return new Promise((resolve, reject) => {
    let ws;
    try {
      ws = new WebSocket(ENDPOINT);
    } catch (e) {
      reject(e);
      return;
    }
    const timer = setTimeout(() => {
      ws.close();
      reject(new Error("typeshelf did not answer"));
    }, timeoutMs);
    ws.onopen = () => ws.send(JSON.stringify(message));
    ws.onmessage = (event) => {
      clearTimeout(timer);
      ws.close();
      resolve(JSON.parse(event.data));
    };
    ws.onerror = () => {
      clearTimeout(timer);
      reject(new Error("typeshelf is not running"));
    };
  });
}

/** Asks typeshelf to activate fonts by PostScript name. */
async function activateFonts(postscriptNames) {
  const unique = [...new Set(postscriptNames.filter(Boolean))];
  if (unique.length === 0) return { activated: 0, alreadyActive: 0, missing: [] };
  const reply = await request({ type: "activate", postscriptNames: unique });
  if (reply.type === "error") throw new Error(reply.message);
  return reply;
}

module.exports = { request, activateFonts };
