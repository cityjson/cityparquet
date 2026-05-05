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
    ...(init.headers ?? {}),
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

export interface CreateTableResponse {
  message: string;
  base_name: string;
  tables: string[];
}

export function createTable(
  base: string,
  body: { source_path?: string; lod?: string; base_name?: string },
): Promise<CreateTableResponse> {
  return request<CreateTableResponse>(`/tables/${encodeURIComponent(base)}`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
}

export function uploadCreateTable(
  base: string,
  file: File,
  qs: { lod?: string; base_name?: string } = {},
): Promise<CreateTableResponse> {
  const params = new URLSearchParams();
  if (qs.lod) params.set("lod", qs.lod);
  if (qs.base_name) params.set("base_name", qs.base_name);
  const query = params.toString();

  const fd = new FormData();
  fd.append("file", file);

  return request<CreateTableResponse>(
    `/tables/${encodeURIComponent(base)}/upload${query ? `?${query}` : ""}`,
    { method: "POST", body: fd },
  );
}

export interface QueryResponse {
  table: string;
  count: number;
  objects: Array<Record<string, unknown>>;
}

export function queryObjects(
  table: string,
  params: { filter?: string; limit?: number; offset?: number } = {},
): Promise<QueryResponse> {
  const qs = new URLSearchParams();
  if (params.filter) qs.set("filter", params.filter);
  if (params.limit !== undefined) qs.set("limit", String(params.limit));
  if (params.offset !== undefined) qs.set("offset", String(params.offset));
  const query = qs.toString();

  return request<QueryResponse>(
    `/tables/${encodeURIComponent(table)}/objects${query ? `?${query}` : ""}`,
  );
}

export function deleteObject(table: string, id: string): Promise<{ message: string }> {
  return request<{ message: string }>(
    `/tables/${encodeURIComponent(table)}/objects/${encodeURIComponent(id)}`,
    { method: "DELETE" },
  );
}

export function updateObject(
  table: string,
  id: string,
  cityjson_data: string,
): Promise<{ message: string }> {
  return request<{ message: string }>(
    `/tables/${encodeURIComponent(table)}/objects/${encodeURIComponent(id)}`,
    {
      method: "PUT",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ cityjson_data }),
    },
  );
}
