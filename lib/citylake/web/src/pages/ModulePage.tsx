import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowLeft, Table2, Trash2 } from "lucide-react";
import { useState } from "react";
import { Link, useParams } from "react-router-dom";

import { Eyebrow } from "@/components/Eyebrow";
import { StatusDot } from "@/components/StatusDot";
import { Tag } from "@/components/Tag";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { ApiError, deleteObject, queryObjects, type ObjectRow } from "@/lib/api";

const PAGE_SIZE = 25;

export default function ModulePage() {
  const { ds = "", module = "" } = useParams();
  const qc = useQueryClient();

  const [filter, setFilter] = useState("");
  const [filterDraft, setFilterDraft] = useState("");
  const [page, setPage] = useState(0);

  const [deleting, setDeleting] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [lastDeleted, setLastDeleted] = useState<number | null>(null);

  const query = useQuery<ObjectRow[]>({
    queryKey: ["objects", ds, module, filter, page],
    queryFn: () =>
      queryObjects(ds, module, {
        filter: filter || undefined,
        limit: PAGE_SIZE + 1,
        offset: page * PAGE_SIZE,
      }),
    enabled: !!ds && !!module,
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => deleteObject(ds, id),
    onSuccess: ({ deleted }) => {
      setDeleting(null);
      setActionError(null);
      setLastDeleted(deleted);
      qc.invalidateQueries({ queryKey: ["objects", ds, module] });
      qc.invalidateQueries({ queryKey: ["dataset", ds] });
    },
    onError: (err: unknown) => {
      setActionError(err instanceof Error ? err.message : String(err));
    },
  });

  const fetched = query.data ?? [];
  const hasNextPage = fetched.length > PAGE_SIZE;
  const objects = fetched.slice(0, PAGE_SIZE);
  const columns = inferColumns(objects);

  return (
    <div className="space-y-8">
      <Button asChild variant="ghost" size="sm" className="-ml-2">
        <Link to={`/datasets/${ds}`}>
          <ArrowLeft className="h-3.5 w-3.5" /> Back to {ds}
        </Link>
      </Button>

      <header className="space-y-2">
        <div className="flex items-center gap-3 flex-wrap">
          <Eyebrow>Module</Eyebrow>
          <Tag tone="ok">
            <StatusDot tone="ok" />
            READY
          </Tag>
        </div>
        <h1 className="font-mono text-[32px] font-semibold leading-tight tracking-tight text-ink-900">
          {module}
        </h1>
        <p className="text-[14px] text-ink-500">Browse and delete CityObjects in this module.</p>
      </header>

      {/* Filter */}
      <Card>
        <CardHeader className="pb-3">
          <Eyebrow>Filter</Eyebrow>
        </CardHeader>
        <CardContent>
          <form
            className="flex gap-2"
            onSubmit={(e) => {
              e.preventDefault();
              setFilter(filterDraft);
              setPage(0);
            }}
          >
            <Input
              mono
              value={filterDraft}
              onChange={(e) => setFilterDraft(e.target.value)}
              placeholder="object_type = 'Building'"
            />
            <Button type="submit">Apply</Button>
            {filter && (
              <Button
                type="button"
                variant="secondary"
                onClick={() => {
                  setFilter("");
                  setFilterDraft("");
                  setPage(0);
                }}
              >
                Clear
              </Button>
            )}
          </form>
          <p className="mt-2 font-mono text-[11px] text-ink-500">
            Optional SQL <code className="cl-code">WHERE</code> clause. Leave blank to show every
            row.
          </p>
        </CardContent>
      </Card>

      {actionError && (
        <Card accent="error">
          <CardContent className="pt-5 font-mono text-[12px] text-roof-700">
            {actionError}
          </CardContent>
        </Card>
      )}

      {lastDeleted !== null && !actionError && (
        <Card accent="ok">
          <CardContent className="pt-5 font-mono text-[12px] text-moss-700">
            Deleted {lastDeleted} object{lastDeleted === 1 ? "" : "s"} (including children).
          </CardContent>
        </Card>
      )}

      {/* Result grid */}
      <Card className="overflow-hidden">
        <div className="flex items-center gap-2.5 border-b border-paper-200 bg-paper-50 px-3 py-1.5">
          <Table2 className="h-3.5 w-3.5 text-ink-500" />
          <Eyebrow>Result</Eyebrow>
          <Tag tone="ok">
            <StatusDot tone="ok" />
            {objects.length} rows
          </Tag>
          <span className="font-mono text-[11px] text-ink-500">page {page + 1}</span>
        </div>
        <CardContent className="p-0">
          {query.isLoading && (
            <div className="p-5">
              <Skeleton className="h-48" />
            </div>
          )}
          {query.error && (
            <p className="px-5 py-4 font-mono text-[12px] text-roof-700">
              {(query.error as ApiError).message}
            </p>
          )}
          {query.data && objects.length === 0 && (
            <p className="px-5 py-6 text-[13px] text-ink-500">No rows.</p>
          )}
          {objects.length > 0 && (
            <Table>
              <TableHeader>
                <TableRow>
                  {columns.map((c) => (
                    <TableHead key={c}>{c}</TableHead>
                  ))}
                  <TableHead className="text-right">Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {objects.map((row, i) => (
                  <TableRow key={String(row.id ?? i)}>
                    {columns.map((c) => (
                      <TableCell key={c}>{formatCell(row[c])}</TableCell>
                    ))}
                    <TableCell className="text-right">
                      <div className="flex justify-end gap-1">
                        <Button
                          size="icon"
                          variant="ghost"
                          onClick={() => {
                            setDeleting(String(row.id ?? ""));
                            setActionError(null);
                          }}
                          aria-label={`Delete ${row.id}`}
                        >
                          <Trash2 className="h-3.5 w-3.5" />
                        </Button>
                      </div>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>

      <div className="flex items-center justify-between">
        <p className="font-mono text-[11px] text-ink-500">
          Page {page + 1}
          {query.data ? ` · ${objects.length} on this page` : ""}
        </p>
        <div className="flex gap-2">
          <Button
            variant="secondary"
            size="sm"
            onClick={() => setPage((p) => Math.max(0, p - 1))}
            disabled={page === 0}
          >
            Previous
          </Button>
          <Button
            variant="secondary"
            size="sm"
            onClick={() => setPage((p) => p + 1)}
            disabled={!hasNextPage}
          >
            Next
          </Button>
        </div>
      </div>

      {/* Delete confirm */}
      <AlertDialog
        open={deleting !== null}
        onOpenChange={(open) => {
          if (!open) setDeleting(null);
        }}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete object?</AlertDialogTitle>
            <AlertDialogDescription>
              This permanently removes <code className="cl-code">{deleting}</code> from{" "}
              <code className="cl-code">{module}</code>, along with every object beneath it in the
              hierarchy. This action cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={(e) => {
                e.preventDefault();
                if (deleting) deleteMutation.mutate(deleting);
              }}
              disabled={deleteMutation.isPending}
            >
              {deleteMutation.isPending ? "Deleting…" : "Delete"}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

function inferColumns(rows: ObjectRow[]): string[] {
  const seen = new Set<string>();
  const ordered: string[] = [];
  const preferred = ["id", "feature_id", "object_type"];

  for (const row of rows) {
    for (const key of Object.keys(row)) seen.add(key);
  }
  for (const p of preferred) {
    if (seen.has(p)) ordered.push(p);
  }
  for (const k of seen) {
    if (!preferred.includes(k) && ordered.length < 6) {
      ordered.push(k);
    }
  }
  return ordered;
}

function formatCell(value: unknown): string {
  if (value === null || value === undefined) return "—";
  if (typeof value === "object") return JSON.stringify(value).slice(0, 80);
  return String(value).slice(0, 80);
}
