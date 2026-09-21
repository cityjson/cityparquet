// A STAC catalogue of CityParquet packages, rendered from the catalogue itself:
// a map of every package's extent and, per collection, a table of its
// packages. Nothing here is specific to a dataset — publish a new collection
// and it appears.
import { useEffect, useRef, useState } from "react";
import L from "leaflet";
import "leaflet/dist/leaflet.css";

import {
  type CatalogSummary,
  type CollectionSummary,
  type ItemSummary,
  formatBytes,
  loadCatalog,
} from "../datasets/stac";
import "./StacCatalog.css";

// Leaflet needs `window`, so the island is never server-rendered.
export const client = "only";

/** Distinguishable on both themes; one per collection, in catalogue order. */
const PALETTE = [
  "#7253ed",
  "#e8590c",
  "#0ca678",
  "#1c7ed6",
  "#d6336c",
  "#f59f00",
];

const fetchJson = async (url: string): Promise<unknown> => {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${response.status} ${url}`);
  return response.json();
};

interface Props {
  /** URL of the root `catalog.json`. */
  href: string;
}

export default function StacCatalog({ href }: Props) {
  const [catalog, setCatalog] = useState<CatalogSummary | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [selected, setSelected] = useState<string | null>(null);

  useEffect(() => {
    loadCatalog(href, fetchJson).then(setCatalog, (e: Error) =>
      setError(e.message),
    );
  }, [href]);

  if (error)
    return (
      <p className="cp-stac-error">Could not load the catalogue: {error}</p>
    );
  if (!catalog)
    return <p className="cp-stac-loading">Loading the catalogue…</p>;

  return (
    <div className="cp-stac">
      <ExtentMap catalog={catalog} selected={selected} onSelect={setSelected} />
      {catalog.collections.map((c, i) => (
        <CollectionTable
          key={c.id}
          collection={c}
          colour={PALETTE[i % PALETTE.length]}
          selected={selected}
          onSelect={setSelected}
        />
      ))}
      <p className="cp-stac-source">
        Read live from <a href={href}>{href}</a>.
      </p>
    </div>
  );
}

const key = (c: CollectionSummary, item: ItemSummary) => `${c.id}/${item.id}`;

function ExtentMap({
  catalog,
  selected,
  onSelect,
}: {
  catalog: CatalogSummary;
  selected: string | null;
  onSelect: (key: string) => void;
}) {
  const host = useRef<HTMLDivElement>(null);
  const map = useRef<L.Map | null>(null);
  const layers = useRef(new Map<string, L.Rectangle>());

  useEffect(() => {
    if (!host.current) return;
    const m = L.map(host.current, {
      worldCopyJump: true,
      scrollWheelZoom: false,
    });
    L.tileLayer("https://tile.openstreetmap.org/{z}/{x}/{y}.png", {
      maxZoom: 18,
      attribution:
        '&copy; <a href="https://www.openstreetmap.org/copyright">OpenStreetMap</a>',
    }).addTo(m);
    const all: L.LatLngBounds[] = [];
    catalog.collections.forEach((c, i) => {
      const colour = PALETTE[i % PALETTE.length];
      for (const item of c.items) {
        if (!item.bbox) continue;
        const [w, s, e, n] = item.bbox;
        const bounds = L.latLngBounds([s, w], [n, e]);
        const rect = L.rectangle(bounds, {
          color: colour,
          weight: 1.5,
          fillOpacity: 0.15,
        })
          .bindTooltip(`${item.title} — ${c.title}`)
          .on("click", () => onSelect(key(c, item)))
          .addTo(m);
        layers.current.set(key(c, item), rect);
        all.push(bounds);
      }
    });
    // A fresh accumulator: `extend` mutates, and each bounds is a rectangle's.
    if (all.length)
      m.fitBounds(
        all.reduce((a, b) => a.extend(b), L.latLngBounds([])),
        { padding: [20, 20] },
      );
    else m.setView([30, 0], 2);
    map.current = m;
    // The island mounts before the page settles its layout, and Leaflet sizes
    // its panes from the container it measured at construction.
    const resized = new ResizeObserver(() => m.invalidateSize());
    resized.observe(host.current);
    return () => {
      resized.disconnect();
      m.remove();
      layers.current.clear();
    };
  }, [catalog, onSelect]);

  useEffect(() => {
    for (const [k, rect] of layers.current) {
      rect.setStyle({
        weight: k === selected ? 3 : 1.5,
        fillOpacity: k === selected ? 0.4 : 0.15,
      });
    }
    const rect = selected ? layers.current.get(selected) : undefined;
    if (rect && map.current)
      map.current.fitBounds(rect.getBounds(), {
        padding: [40, 40],
        maxZoom: 12,
      });
  }, [selected]);

  return <div ref={host} className="cp-stac-map" />;
}

function CollectionTable({
  collection: c,
  colour,
  selected,
  onSelect,
}: {
  collection: CollectionSummary;
  colour: string;
  selected: string | null;
  onSelect: (key: string) => void;
}) {
  const objects = c.items.reduce((n, i) => n + (i.objects ?? 0), 0);
  const bytes = c.items.reduce((n, i) => n + i.bytes, 0);
  return (
    <section className="cp-stac-collection">
      <h3>
        <span className="cp-stac-swatch" style={{ background: colour }} />
        {c.title}
      </h3>
      {c.description && (
        <p className="cp-stac-description">{c.description.split("\n")[0]}</p>
      )}
      <p className="cp-stac-meta">
        {c.items.length} {c.items.length === 1 ? "package" : "packages"} ·{" "}
        {objects.toLocaleString("en-GB")} city objects · {formatBytes(bytes)}
        {c.license && <> · licence {c.license}</>}
        {c.providers.length > 0 && (
          <>
            {" "}
            ·{" "}
            {c.providers.map((p, i) => (
              <span key={p.name}>
                {i > 0 && ", "}
                {p.url ? <a href={p.url}>{p.name}</a> : p.name}
              </span>
            ))}
          </>
        )}
        {" · "}
        <a href={c.url}>collection.json</a>
      </p>
      <div className="cp-stac-scroll">
        <table>
          <thead>
            <tr>
              <th>Package</th>
              <th className="cp-num">City objects</th>
              <th>LoD</th>
              <th>City object types</th>
              <th>Extent (W, S, E, N)</th>
              <th className="cp-num">Size</th>
            </tr>
          </thead>
          <tbody>
            {c.items.map((item) => (
              <tr
                key={item.id}
                className={
                  selected === key(c, item) ? "cp-stac-selected" : undefined
                }
                onClick={() => onSelect(key(c, item))}
              >
                <td>
                  <a href={item.url} onClick={(e) => e.stopPropagation()}>
                    {item.title}
                  </a>
                </td>
                <td className="cp-num">
                  {item.objects?.toLocaleString("en-GB") ?? "—"}
                </td>
                <td>{item.lods.join(", ")}</td>
                <td className="cp-stac-types">{item.types.join(", ")}</td>
                <td className="cp-stac-bbox">
                  {item.bbox
                    ? item.bbox.map((v) => v.toFixed(3)).join(", ")
                    : "—"}
                </td>
                <td className="cp-num">{formatBytes(item.bytes)}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {c.failed.length > 0 && (
        <p className="cp-stac-error">
          {c.failed.length} package{" "}
          {c.failed.length === 1 ? "document" : "documents"} could not be read.
        </p>
      )}
    </section>
  );
}
