// Counting the bytes a query actually pulls over the network.
//
// This is the number that makes the CityParquet argument concrete: "that swept
// a million buildings and read 400 kB". Nothing in DuckDB reports it, and it
// cannot be measured from the main thread either — DuckDB-Wasm does its HTTP
// inside the worker, through XMLHttpRequest, where neither `fetch` interception
// nor the page's resource timings can see it.
//
// So the worker is started from a shim that patches `XMLHttpRequest` and then
// loads DuckDB's own worker script. The shim answers a ping with its running
// totals. This was verified to survive DuckDB's bootstrap: the instance comes up
// and reports `wasm_eh` exactly as it does unpatched.
//
// If any of that stops working, `supported` goes false and the UI hides the
// readout rather than showing a figure that might be wrong.

export interface ByteStats {
  readonly bytes: number;
  readonly requests: number;
}

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
    if (e.data && e.data.${PING}) {
      self.postMessage({ ${REPLY}: { bytes: bytes, requests: requests } });
    }
  });
})();
`;

/**
 * Build a worker running DuckDB's worker script behind the counting shim.
 *
 * `workerUrl` must be absolute: the shim loads it with `importScripts`, which
 * resolves relative to the blob's own location rather than the page's.
 */
export function createCountingWorker(workerUrl: string): Worker {
  const absolute = new URL(workerUrl, location.href).href;
  const source = `${SHIM}\nimportScripts(${JSON.stringify(absolute)});`;
  const blob = new Blob([source], { type: "text/javascript" });
  const worker = new Worker(URL.createObjectURL(blob));
  return worker;
}

/**
 * Ask the worker for its byte totals. Resolves to `null` if the shim does not
 * answer, which is the signal to hide the readout rather than guess.
 */
export function readByteStats(worker: Worker, timeoutMs = 2_000): Promise<ByteStats | null> {
  return new Promise((resolve) => {
    let settled = false;
    const finish = (value: ByteStats | null) => {
      if (settled) return;
      settled = true;
      worker.removeEventListener("message", onMessage);
      clearTimeout(timer);
      resolve(value);
    };
    const onMessage = (event: MessageEvent) => {
      const stats = (event.data as Record<string, unknown> | null)?.[REPLY];
      if (stats) finish(stats as ByteStats);
    };
    const timer = setTimeout(() => finish(null), timeoutMs);
    worker.addEventListener("message", onMessage);
    worker.postMessage({ [PING]: true });
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
