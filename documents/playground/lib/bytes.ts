// Counting the bytes a query actually pulls over the network.
//
// This is the number that makes the CityParquet argument concrete: "that swept
// a million buildings and read 400 kB". Nothing in DuckDB reports it, and it
// cannot be measured from the main thread either — DuckDB-Wasm does its HTTP
// inside the worker, through XMLHttpRequest, where neither `fetch` interception
// nor the page's resource timings can see it.
//
// So the worker is started from a shim that patches `XMLHttpRequest` and then
// loads DuckDB's own worker script. This was verified to survive DuckDB's
// bootstrap: the instance comes up and reports `wasm_eh` exactly as it does
// unpatched.
//
// The shim answers on a `MessageChannel` of its own, never on the worker's
// default channel, and that separation is load-bearing. DuckDB owns both ends
// of the default channel and reads every message there as one of its own
// envelopes: the main-thread handler does `response.type.toString()` on
// anything it cannot associate with a pending request, so a reply carrying no
// `type` throws an uncaught `TypeError` out of DuckDB's `onMessage`, and a ping
// carrying no `type` makes the worker side manufacture a spurious error
// response. One handshake is unavoidable — the port has to be delivered somehow
// — and the shim stops that event before DuckDB's handler sees it, which it can
// because its listener is registered ahead of `importScripts`. Everything after
// the handshake stays on the port.
//
// If any of that stops working, `readByteStats` resolves to `null` and the UI
// hides the readout rather than showing a figure that might be wrong.

export interface ByteStats {
  readonly bytes: number;
  readonly requests: number;
}

const PORT = "__cityparquet_stats_port";
const PING = "__cityparquet_stats_ping";
const REPLY = "__cityparquet_stats";

/**
 * The shim source, injected ahead of DuckDB's worker. It is a string because it
 * has to run inside the worker before DuckDB's script does, and it must not
 * capture anything from this module's scope.
 */
const SHIM = `
(function () {
  var bytes = 0, requests = 0;
  var send = XMLHttpRequest.prototype.send;
  XMLHttpRequest.prototype.send = function () {
    var xhr = this;
    xhr.addEventListener('load', function () {
      requests++;
      var n = 0;
      try {
        var r = xhr.response;
        if (r && typeof r.byteLength === 'number') n = r.byteLength;
        else if (typeof r === 'string') n = r.length;
      } catch (e) { /* response can throw for some responseTypes */ }
      // Content-Length is CORS-safelisted, so it is readable cross-origin even
      // when Content-Range and friends are not.
      if (!n) {
        try {
          var cl = xhr.getResponseHeader('Content-Length');
          if (cl) n = parseInt(cl, 10) || 0;
        } catch (e) { /* ignore */ }
      }
      bytes += n;
    });
    return send.apply(this, arguments);
  };
  self.addEventListener('message', function (e) {
    if (!e.data || !e.data.${PORT}) return;
    // The handshake is the only stats message ever to touch DuckDB's channel.
    // This listener runs before DuckDB's, which is installed only once
    // importScripts has evaluated, so stopping the event here keeps DuckDB from
    // reading a message it would treat as a malformed request.
    e.stopImmediatePropagation();
    var port = e.ports[0];
    if (!port) return;
    port.onmessage = function (m) {
      if (m.data && m.data.${PING}) {
        port.postMessage({ ${REPLY}: { bytes: bytes, requests: requests } });
      }
    };
  });
})();
`;

/** The stats port for each counting worker, kept out of the `Session` shape. */
const ports = new WeakMap<Worker, MessagePort>();

/**
 * Build a worker running DuckDB's worker script behind the counting shim.
 *
 * `workerUrl` must be absolute: the shim loads it with `importScripts`, which
 * resolves relative to the blob's own location rather than the page's.
 *
 * The port is handed over immediately, before `AsyncDuckDB` is constructed
 * around the worker, so the handshake never races DuckDB's own traffic.
 */
export function createCountingWorker(workerUrl: string): Worker {
  const absolute = new URL(workerUrl, location.href).href;
  const source = `${SHIM}\nimportScripts(${JSON.stringify(absolute)});`;
  const blob = new Blob([source], { type: "text/javascript" });
  const worker = new Worker(URL.createObjectURL(blob));

  const channel = new MessageChannel();
  worker.postMessage({ [PORT]: true }, [channel.port2]);
  // `addEventListener` on a port does not imply `start()`, unlike `onmessage`.
  channel.port1.start();
  ports.set(worker, channel.port1);

  return worker;
}

/**
 * Ask the worker for its byte totals. Resolves to `null` if the shim does not
 * answer, which is the signal to hide the readout rather than guess.
 */
export function readByteStats(worker: Worker, timeoutMs = 2_000): Promise<ByteStats | null> {
  const port = ports.get(worker);
  if (!port) return Promise.resolve(null);

  return new Promise((resolve) => {
    let settled = false;
    const finish = (value: ByteStats | null) => {
      if (settled) return;
      settled = true;
      port.removeEventListener("message", onMessage);
      clearTimeout(timer);
      resolve(value);
    };
    const onMessage = (event: MessageEvent) => {
      const stats = (event.data as Record<string, unknown> | null)?.[REPLY];
      if (stats) finish(stats as ByteStats);
    };
    const timer = setTimeout(() => finish(null), timeoutMs);
    port.addEventListener("message", onMessage);
    port.postMessage({ [PING]: true });
  });
}

/** Bytes as a human-readable string. Decimal units, as storage vendors bill them. */
export function formatBytes(bytes: number): string {
  if (bytes < 1_000) return `${bytes} B`;
  const units = ["kB", "MB", "GB", "TB"];
  let value = bytes / 1_000;
  let unit = 0;
  while (value >= 1_000 && unit < units.length - 1) {
    value /= 1_000;
    unit++;
  }
  // One decimal up to three digits, so "16.4 GB" keeps the precision the figure
  // is usually quoted with; beyond that the decimal is noise.
  return `${value < 100 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}
