import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

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
import { Label } from "@/components/ui/label";
import { Select } from "@/components/ui/select";
import { errorMessage, listDatasets, mergeDataset } from "@/lib/api";

/**
 * Merges another dataset into `ds`. `ds` is the destination and is what
 * changes, so the operation confirms — but the extension itself refuses the
 * whole merge on the first duplicate id or CRS mismatch rather than applying
 * it partially, which is worth saying before the user commits.
 */
export function MergeDialog({ ds }: { ds: string }) {
  const qc = useQueryClient();
  const [open, setOpen] = useState(false);
  const [source, setSource] = useState("");

  const datasetsQuery = useQuery({
    queryKey: ["datasets"],
    queryFn: listDatasets,
    enabled: open,
  });
  const sources = (datasetsQuery.data ?? []).filter((name) => name !== ds);

  const mergeMutation = useMutation({
    mutationFn: () => mergeDataset(ds, source),
    onSuccess: () => {
      setOpen(false);
      setSource("");
      qc.invalidateQueries({ queryKey: ["dataset", ds] });
    },
  });

  function onOpenChange(next: boolean) {
    setOpen(next);
    if (!next) {
      setSource("");
      mergeMutation.reset();
    }
  }

  return (
    <div className="space-y-3">
      <div className="flex items-center justify-between gap-4">
        <p className="text-[13px] text-ink-500">
          Fold another dataset&rsquo;s schema into <code className="cl-code">{ds}</code>.
        </p>
        <Button
          size="sm"
          variant="danger"
          onClick={() => {
            mergeMutation.reset();
            setOpen(true);
          }}
        >
          Merge…
        </Button>
      </div>
      {mergeMutation.isSuccess && (
        <p className="font-mono text-[12px] text-moss-700">Merge completed.</p>
      )}

      <AlertDialog open={open} onOpenChange={onOpenChange}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Merge a dataset into {ds}?</AlertDialogTitle>
            <AlertDialogDescription>
              Object ids must be unique across the whole destination and the two CRSs must agree, or
              the extension refuses the entire merge rather than partially applying it — there is no
              half-merged state.
            </AlertDialogDescription>
          </AlertDialogHeader>

          <div className="space-y-2">
            <Label htmlFor="merge-source">Source dataset</Label>
            <Select
              id="merge-source"
              mono
              value={source}
              onChange={(e) => setSource(e.target.value)}
              disabled={datasetsQuery.isLoading}
            >
              <option value="">
                {datasetsQuery.isLoading ? "Loading datasets…" : "Choose a dataset…"}
              </option>
              {sources.map((name) => (
                <option key={name} value={name}>
                  {name}
                </option>
              ))}
            </Select>
            {datasetsQuery.isError && (
              <p className="font-mono text-[12px] text-roof-700">
                {errorMessage(datasetsQuery.error)}
              </p>
            )}
            {datasetsQuery.isSuccess && sources.length === 0 && (
              <p className="text-[12px] text-ink-500">No other datasets to merge from.</p>
            )}
          </div>

          {mergeMutation.isError && (
            <p className="font-mono text-[12px] text-roof-700">
              {errorMessage(mergeMutation.error)}
            </p>
          )}

          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={(e) => {
                e.preventDefault();
                mergeMutation.mutate();
              }}
              disabled={!source || mergeMutation.isPending}
            >
              {mergeMutation.isPending ? "Merging…" : "Merge"}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
