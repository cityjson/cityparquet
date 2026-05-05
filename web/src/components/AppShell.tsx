import { useQuery } from "@tanstack/react-query";
import { Database, LogOut, Plus, Upload as UploadIcon } from "lucide-react";
import { NavLink, Outlet, useLocation, useNavigate } from "react-router-dom";

import logoWordmark from "@/assets/logo-wordmark.svg";
import { useAuth } from "@/auth/AuthContext";
import { Eyebrow } from "@/components/Eyebrow";
import { StatusDot } from "@/components/StatusDot";
import { Button } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { listTables, type TableInfo } from "@/lib/api";
import { cn } from "@/lib/utils";

const METADATA_TABLE = "cityjson_metadata";

interface Dataset {
  base: string;
  lods: string[];
}

function groupByBase(tables: TableInfo[]): Dataset[] {
  const map = new Map<string, Dataset>();
  for (const t of tables) {
    if (!t.base || !t.lod || t.name === METADATA_TABLE) continue;
    const existing = map.get(t.base);
    if (existing) {
      existing.lods.push(t.lod);
    } else {
      map.set(t.base, { base: t.base, lods: [t.lod] });
    }
  }
  for (const ds of map.values()) ds.lods.sort();
  return Array.from(map.values()).sort((a, b) => a.base.localeCompare(b.base));
}

export default function AppShell() {
  const { session, signOut } = useAuth();
  const navigate = useNavigate();
  const location = useLocation();

  const { data, isLoading } = useQuery({
    queryKey: ["tables"],
    queryFn: listTables,
  });

  const datasets = data ? groupByBase(data.tables) : [];

  return (
    <div className="min-h-screen flex flex-col bg-paper-50">
      {/* Top bar */}
      <header className="h-14 shrink-0 flex items-center gap-4 border-b border-paper-200 bg-white px-5">
        <img src={logoWordmark} alt="CityLake" className="h-7" />
        <div className="h-6 w-px bg-paper-200" />
        <span className="font-mono text-[12px] text-ink-700 px-2 py-1 rounded-sm bg-paper-100 border border-paper-300">
          local · 127.0.0.1:3000
        </span>
        <div className="flex-1" />
        <span className="hidden md:inline-flex items-center gap-2 font-mono text-[11px] text-ink-500">
          <StatusDot tone="ok" />
          duckdb 1.5.1 · cityjson 0.1.0 · ducklake
        </span>
      </header>

      <div className="flex flex-1 min-h-0">
        {/* Sidebar */}
        <aside className="w-60 shrink-0 border-r border-paper-200 bg-paper-100 flex flex-col">
          <div className="px-4 pt-3.5 pb-2 flex items-center justify-between">
            <Eyebrow>Tables · {datasets.length}</Eyebrow>
            <button
              type="button"
              onClick={() => navigate("/upload")}
              aria-label="Upload"
              className="text-lake-500 hover:text-lake-700 p-1 rounded-sm focus:outline-none focus-visible:ring-2 focus-visible:ring-lake-300"
            >
              <Plus className="h-3.5 w-3.5" />
            </button>
          </div>

          <nav className="flex-1 overflow-y-auto px-2 pb-3 space-y-0.5">
            {isLoading && (
              <div className="space-y-1 px-1">
                {Array.from({ length: 4 }).map((_, i) => (
                  <Skeleton key={i} className="h-10" />
                ))}
              </div>
            )}

            {!isLoading && datasets.length === 0 && (
              <p className="font-mono text-[11px] text-ink-500 px-2 py-3">
                No datasets yet.
              </p>
            )}

            {datasets.map((ds) => {
              const isActive = location.pathname.startsWith(
                `/datasets/${ds.base}`,
              );
              return (
                <NavLink
                  key={ds.base}
                  to={`/datasets/${ds.base}`}
                  className={({ isActive: navActive }) =>
                    cn(
                      "flex items-center gap-2 rounded-sm px-2.5 py-2 transition-colors duration-150 ease-cl",
                      navActive || isActive
                        ? "bg-white border border-paper-300 shadow-cl-1"
                        : "border border-transparent hover:bg-white/60",
                    )
                  }
                >
                  {({ isActive: navActive }) => (
                    <>
                      <Database
                        className={cn(
                          "h-3.5 w-3.5 shrink-0",
                          navActive ? "text-lake-500" : "text-ink-500",
                        )}
                      />
                      <div className="min-w-0 flex-1">
                        <div className="font-mono text-[12px] text-ink-900 truncate">
                          {ds.base}
                        </div>
                        <div className="font-mono text-[10px] text-ink-500">
                          {ds.lods.length} LOD
                          {ds.lods.length === 1 ? "" : "s"} ·{" "}
                          {ds.lods.join(", ")}
                        </div>
                      </div>
                    </>
                  )}
                </NavLink>
              );
            })}
          </nav>

          <div className="border-t border-paper-200 p-3 space-y-2">
            <NavLink
              to="/upload"
              className={({ isActive }) =>
                cn(
                  "flex items-center gap-2 rounded-sm px-2.5 py-1.5 font-sans text-[13px] transition-colors duration-150 ease-cl",
                  isActive
                    ? "bg-lake-50 text-lake-700"
                    : "text-ink-700 hover:bg-white/60",
                )
              }
            >
              <UploadIcon className="h-3.5 w-3.5" />
              Upload
            </NavLink>
            <div className="border-t border-paper-200 pt-2">
              <p className="font-mono text-[10px] text-ink-500 px-1 truncate">
                {session?.user.email ?? ""}
              </p>
              <Button
                variant="ghost"
                size="sm"
                className="w-full justify-start mt-1 px-1.5"
                onClick={async () => {
                  await signOut();
                  navigate("/login", { replace: true });
                }}
              >
                <LogOut className="h-3.5 w-3.5" />
                Sign out
              </Button>
            </div>
          </div>
        </aside>

        {/* Main content */}
        <main className="flex-1 min-w-0 overflow-y-auto">
          <div className="container max-w-6xl py-8">
            <Outlet />
          </div>
        </main>
      </div>
    </div>
  );
}
