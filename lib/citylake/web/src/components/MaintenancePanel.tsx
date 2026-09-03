import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";

import { Eyebrow } from "@/components/Eyebrow";
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
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  ApiError,
  compactDataset,
  reconcileDataset,
  vacuumDataset,
  validateDataset,
  type ValidationFinding,
} from "@/lib/api";

type Tone = "neutral" | "ok" | "warn" | "error" | "info";

const severityTone: Record<string, Tone> = {
  error: "error",
  warning: "warn",
  warn: "warn",
  info: "info",
};

function errorMessage(err: unknown): string {
  return err instanceof ApiError ? err.message : err instanceof Error ? err.message : String(err);
}

/**
 * The four dataset maintenance operations that take no input beyond the
 * dataset itself: validate, reconcile, vacuum and compact. Each reports its
 * own result and its own error beside itself, so an operator can tell which
 * operation is being reported on.
 */
export function MaintenancePanel({ ds }: { ds: string }) {
  const qc = useQueryClient();
  const [vacuumConfirmOpen, setVacuumConfirmOpen] = useState(false);

  const validateMutation = useMutation({
    mutationFn: () => validateDataset(ds),
  });

  const reconcileMutation = useMutation({
    mutationFn: () => reconcileDataset(ds),
  });

  const vacuumMutation = useMutation({
    mutationFn: () => vacuumDataset(ds),
    onSuccess: () => {
      setVacuumConfirmOpen(false);
      qc.invalidateQueries({ queryKey: ["dataset", ds] });
    },
  });

  const compactMutation = useMutation({
    mutationFn: () => compactDataset(ds),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["dataset", ds] });
    },
  });

  return (
    <Card>
      <CardHeader>
        <Eyebrow>Maintenance</Eyebrow>
        <CardTitle className="text-[14px] mt-2 font-sans">
          Validate, reconcile, vacuum and compact this dataset
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-6">
        {/* Validate */}
        <div className="space-y-3">
          <div className="flex items-center justify-between gap-4">
            <p className="text-[13px] text-ink-500">
              Run every structural check the extension knows. Reports; does not repair.
            </p>
            <Button
              size="sm"
              variant="secondary"
              onClick={() => validateMutation.mutate()}
              disabled={validateMutation.isPending}
            >
              {validateMutation.isPending ? "Validating…" : "Validate"}
            </Button>
          </div>
          {validateMutation.isError && (
            <p className="font-mono text-[12px] text-roof-700">
              {errorMessage(validateMutation.error)}
            </p>
          )}
          {validateMutation.isSuccess && <ValidationReport findings={validateMutation.data} />}
        </div>

        <div className="border-t border-paper-200" />

        {/* Reconcile */}
        <div className="space-y-3">
          <div className="flex items-center justify-between gap-4">
            <p className="text-[13px] text-ink-500">
              Re-derive <code className="cl-code">feature_id</code>, the reciprocal hierarchy and
              bbox.
            </p>
            <Button
              size="sm"
              variant="secondary"
              onClick={() => reconcileMutation.mutate()}
              disabled={reconcileMutation.isPending}
            >
              {reconcileMutation.isPending ? "Reconciling…" : "Reconcile"}
            </Button>
          </div>
          {reconcileMutation.isError && (
            <p className="font-mono text-[12px] text-roof-700">
              {errorMessage(reconcileMutation.error)}
            </p>
          )}
          {reconcileMutation.isSuccess && (
            <p className="font-mono text-[12px] text-moss-700">Reconcile completed.</p>
          )}
        </div>

        <div className="border-t border-paper-200" />

        {/* Vacuum */}
        <div className="space-y-3">
          <div className="flex items-center justify-between gap-4">
            <p className="text-[13px] text-ink-500">
              Reclaim unreferenced sidecar rows (materials, textures). Deletes data.
            </p>
            <Button
              size="sm"
              variant="danger"
              onClick={() => setVacuumConfirmOpen(true)}
              disabled={vacuumMutation.isPending}
            >
              {vacuumMutation.isPending ? "Vacuuming…" : "Vacuum"}
            </Button>
          </div>
          {vacuumMutation.isError && (
            <p className="font-mono text-[12px] text-roof-700">
              {errorMessage(vacuumMutation.error)}
            </p>
          )}
          {vacuumMutation.isSuccess && (
            <p className="font-mono text-[12px] text-moss-700">
              {vacuumMutation.data.vacuumed} row{vacuumMutation.data.vacuumed === 1 ? "" : "s"}{" "}
              reclaimed.
            </p>
          )}
        </div>

        <div className="border-t border-paper-200" />

        {/* Compact */}
        <div className="space-y-3">
          <div className="flex items-center justify-between gap-4">
            <p className="text-[13px] text-ink-500">
              Merge each object table's small Parquet files via DuckLake.
            </p>
            <Button
              size="sm"
              variant="secondary"
              onClick={() => compactMutation.mutate()}
              disabled={compactMutation.isPending}
            >
              {compactMutation.isPending ? "Compacting…" : "Compact"}
            </Button>
          </div>
          {compactMutation.isError && (
            <p className="font-mono text-[12px] text-roof-700">
              {errorMessage(compactMutation.error)}
            </p>
          )}
          {compactMutation.isSuccess && (
            <p className="font-mono text-[12px] text-moss-700">
              {compactMutation.data.files_processed} file
              {compactMutation.data.files_processed === 1 ? "" : "s"} processed,{" "}
              {compactMutation.data.files_created} file
              {compactMutation.data.files_created === 1 ? "" : "s"} created.
            </p>
          )}
        </div>
      </CardContent>

      <AlertDialog
        open={vacuumConfirmOpen}
        onOpenChange={(open) => !open && setVacuumConfirmOpen(false)}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Vacuum dataset?</AlertDialogTitle>
            <AlertDialogDescription>
              This permanently deletes unreferenced sidecar rows in{" "}
              <code className="cl-code">{ds}</code>. This action cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={(e) => {
                e.preventDefault();
                vacuumMutation.mutate();
              }}
              disabled={vacuumMutation.isPending}
            >
              {vacuumMutation.isPending ? "Vacuuming…" : "Vacuum"}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </Card>
  );
}

function ValidationReport({ findings }: { findings: ValidationFinding[] }) {
  if (findings.length === 0) {
    return <p className="font-mono text-[12px] text-moss-700">No problems found.</p>;
  }

  return (
    <Table>
      <TableHeader>
        <TableRow>
          <TableHead>Check</TableHead>
          <TableHead>Severity</TableHead>
          <TableHead>Table</TableHead>
          <TableHead>Object</TableHead>
          <TableHead>Message</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {findings.map((f, i) => (
          <TableRow key={`${f.check_name}-${f.table_name}-${f.object_id ?? "table"}-${i}`}>
            <TableCell>{f.check_name}</TableCell>
            <TableCell>
              <Tag tone={severityTone[f.severity] ?? "neutral"}>{f.severity}</Tag>
            </TableCell>
            <TableCell>{f.table_name}</TableCell>
            <TableCell>
              {f.object_id === null ? (
                <span className="italic text-ink-500">whole table</span>
              ) : (
                f.object_id
              )}
            </TableCell>
            <TableCell className="font-sans text-[12px]">{f.message}</TableCell>
          </TableRow>
        ))}
      </TableBody>
    </Table>
  );
}
