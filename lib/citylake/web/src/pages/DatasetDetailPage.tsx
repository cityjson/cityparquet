import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowLeft, Layers3, Trash2 } from "lucide-react";
import { useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";

import { Eyebrow } from "@/components/Eyebrow";
import { ExportDialog } from "@/components/ExportDialog";
import { MaintenancePanel } from "@/components/MaintenancePanel";
import { MergeDialog } from "@/components/MergeDialog";
import { PackageDialog } from "@/components/PackageDialog";
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
import { describeDataset, dropDataset, errorMessage } from "@/lib/api";

export default function DatasetDetailPage() {
  const { ds = "" } = useParams();
  const navigate = useNavigate();
  const qc = useQueryClient();

  const [dropping, setDropping] = useState(false);
  const [dropError, setDropError] = useState<string | null>(null);

  const query = useQuery({
    queryKey: ["dataset", ds],
    queryFn: () => describeDataset(ds),
    enabled: !!ds,
  });

  const dropMutation = useMutation({
    mutationFn: () => dropDataset(ds),
    onSuccess: () => {
      setDropping(false);
      qc.invalidateQueries({ queryKey: ["datasets"] });
      navigate("/datasets");
    },
    onError: (err: unknown) => {
      setDropError(errorMessage(err));
    },
  });

  const dataset = query.data;
  const modules = dataset?.modules ?? [];

  return (
    <div className="space-y-8">
      <Button asChild variant="ghost" size="sm" className="-ml-2">
        <Link to="/datasets">
          <ArrowLeft className="h-3.5 w-3.5" /> Back to datasets
        </Link>
      </Button>

      <header className="space-y-2">
        <div className="flex items-center gap-3 flex-wrap">
          <Eyebrow>Dataset</Eyebrow>
          <Tag tone="ok">
            <StatusDot tone="ok" />
            READY
          </Tag>
          {dataset && (
            <Tag tone="info" square>
              {dataset.crs ?? "not stated"}
            </Tag>
          )}
        </div>
        <h1 className="font-mono text-[40px] font-semibold leading-tight tracking-tight text-ink-900">
          {ds}
        </h1>
        <p className="text-[14px] text-ink-500">
          {modules.length} module{modules.length === 1 ? "" : "s"}.
        </p>
      </header>

      {query.error && (
        <Card accent="error">
          <CardContent className="pt-5 font-mono text-[12px] text-roof-700">
            {errorMessage(query.error)}
          </CardContent>
        </Card>
      )}

      {dropError && (
        <Card accent="error">
          <CardContent className="pt-5 font-mono text-[12px] text-roof-700">
            {dropError}
          </CardContent>
        </Card>
      )}

      <Card>
        <CardHeader>
          <Eyebrow>Modules</Eyebrow>
          <CardTitle className="text-[14px] mt-2 font-sans">
            Open an object module to browse its CityObjects
          </CardTitle>
        </CardHeader>
        <CardContent>
          {query.isLoading && <Skeleton className="h-24" />}
          {!query.isLoading && modules.length === 0 && (
            <p className="text-[13px] text-ink-500">No modules for this dataset.</p>
          )}
          {modules.length > 0 && (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Module</TableHead>
                  <TableHead>Role</TableHead>
                  <TableHead className="text-right">Rows</TableHead>
                  <TableHead className="text-right" />
                </TableRow>
              </TableHeader>
              <TableBody>
                {modules.map((m) => (
                  <TableRow key={m.name}>
                    <TableCell>
                      <span className="inline-flex items-center gap-1.5">
                        <Layers3 className="h-3.5 w-3.5 text-ink-500" />
                        {m.name}
                      </span>
                    </TableCell>
                    <TableCell className="text-ink-500">{m.role}</TableCell>
                    <TableCell className="text-right font-mono text-[12px] text-ink-500">
                      {m.rows.toLocaleString()}
                    </TableCell>
                    <TableCell className="text-right">
                      {m.role === "object" ? (
                        <Button asChild size="sm" variant="secondary">
                          <Link to={`/datasets/${ds}/modules/${m.name}`}>Open</Link>
                        </Button>
                      ) : null}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}
        </CardContent>
      </Card>

      {ds && <MaintenancePanel ds={ds} />}

      {ds && (
        <Card>
          <CardHeader>
            <Eyebrow>Package operations</Eyebrow>
            <CardTitle className="text-[14px] mt-2 font-sans">
              Merge, export and package write
            </CardTitle>
          </CardHeader>
          <CardContent className="space-y-6">
            <MergeDialog ds={ds} />
            <div className="border-t border-paper-200" />
            <ExportDialog ds={ds} modules={modules} />
            <div className="border-t border-paper-200" />
            <PackageDialog ds={ds} />
          </CardContent>
        </Card>
      )}

      <Card accent="error">
        <CardHeader>
          <Eyebrow>Danger zone</Eyebrow>
        </CardHeader>
        <CardContent className="flex items-center justify-between gap-4">
          <p className="text-[13px] text-ink-500">
            Drop this dataset and every module table it contains. This cannot be undone.
          </p>
          <Button
            variant="danger"
            size="sm"
            onClick={() => {
              setDropError(null);
              setDropping(true);
            }}
          >
            <Trash2 className="h-3.5 w-3.5" /> Drop dataset
          </Button>
        </CardContent>
      </Card>

      <AlertDialog open={dropping} onOpenChange={(open) => !open && setDropping(false)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Drop dataset?</AlertDialogTitle>
            <AlertDialogDescription>
              This permanently removes <code className="cl-code">{ds}</code> and every module table
              it contains. This action cannot be undone.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={(e) => {
                e.preventDefault();
                dropMutation.mutate();
              }}
              disabled={dropMutation.isPending}
            >
              {dropMutation.isPending ? "Dropping…" : "Drop"}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}
