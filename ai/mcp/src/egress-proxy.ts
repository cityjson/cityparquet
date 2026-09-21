// What a sandboxed engine can reach over the network, decided outside DuckDB.
//
// The hosted query tool is public, and httpfs fetches any URL a query names.
// Cloud Run offers no host allowlist, and its metadata server — which hands
// out the service account's token — is reachable from every container. So a
// sandboxed engine's `http_proxy` is locked to this proxy, which admits
// HTTPS to the allowlisted hosts and nothing else. Every reader was checked
// to go through it: read_parquet, read_json, read_text, the cityjson and
// FlatCityBuf readers, and spatial's GDAL (ST_Read, /vsicurl/).

import { lookup } from "node:dns/promises";
import { createServer, type Server } from "node:http";
import { BlockList, connect, isIP } from "node:net";

/** Private, loopback, link-local, CGNAT and metadata ranges: never a data host. */
const INTERNAL = new BlockList();
for (const [network, prefix] of [
  ["0.0.0.0", 8],
  ["10.0.0.0", 8],
  ["100.64.0.0", 10],
  ["127.0.0.0", 8],
  ["169.254.0.0", 16],
  ["172.16.0.0", 12],
  ["192.168.0.0", 16],
] as const) {
  INTERNAL.addSubnet(network, prefix, "ipv4");
}
for (const [network, prefix] of [
  ["::", 128],
  ["::1", 128],
  ["fc00::", 7],
  ["fe80::", 10],
] as const) {
  INTERNAL.addSubnet(network, prefix, "ipv6");
}

export function isInternalAddress(address: string): boolean {
  // An IPv4-mapped IPv6 address is its IPv4 address. (Not a `::ffff:0:0/96`
  // subnet in the list: BlockList matches every plain IPv4 address against
  // that range, which would refuse the whole internet.)
  const mapped = /^::ffff:(\d+\.\d+\.\d+\.\d+)$/i.exec(address);
  if (mapped) return isInternalAddress(mapped[1]!);
  const family = isIP(address);
  if (family === 0) return false;
  return INTERNAL.check(address, family === 4 ? "ipv4" : "ipv6");
}

export interface EgressProxyOptions {
  /** Exact hostnames, compared case-insensitively. No wildcards, no IP literals. */
  readonly allowedHosts: readonly string[];
  readonly onRefused?: (target: string, reason: string) => void;
}

export interface EgressProxy {
  /** `127.0.0.1:<port>`, as DuckDB's `http_proxy` setting takes it. */
  readonly address: string;
  close(): Promise<void>;
}

function parseTarget(target: string): { host: string; port: number } | null {
  // `host:443`, or `[v6]:443` — the latter never matches an allowlisted name.
  const match = /^(\[[^\]]+\]|[^:]+):(\d+)$/.exec(target);
  if (!match) return null;
  return { host: match[1]!.toLowerCase(), port: Number(match[2]) };
}

export async function startEgressProxy(options: EgressProxyOptions): Promise<EgressProxy> {
  const allowed = new Set(options.allowedHosts.map((host) => host.toLowerCase()));
  const refuse = (target: string, reason: string) => options.onRefused?.(target, reason);

  const server: Server = createServer((req, res) => {
    // Plain HTTP is never forwarded: every data host is HTTPS, and the cloud
    // metadata server answers plain HTTP.
    refuse(req.url ?? "", "plain HTTP");
    res.writeHead(403, { "content-type": "text/plain" });
    res.end("egress refused: only HTTPS to the data hosts is allowed");
  });

  server.on("connect", (req, socket, head) => {
    const target = req.url ?? "";
    const parsed = parseTarget(target);
    const deny = (reason: string) => {
      refuse(target, reason);
      socket.end("HTTP/1.1 403 Forbidden\r\n\r\n");
    };
    if (!parsed) return deny("malformed target");
    if (parsed.port !== 443) return deny("port other than 443");
    if (!allowed.has(parsed.host)) return deny("host not on the allowlist");

    // Resolve here and connect to the address checked, so the name cannot be
    // re-resolved to an internal address between the check and the connect.
    lookup(parsed.host, { all: true })
      .then((addresses) => {
        if (addresses.length === 0 || addresses.some((a) => isInternalAddress(a.address))) {
          return deny("resolves to an internal address");
        }
        const upstream = connect(443, addresses[0]!.address, () => {
          socket.write("HTTP/1.1 200 Connection Established\r\n\r\n");
          if (head.length > 0) upstream.write(head);
          upstream.pipe(socket);
          socket.pipe(upstream);
        });
        upstream.on("error", () => socket.destroy());
        socket.on("error", () => upstream.destroy());
      })
      .catch(() => deny("does not resolve"));
  });

  await new Promise<void>((resolve, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => resolve());
  });
  const { port } = server.address() as { port: number };

  return {
    address: `127.0.0.1:${port}`,
    close: () =>
      new Promise<void>((resolve) => {
        server.closeAllConnections();
        server.close(() => resolve());
      }),
  };
}
