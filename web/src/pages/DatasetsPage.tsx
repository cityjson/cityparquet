import { useQuery } from "@tanstack/react-query";
import { ChevronRight } from "lucide-react";
import { Link } from "react-router-dom";

import { Eyebrow } from "@/components/Eyebrow";
import { StatusDot } from "@/components/StatusDot";
import { Tag } from "@/components/Tag";
import {
  Card,
  CardContent,
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
    if (!t.base || !t.lod || t.name === METADATA_TABLE) continue;
    const existing = map.get(t.base);
    if (existing) {
      existing.lods.push(t.lod);
      existing.tables.push(t);
    } else {
      map.set(t.base, { base: t.base, lods: [t.lod], tables: [t] });
    }
  }
  for (const ds of map.values()) ds.lods.sort();
  return Array.from(map.values()).sort((a, b) => a.base.localeCompare(b.base));
}

export default function DatasetsPage() {
  const { data, isLoading, error } = useQuery({
    queryKey: ["tables"],
    queryFn: listTables,
  });

  const datasets = data ? groupByBase(data.tables) : [];

  return (
    <div className="space-y-8">
      <header className="space-y-1.5">
        <Eyebrow>Datasets · {datasets.length}</Eyebrow>
        <h1 className="text-[40px] font-semibold leading-tight tracking-tight text-ink-900 font-sans">
          Datasets
        </h1>
        <p className="text-[14px] text-ink-500 max-w-prose">
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
        <Card accent="error">
          <CardContent className="pt-5 font-mono text-[12px] text-roof-700">
            Failed to load tables: {(error as Error).message}
          </CardContent>
        </Card>
      )}

      {!isLoading && !error && datasets.length === 0 && (
        <Card>
          <CardContent className="pt-5 text-[14px] text-ink-500">
            No datasets yet.{" "}
            <Link to="/upload" className="text-lake-500 hover:text-lake-700 underline">
              Upload your first CityJSON file
            </Link>
            .
          </CardContent>
        </Card>
      )}

      {datasets.length > 0 && (
        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
          {datasets.map((ds) => (
            <Link key={ds.base} to={`/datasets/${ds.base}`} className="block">
              <Card className="transition-colors duration-150 ease-cl hover:border-paper-300/0 hover:bg-white hover:shadow-cl-2">
                <CardHeader className="pb-2">
                  <div className="flex items-center justify-between">
                    <Eyebrow className="flex items-center gap-1.5">
                      <span>Table</span>
                      <span className="text-paper-300">·</span>
                      <StatusDot tone="ok" />
                      <span className="text-moss-700">READY</span>
                    </Eyebrow>
                    <ChevronRight className="h-4 w-4 text-ink-400" />
                  </div>
                  <CardTitle className="font-mono text-[18px] mt-2">
                    {ds.base}
                  </CardTitle>
                </CardHeader>
                <CardContent className="space-y-3">
                  <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 font-mono text-[12px] text-ink-500">
                    <dt>lods</dt>
                    <dd className="text-ink-900">
                      {ds.lods.length} · {ds.lods.join(", ")}
                    </dd>
                    <dt>tables</dt>
                    <dd className="text-ink-900">{ds.tables.length}</dd>
                  </dl>
                  <div className="flex flex-wrap gap-1.5">
                    {ds.lods.map((lod) => (
                      <Tag key={lod} tone="info" square>
                        LOD {lod}
                      </Tag>
                    ))}
                  </div>
                </CardContent>
              </Card>
            </Link>
          ))}
        </div>
      )}
    </div>
  );
}
