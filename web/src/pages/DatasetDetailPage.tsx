import { useQuery } from "@tanstack/react-query";
import { ArrowLeft, Layers3 } from "lucide-react";
import { Link, useParams } from "react-router-dom";

import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
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

  return (
    <div className="space-y-6">
      <Button asChild variant="ghost" size="sm" className="-ml-2">
        <Link to="/datasets">
          <ArrowLeft className="h-4 w-4" /> Back to datasets
        </Link>
      </Button>

      <header>
        <h1 className="text-2xl font-semibold tracking-tight">{base}</h1>
        <p className="text-sm text-muted-foreground">
          {lodTables.length} LOD table{lodTables.length === 1 ? "" : "s"}.
        </p>
      </header>

      <div className="grid gap-4 md:grid-cols-2">
        <Card>
          <CardHeader>
            <CardTitle className="text-base">Metadata</CardTitle>
            <CardDescription>
              From the shared <code>cityjson_metadata</code> table.
            </CardDescription>
          </CardHeader>
          <CardContent>
            {metadataQuery.isLoading && <Skeleton className="h-24" />}
            {metadataQuery.error && (
              <p className="text-sm text-destructive">
                {(metadataQuery.error as Error).message}
              </p>
            )}
            {!metadataQuery.isLoading && !metadata && (
              <p className="text-sm text-muted-foreground">
                No metadata row for this dataset.
              </p>
            )}
            {metadata && (
              <dl className="grid grid-cols-[120px_1fr] gap-x-4 gap-y-2 text-sm">
                {metadata.version && (
                  <Field label="Version" value={metadata.version} />
                )}
                {metadata.identifier && (
                  <Field label="Identifier" value={metadata.identifier} />
                )}
                {metadata.source_path && (
                  <Field label="Source" value={metadata.source_path} />
                )}
                {metadata.reference_system?.code && (
                  <Field
                    label="CRS"
                    value={`${metadata.reference_system.authority ?? ""}:${metadata.reference_system.code}`}
                  />
                )}
                {typeof metadata.city_objects_count === "number" && (
                  <Field
                    label="Object count"
                    value={metadata.city_objects_count.toLocaleString()}
                  />
                )}
              </dl>
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="text-base">LOD tables</CardTitle>
            <CardDescription>
              Click an LOD to browse its CityObjects.
            </CardDescription>
          </CardHeader>
          <CardContent>
            {tablesQuery.isLoading && <Skeleton className="h-24" />}
            {tablesQuery.error && (
              <p className="text-sm text-destructive">
                {(tablesQuery.error as Error).message}
              </p>
            )}
            {!tablesQuery.isLoading && lodTables.length === 0 && (
              <p className="text-sm text-muted-foreground">
                No LOD tables for this dataset.
              </p>
            )}
            {lodTables.length > 0 && (
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead>LOD</TableHead>
                    <TableHead>Table</TableHead>
                    <TableHead className="text-right"></TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {lodTables.map((t) => (
                    <TableRow key={t.name}>
                      <TableCell className="flex items-center gap-2 font-medium">
                        <Layers3 className="h-3.5 w-3.5 text-muted-foreground" />
                        {t.lod}
                      </TableCell>
                      <TableCell className="font-mono text-xs text-muted-foreground">
                        {t.name}
                      </TableCell>
                      <TableCell className="text-right">
                        <Button asChild size="sm" variant="outline">
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
      <dt className="text-muted-foreground">{label}</dt>
      <dd className="break-all font-mono text-xs">{value}</dd>
    </>
  );
}
