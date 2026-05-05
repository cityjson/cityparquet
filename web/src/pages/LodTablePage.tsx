import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowLeft, Pencil, Trash2 } from "lucide-react";
import { useState } from "react";
import { Link, useParams } from "react-router-dom";

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
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Textarea } from "@/components/ui/textarea";
import {
  ApiError,
  deleteObject,
  queryObjects,
  updateObject,
  type QueryResponse,
} from "@/lib/api";

const PAGE_SIZE = 25;

interface ObjectRow {
  id?: string;
  feature_id?: string;
  object_type?: string;
  [k: string]: unknown;
}

export default function LodTablePage() {
  const { tableName = "" } = useParams();
  const qc = useQueryClient();

  const [filter, setFilter] = useState("");
  const [filterDraft, setFilterDraft] = useState("");
  const [page, setPage] = useState(0);

  const [editing, setEditing] = useState<{ id: string; json: string } | null>(
    null,
  );
  const [editDraft, setEditDraft] = useState("");
  const [editError, setEditError] = useState<string | null>(null);

  const [deleting, setDeleting] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);

  const query = useQuery<QueryResponse>({
    queryKey: ["objects", tableName, filter, page],
    queryFn: () =>
      queryObjects(tableName, {
        filter: filter || undefined,
        limit: PAGE_SIZE,
        offset: page * PAGE_SIZE,
      }),
    enabled: !!tableName,
  });

  const updateMutation = useMutation({
    mutationFn: ({ id, json }: { id: string; json: string }) =>
      updateObject(tableName, id, json),
    onSuccess: () => {
      setEditing(null);
      setEditError(null);
      qc.invalidateQueries({ queryKey: ["objects", tableName] });
    },
    onError: (err: unknown) => {
      setEditError(err instanceof Error ? err.message : String(err));
    },
  });

  const deleteMutation = useMutation({
    mutationFn: (id: string) => deleteObject(tableName, id),
    onSuccess: () => {
      setDeleting(null);
      setActionError(null);
      qc.invalidateQueries({ queryKey: ["objects", tableName] });
    },
    onError: (err: unknown) => {
      setActionError(err instanceof Error ? err.message : String(err));
    },
  });

  const objects = (query.data?.objects ?? []) as ObjectRow[];
  const columns = inferColumns(objects);

  return (
    <div className="space-y-6">
      <Button asChild variant="ghost" size="sm" className="-ml-2">
        <Link to="/datasets">
          <ArrowLeft className="h-4 w-4" /> Back to datasets
        </Link>
      </Button>

      <header>
        <h1 className="text-2xl font-semibold tracking-tight font-mono">
          {tableName}
        </h1>
        <p className="text-sm text-muted-foreground">
          Browse, edit, and delete CityObjects in this LOD table.
        </p>
      </header>

      <Card>
        <CardHeader>
          <CardTitle className="text-base">Filter</CardTitle>
          <CardDescription>
            Optional SQL <code>WHERE</code> clause (e.g.{" "}
            <code>object_type = 'Building'</code>). Leave blank to show all rows.
          </CardDescription>
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
              value={filterDraft}
              onChange={(e) => setFilterDraft(e.target.value)}
              placeholder="object_type = 'Building'"
              className="font-mono"
            />
            <Button type="submit">Apply</Button>
            {filter && (
              <Button
                type="button"
                variant="outline"
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
        </CardContent>
      </Card>

      {actionError && (
        <Card className="border-destructive/50">
          <CardContent className="pt-6 text-sm text-destructive">
            {actionError}
          </CardContent>
        </Card>
      )}

      <Card>
        <CardContent className="pt-6">
          {query.isLoading && <Skeleton className="h-64" />}
          {query.error && (
            <p className="text-sm text-destructive">
              {(query.error as ApiError).message}
            </p>
          )}
          {query.data && objects.length === 0 && (
            <p className="text-sm text-muted-foreground">No rows.</p>
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
                      <TableCell key={c} className="font-mono text-xs">
                        {formatCell(row[c])}
                      </TableCell>
                    ))}
                    <TableCell className="text-right">
                      <div className="flex justify-end gap-2">
                        <Button
                          size="icon"
                          variant="ghost"
                          onClick={() => {
                            const id = String(row.id ?? "");
                            const json = JSON.stringify(row, null, 2);
                            setEditing({ id, json });
                            setEditDraft(json);
                            setEditError(null);
                          }}
                          aria-label={`Edit ${row.id}`}
                        >
                          <Pencil className="h-4 w-4" />
                        </Button>
                        <Button
                          size="icon"
                          variant="ghost"
                          onClick={() => {
                            setDeleting(String(row.id ?? ""));
                            setActionError(null);
                          }}
                          aria-label={`Delete ${row.id}`}
                        >
                          <Trash2 className="h-4 w-4" />
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
        <p className="text-xs text-muted-foreground">
          Page {page + 1}
          {query.data ? ` · ${query.data.count} on this page` : ""}
        </p>
        <div className="flex gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={() => setPage((p) => Math.max(0, p - 1))}
            disabled={page === 0}
          >
            Previous
          </Button>
          <Button
            variant="outline"
            size="sm"
            onClick={() => setPage((p) => p + 1)}
            disabled={!query.data || query.data.count < PAGE_SIZE}
          >
            Next
          </Button>
        </div>
      </div>

      {/* Edit dialog */}
      <Dialog
        open={editing !== null}
        onOpenChange={(open) => {
          if (!open) {
            setEditing(null);
            setEditError(null);
          }
        }}
      >
        <DialogContent className="max-w-2xl">
          <DialogHeader>
            <DialogTitle>Edit object</DialogTitle>
            <DialogDescription>
              Replace the row identified by <code>{editing?.id}</code> with
              this CityJSONFeature snippet. The new payload is parsed by the
              cityjson DuckDB extension.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-2">
            <Label htmlFor="cityjson_data">CityJSON snippet</Label>
            <Textarea
              id="cityjson_data"
              value={editDraft}
              onChange={(e) => setEditDraft(e.target.value)}
              className="h-72"
            />
            {editError && (
              <p className="text-sm text-destructive">{editError}</p>
            )}
          </div>
          <DialogFooter>
            <Button variant="outline" onClick={() => setEditing(null)}>
              Cancel
            </Button>
            <Button
              onClick={() => {
                if (!editing) return;
                updateMutation.mutate({ id: editing.id, json: editDraft });
              }}
              disabled={updateMutation.isPending}
            >
              {updateMutation.isPending ? "Saving…" : "Save"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

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
              This permanently removes <code>{deleting}</code> from{" "}
              <code>{tableName}</code>. This action cannot be undone.
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
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            >
              {deleteMutation.isPending ? "Deleting…" : "Delete"}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

/** Pick the columns to show, putting common keys first. */
function inferColumns(rows: ObjectRow[]): string[] {
  const seen = new Set<string>();
  const ordered: string[] = [];
  const preferred = ["id", "feature_id", "object_type"];

  for (const row of rows) {
    for (const key of Object.keys(row)) {
      if (!seen.has(key)) {
        seen.add(key);
      }
    }
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
