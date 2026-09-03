import { supabase } from "@/lib/supabase";

const API_BASE_URL = import.meta.env.VITE_API_BASE_URL ?? "/api";

export class ApiError extends Error {
  constructor(
    public status: number,
    message: string,
    public payload?: unknown,
  ) {
    super(message);
    this.name = "ApiError";
  }
}

async function authHeader(): Promise<HeadersInit> {
  const { data } = await supabase.auth.getSession();
  const token = data.session?.access_token;
  return token ? { Authorization: `Bearer ${token}` } : {};
}

async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const headers: HeadersInit = {
    Accept: "application/json",
    ...(await authHeader()),
    ...init.headers,
  };

  const res = await fetch(`${API_BASE_URL}${path}`, { ...init, headers });
  const text = await res.text();
  const payload = text ? safeParse(text) : undefined;

  if (!res.ok) {
    const message =
      (payload && typeof payload === "object" && "error" in payload
        ? String((payload as { error: unknown }).error)
        : null) ?? res.statusText;
    throw new ApiError(res.status, message, payload);
  }

  return payload as T;
}

function safeParse(text: string): unknown {
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

// ---------- typed endpoints ----------

/** A module table inside a dataset. `role` is "object" or "sidecar". */
export interface ModuleInfo {
  name: string;
  role: string;
  rows: number;
}

/** A dataset: a CityParquet package, one table per CityGML module. */
export interface DatasetInfo {
  name: string;
  modules: ModuleInfo[];
  crs: string | null;
}

/** One CityObject, as the server returns it: a flat JSON row. */
export type ObjectRow = Record<string, unknown>;

/** The server returns a bare array of names, not an envelope. */
export function listDatasets(): Promise<string[]> {
  return request<string[]>("/datasets");
}

export function describeDataset(ds: string): Promise<DatasetInfo> {
  return request<DatasetInfo>(`/datasets/${encodeURIComponent(ds)}`);
}

export function createDataset(ds: string, sourcePath: string): Promise<DatasetInfo> {
  return request<DatasetInfo>(`/datasets/${encodeURIComponent(ds)}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ source_path: sourcePath }),
  });
}

export function uploadDataset(ds: string, file: File): Promise<DatasetInfo> {
  const fd = new FormData();
  fd.append("file", file);
  return request<DatasetInfo>(`/datasets/${encodeURIComponent(ds)}/upload`, {
    method: "POST",
    body: fd,
  });
}

/** Drops the dataset and everything in it. The server answers 204. */
export function dropDataset(ds: string): Promise<void> {
  return request<void>(`/datasets/${encodeURIComponent(ds)}`, { method: "DELETE" });
}

export function ingestSource(ds: string, sourcePath: string): Promise<{ ingested: number }> {
  return request<{ ingested: number }>(`/datasets/${encodeURIComponent(ds)}/objects`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ source_path: sourcePath }),
  });
}

/** Also a bare array — there is no envelope and no total count. */
export function queryObjects(
  ds: string,
  module: string,
  params: { filter?: string; limit?: number; offset?: number } = {},
): Promise<ObjectRow[]> {
  const qs = new URLSearchParams();
  if (params.filter) qs.set("filter", params.filter);
  if (params.limit !== undefined) qs.set("limit", String(params.limit));
  if (params.offset !== undefined) qs.set("offset", String(params.offset));
  const query = qs.toString();

  return request<ObjectRow[]>(
    `/datasets/${encodeURIComponent(ds)}/modules/${encodeURIComponent(module)}/objects${
      query ? `?${query}` : ""
    }`,
  );
}

/** Deletes by id, cascading to the object's children; returns how many went. */
export function deleteObject(ds: string, id: string): Promise<{ deleted: number }> {
  return request<{ deleted: number }>(
    `/datasets/${encodeURIComponent(ds)}/objects/${encodeURIComponent(id)}`,
    { method: "DELETE" },
  );
}

/**
 * One structural problem `cityparquet_validate` found. `object_id` is `null`
 * when the finding is about the table itself rather than one row in it.
 */
export interface ValidationFinding {
  check_name: string;
  severity: string;
  table_name: string;
  object_id: string | null;
  message: string;
}

/** Runs every structural check; reports, does not repair. A bare array. */
export function validateDataset(ds: string): Promise<ValidationFinding[]> {
  return request<ValidationFinding[]>(`/datasets/${encodeURIComponent(ds)}/validate`, {
    method: "POST",
  });
}

/** Re-derives `feature_id`, the reciprocal hierarchy and bbox. The server answers 204. */
export function reconcileDataset(ds: string): Promise<void> {
  return request<void>(`/datasets/${encodeURIComponent(ds)}/reconcile`, { method: "POST" });
}

/** Reclaims unreferenced sidecar rows. */
export function vacuumDataset(ds: string): Promise<{ vacuumed: number }> {
  return request<{ vacuumed: number }>(`/datasets/${encodeURIComponent(ds)}/vacuum`, {
    method: "POST",
  });
}

/** Merges each object table's small Parquet files via DuckLake. */
export function compactDataset(
  ds: string,
): Promise<{ files_processed: number; files_created: number }> {
  return request<{ files_processed: number; files_created: number }>(
    `/datasets/${encodeURIComponent(ds)}/compact`,
    { method: "POST" },
  );
}
