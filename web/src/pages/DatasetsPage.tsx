import { useQuery } from "@tanstack/react-query";
import { ChevronRight, Database } from "lucide-react";
import { Link } from "react-router-dom";

import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { listTables, type TableInfo } from "@/lib/api";

const METADATA_TABLE = "cityjson_metadata";

interface Dataset {
  base: string;
  lods: string[];
  tables: TableInfo[];
}

function groupByBase(tables: TableInfo[]): Dataset[] {
  const map = new Map<string, Dataset>();
  for (const t of tables) {
    if (!t.base || !t.lod) continue;
    if (t.name === METADATA_TABLE) continue;
    const existing = map.get(t.base);
    if (existing) {
      existing.lods.push(t.lod);
      existing.tables.push(t);
    } else {
      map.set(t.base, { base: t.base, lods: [t.lod], tables: [t] });
    }
  }
  for (const ds of map.values()) {
    ds.lods.sort();
  }
  return Array.from(map.values()).sort((a, b) => a.base.localeCompare(b.base));
}

export default function DatasetsPage() {
  const { data, isLoading, error } = useQuery({
    queryKey: ["tables"],
    queryFn: listTables,
  });

  const datasets = data ? groupByBase(data.tables) : [];

  return (
    <div className="space-y-6">
      <header>
        <h1 className="text-2xl font-semibold tracking-tight">Datasets</h1>
        <p className="text-sm text-muted-foreground">
          CityJSON datasets ingested into CityLake. Each dataset is split into
          one table per Level of Detail.
        </p>
      </header>

      {isLoading && (
        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
          {Array.from({ length: 3 }).map((_, i) => (
            <Skeleton key={i} className="h-32" />
          ))}
        </div>
      )}

      {error && (
        <Card className="border-destructive/50">
          <CardContent className="pt-6 text-sm text-destructive">
            Failed to load tables: {(error as Error).message}
          </CardContent>
        </Card>
      )}

      {!isLoading && !error && datasets.length === 0 && (
        <Card>
          <CardContent className="pt-6 text-sm text-muted-foreground">
            No datasets yet.{" "}
            <Link to="/upload" className="text-foreground underline">
              Upload your first CityJSON file
            </Link>
            .
          </CardContent>
        </Card>
      )}

      {datasets.length > 0 && (
        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
          {datasets.map((ds) => (
            <Link key={ds.base} to={`/datasets/${ds.base}`}>
              <Card className="transition-colors hover:bg-muted/40">
                <CardHeader>
                  <div className="flex items-center justify-between">
                    <div className="flex items-center gap-2">
                      <Database className="h-4 w-4 text-muted-foreground" />
                      <CardTitle className="text-base">{ds.base}</CardTitle>
                    </div>
                    <ChevronRight className="h-4 w-4 text-muted-foreground" />
                  </div>
                  <CardDescription>
                    {ds.lods.length} LOD{ds.lods.length === 1 ? "" : "s"}:{" "}
                    {ds.lods.join(", ")}
                  </CardDescription>
                </CardHeader>
              </Card>
            </Link>
          ))}
        </div>
      )}
    </div>
  );
}
