import { useQuery } from "@tanstack/react-query";
import { ArrowLeft, Layers3 } from "lucide-react";
import { Link, useParams } from "react-router-dom";

import { Eyebrow } from "@/components/Eyebrow";
import { StatusDot } from "@/components/StatusDot";
import { Tag } from "@/components/Tag";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { listTables, queryObjects } from "@/lib/api";

const METADATA_TABLE = "cityjson_metadata";

interface MetadataRow {
  dataset?: string;
  source_path?: string;
  version?: string;
  identifier?: string;
  city_objects_count?: number;
  reference_system?: { code?: string; authority?: string };
  geographical_extent?: {
    min_x?: number; min_y?: number; min_z?: number;
    max_x?: number; max_y?: number; max_z?: number;
  };
  [k: string]: unknown;
}

export default function DatasetDetailPage() {
  const { base = "" } = useParams();

  const tablesQuery = useQuery({
    queryKey: ["tables"],
    queryFn: listTables,
  });

  const metadataQuery = useQuery({
    queryKey: ["metadata", base],
    queryFn: () =>
      queryObjects(METADATA_TABLE, {
        filter: `dataset = '${base.replace(/'/g, "''")}'`,
        limit: 1,
      }),
    enabled: !!base,
  });

  const lodTables =
    tablesQuery.data?.tables
      .filter((t) => t.base === base && t.lod)
      .sort((a, b) => (a.lod ?? "").localeCompare(b.lod ?? "")) ?? [];

  const metadata = metadataQuery.data?.objects?.[0] as MetadataRow | undefined;
  const crs =
    metadata?.reference_system?.code &&
    `${metadata.reference_system.authority ?? ""}:${metadata.reference_system.code}`;

  return (
    <div className="space-y-8">
      <Button asChild variant="ghost" size="sm" className="-ml-2">
        <Link to="/datasets">
          <ArrowLeft className="h-3.5 w-3.5" /> Back to datasets
        </Link>
      </Button>

      <header className="space-y-2">
        <div className="flex items-center gap-3 flex-wrap">
          <Eyebrow>Table</Eyebrow>
          <Tag tone="ok">
            <StatusDot tone="ok" />
            READY
          </Tag>
          {crs && (
            <Tag tone="info" square>
              {crs}
            </Tag>
          )}
          {typeof metadata?.city_objects_count === "number" && (
            <span className="font-mono text-[12px] text-ink-500">
              {metadata.city_objects_count.toLocaleString()} objects
            </span>
          )}
        </div>
        <h1 className="font-mono text-[40px] font-semibold leading-tight tracking-tight text-ink-900">
          {base}
        </h1>
        <p className="text-[14px] text-ink-500">
          {lodTables.length} LOD table{lodTables.length === 1 ? "" : "s"}.
        </p>
      </header>

      <div className="grid gap-4 md:grid-cols-2">
        <Card>
          <CardHeader>
            <Eyebrow>CityJSON metadata</Eyebrow>
          </CardHeader>
          <CardContent>
            {metadataQuery.isLoading && <Skeleton className="h-24" />}
            {metadataQuery.error && (
              <p className="font-mono text-[12px] text-roof-700">
                {(metadataQuery.error as Error).message}
              </p>
            )}
            {!metadataQuery.isLoading && !metadata && (
              <p className="text-[13px] text-ink-500">
                No metadata row for this dataset.
              </p>
            )}
            {metadata && (
              <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1.5 font-mono text-[12px]">
                {metadata.version && (
                  <Field label="version" value={metadata.version} />
                )}
                {metadata.identifier && (
                  <Field label="identifier" value={metadata.identifier} />
                )}
                {metadata.source_path && (
                  <Field label="source" value={metadata.source_path} />
                )}
                {crs && <Field label="crs" value={crs} />}
                {typeof metadata.city_objects_count === "number" && (
                  <Field
                    label="objects"
                    value={metadata.city_objects_count.toLocaleString()}
                  />
                )}
              </dl>
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <Eyebrow>LOD tables</Eyebrow>
            <CardTitle className="text-[14px] mt-2 font-sans">
              Open an LOD to browse its CityObjects
            </CardTitle>
          </CardHeader>
          <CardContent>
            {tablesQuery.isLoading && <Skeleton className="h-24" />}
            {tablesQuery.error && (
              <p className="font-mono text-[12px] text-roof-700">
                {(tablesQuery.error as Error).message}
              </p>
            )}
            {!tablesQuery.isLoading && lodTables.length === 0 && (
              <p className="text-[13px] text-ink-500">
                No LOD tables for this dataset.
              </p>
            )}
            {lodTables.length > 0 && (
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>LOD</TableHead>
                    <TableHead>Table</TableHead>
                    <TableHead className="text-right" />
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {lodTables.map((t) => (
                    <TableRow key={t.name}>
                      <TableCell>
                        <span className="inline-flex items-center gap-1.5">
                          <Layers3 className="h-3.5 w-3.5 text-ink-500" />
                          {t.lod}
                        </span>
                      </TableCell>
                      <TableCell className="text-ink-500">
                        {t.name}
                      </TableCell>
                      <TableCell className="text-right">
                        <Button asChild size="sm" variant="secondary">
                          <Link to={`/tables/${t.name}`}>Open</Link>
                        </Button>
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            )}
          </CardContent>
        </Card>
      </div>
    </div>
  );
}

function Field({ label, value }: { label: string; value: string }) {
  return (
    <>
      <dt className="text-ink-500">{label}</dt>
      <dd className="break-all text-ink-900 font-medium">{value}</dd>
    </>
  );
}
