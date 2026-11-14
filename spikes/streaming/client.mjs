// Reference for the proposed SSE dialect, not a production SDK.
// Return raw data text so document numbers are not coerced by JSON.parse.
export async function* subscribe(url, token, {
  fetchImpl = globalThis.fetch, signal, maxFrameBytes = 3 * 1024 * 1024,
} = {}) {
  const target = new URL(url, globalThis.location?.href);
  const loopback = ["127.0.0.1", "localhost", "[::1]"].includes(target.hostname);
  if (target.username || target.password || target.search || target.hash ||
      !(target.protocol === "https:" || (target.protocol === "http:" && loopback))) {
    throw new Error("invalid_stream_url");
  }
  if (typeof token !== "string" || !token || /[\r\n]/.test(token)) throw new Error("invalid_token");
  if (!Number.isSafeInteger(maxFrameBytes) || maxFrameBytes < 1 || maxFrameBytes > 3 * 1024 * 1024) {
    throw new Error("invalid_frame_limit");
  }
  if (signal?.aborted) throw new Error("aborted");
  let response;
  try {
    response = await fetchImpl(target.href, {
      headers: { Accept: "text/event-stream", Authorization: `Bearer ${token}` },
      credentials: "omit", mode: "same-origin", cache: "no-store", redirect: "error", signal,
    });
  } catch {
    throw new Error(signal?.aborted ? "aborted" : "connection_failed");
  }
  if (response.status !== 200 || !response.body ||
      response.headers.get("content-type")?.split(";")[0].trim() !== "text/event-stream") {
    await response.body?.cancel();
    throw new Error(response.status === 401 ? "unauthorized" : "invalid_stream_response");
  }
  const reader = response.body.getReader();
  const abort = () => { reader.cancel().catch(() => {}); };
  signal?.addEventListener("abort", abort, { once: true });
  let line = [], data = [], event = "message", frameBytes = 0;
  const decoder = new TextDecoder("utf-8", { fatal: true });
  try {
    while (true) {
      if (signal?.aborted) throw new Error("aborted");
      let chunk;
      try { chunk = await reader.read(); }
      catch { throw new Error(signal?.aborted ? "aborted" : "stream_interrupted"); }
      if (signal?.aborted) throw new Error("aborted");
      if (chunk.done) throw new Error("stream_interrupted");
      for (const byte of chunk.value) {
        if (++frameBytes > maxFrameBytes) throw new Error("frame_too_large");
        if (byte !== 10) { line.push(byte); continue; }
        if (line.at(-1) === 13) line.pop();
        let text;
        try { text = decoder.decode(Uint8Array.from(line)); }
        catch { throw new Error("invalid_utf8"); }
        line = [];
        // Endpoint emits LF; CRLF is accepted. Bare CR framing is not supported
        // by this reference, which is not a general EventSource polyfill.
        if (text.includes("\r")) throw new Error("invalid_framing");
        if (!text) {
          if (data.length) {
            if (!["ready", "change", "stream_end"].includes(event)) throw new Error("invalid_event");
            const record = { event, data: data.join("\n") };
            data = []; event = "message"; frameBytes = 0;
            yield record;
            if (record.event === "stream_end") return;
          } else { event = "message"; frameBytes = 0; }
          continue;
        }
        if (text.startsWith(":")) continue;
        const colon = text.indexOf(":");
        const field = colon < 0 ? text : text.slice(0, colon);
        let value = colon < 0 ? "" : text.slice(colon + 1);
        if (value.startsWith(" ")) value = value.slice(1);
        if (field === "id" || field === "retry") throw new Error("resume_not_supported");
        if (field === "event") event = value;
        if (field === "data") data.push(value);
      }
    }
  } finally {
    signal?.removeEventListener("abort", abort);
    await reader.cancel().catch(() => {});
    reader.releaseLock();
  }
}
