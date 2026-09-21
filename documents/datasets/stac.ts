// Read a STAC catalogue of CityParquet packages: the catalogue, its child
// collections, and their items — every href resolved against the document it
// came from, as STAC requires. Nothing here knows any particular dataset; the
// datasets page renders whatever the catalogue lists.

/** A STAC link. */
interface Link {
  rel: string;
  href: string;
}

/** The subset of a STAC document this page reads. */
interface StacDoc {
  id: string;
  title?: string;
  description?: string;
  license?: string;
  providers?: { name: string; url?: string; roles?: string[] }[];
  links?: Link[];
  bbox?: number[];
  properties?: Record<string, unknown>;
  assets?: Record<string, { href: string; "file:size"?: number }>;
}

export type FetchJson = (url: string) => Promise<unknown>;

export interface ItemSummary {
  id: string;
  url: string;
  /** The directory the package's files are served from. */
  folder: string;
  title: string;
  objects: number | null;
  /** LoDs as CityGML writes them: `2`, `1.2`. */
  lods: string[];
  types: string[];
  /** `[west, south, east, north]` in WGS 84, or null for an unlocated package. */
  bbox: [number, number, number, number] | null;
  /** Total size of the package's files, each counted once. */
  bytes: number;
}

export interface CollectionSummary {
  id: string;
  url: string;
  title: string;
  description: string;
  license: string | null;
  providers: { name: string; url?: string }[];
  items: ItemSummary[];
  /** Item documents that could not be read; the rest still render. */
  failed: string[];
}

export interface CatalogSummary {
  title: string;
  description: string;
  collections: CollectionSummary[];
}

const hrefs = (doc: StacDoc, rel: string, base: string): string[] =>
  (doc.links ?? [])
    .filter((l) => l.rel === rel)
    .map((l) => new URL(l.href, base).href);

/** A bbox's horizontal extent: STAC allows 2D (4 numbers) and 3D (6). */
export function bbox2d(
  bbox: number[] | undefined,
): [number, number, number, number] | null {
  if (!bbox) return null;
  if (bbox.length === 6) return [bbox[0], bbox[1], bbox[3], bbox[4]];
  if (bbox.length === 4) return [bbox[0], bbox[1], bbox[2], bbox[3]];
  return null;
}

/** `0.0` → `0`; `1.2` stays. The city3d extension writes LoDs as strings. */
const lod = (value: string): string => value.replace(/\.0$/, "");

export function summariseItem(doc: StacDoc, url: string): ItemSummary {
  const props = doc.properties ?? {};
  const files = new Map<string, number>();
  for (const asset of Object.values(doc.assets ?? {})) {
    const target = new URL(asset.href, url).href;
    if (typeof asset["file:size"] === "number")
      files.set(target, asset["file:size"]);
  }
  const objects = props["city3d:city_objects"];
  return {
    id: doc.id,
    url,
    folder: new URL("./", url).href,
    title: typeof props.title === "string" ? props.title : doc.id,
    objects: typeof objects === "number" ? objects : null,
    lods: ((props["city3d:lods"] as string[] | undefined) ?? []).map(lod),
    types: (props["city3d:co_types"] as string[] | undefined) ?? [],
    bbox: bbox2d(doc.bbox),
    bytes: [...files.values()].reduce((a, b) => a + b, 0),
  };
}

async function loadCollection(
  url: string,
  fetchJson: FetchJson,
): Promise<CollectionSummary> {
  const doc = (await fetchJson(url)) as StacDoc;
  const itemUrls = hrefs(doc, "item", url);
  const settled = await Promise.allSettled(itemUrls.map((u) => fetchJson(u)));
  const items: ItemSummary[] = [];
  const failed: string[] = [];
  settled.forEach((result, i) => {
    if (result.status === "fulfilled")
      items.push(summariseItem(result.value as StacDoc, itemUrls[i]));
    else failed.push(itemUrls[i]);
  });
  return {
    id: doc.id,
    url,
    title: doc.title ?? doc.id,
    description: doc.description ?? "",
    license: doc.license ?? null,
    providers: (doc.providers ?? []).map(({ name, url: href }) => ({
      name,
      url: href,
    })),
    items,
    failed,
  };
}

/** Load a catalogue and every collection and item it links to. */
export async function loadCatalog(
  url: string,
  fetchJson: FetchJson,
): Promise<CatalogSummary> {
  const doc = (await fetchJson(url)) as StacDoc;
  const collections = await Promise.all(
    hrefs(doc, "child", url).map((u) => loadCollection(u, fetchJson)),
  );
  return {
    title: doc.title ?? doc.id,
    description: doc.description ?? "",
    collections,
  };
}

/** Decimal units, one decimal place: file sizes as a download would show them. */
export function formatBytes(bytes: number): string {
  const units = ["B", "kB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit += 1;
  }
  return unit === 0
    ? `${value} ${units[0]}`
    : `${value.toFixed(1)} ${units[unit]}`;
}
