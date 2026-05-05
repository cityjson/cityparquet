import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Upload as UploadIcon } from "lucide-react";
import { useRef, useState, type FormEvent } from "react";
import { Link } from "react-router-dom";

import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { cn } from "@/lib/utils";
import { uploadCreateTable, type CreateTableResponse } from "@/lib/api";

export default function UploadPage() {
  const qc = useQueryClient();

  const [file, setFile] = useState<File | null>(null);
  const [baseName, setBaseName] = useState("");
  const [lod, setLod] = useState("");
  const [dragActive, setDragActive] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  const mutation = useMutation<CreateTableResponse, Error, void>({
    mutationFn: async () => {
      if (!file) throw new Error("Pick a file first");
      const baseForRoute = baseName.trim() || "city_objects";
      return uploadCreateTable(baseForRoute, file, {
        lod: lod.trim() || undefined,
        base_name: baseName.trim() || undefined,
      });
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["tables"] });
    },
  });

  function pickFile(f: File | null) {
    setFile(f);
    if (!baseName && f?.name) {
      const stem = f.name
        .replace(/\.city\.jsonl?$/i, "")
        .replace(/\.fcb$/i, "")
        .replace(/[^a-zA-Z0-9_]/g, "_");
      setBaseName(stem);
    }
  }

  function onSubmit(e: FormEvent) {
    e.preventDefault();
    mutation.mutate();
  }

  return (
    <div className="space-y-6">
      <header>
        <h1 className="text-2xl font-semibold tracking-tight">Upload</h1>
        <p className="text-sm text-muted-foreground">
          Send a CityJSON, CityJSONSeq, or FlatCityBuf file to CityLake. One
          table per LOD will be created (or just the LOD you pin below).
        </p>
      </header>

      <form onSubmit={onSubmit} className="space-y-6">
        <Card>
          <CardHeader>
            <CardTitle className="text-base">File</CardTitle>
            <CardDescription>
              Drop a <code>.city.json</code>, <code>.city.jsonl</code>, or{" "}
              <code>.fcb</code> file here, or click to browse.
            </CardDescription>
          </CardHeader>
          <CardContent>
            <div
              role="button"
              tabIndex={0}
              onClick={() => inputRef.current?.click()}
              onKeyDown={(e) => {
                if (e.key === "Enter" || e.key === " ") {
                  e.preventDefault();
                  inputRef.current?.click();
                }
              }}
              onDragOver={(e) => {
                e.preventDefault();
                setDragActive(true);
              }}
              onDragLeave={() => setDragActive(false)}
              onDrop={(e) => {
                e.preventDefault();
                setDragActive(false);
                const f = e.dataTransfer.files?.[0];
                if (f) pickFile(f);
              }}
              className={cn(
                "flex flex-col items-center justify-center gap-2 rounded-md border-2 border-dashed border-border p-8 text-center transition-colors cursor-pointer",
                dragActive && "border-primary bg-muted/40",
              )}
            >
              <UploadIcon className="h-8 w-8 text-muted-foreground" />
              {file ? (
                <p className="text-sm font-medium">{file.name}</p>
              ) : (
                <p className="text-sm text-muted-foreground">
                  Drop a file or click to browse
                </p>
              )}
              <input
                ref={inputRef}
                type="file"
                className="hidden"
                accept=".json,.jsonl,.fcb,.city.json,.city.jsonl"
                onChange={(e) => pickFile(e.target.files?.[0] ?? null)}
              />
            </div>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="text-base">Options</CardTitle>
            <CardDescription>
              Defaults work for most uploads. The base name controls the table
              prefix; pin a LOD if you want only one LOD ingested.
            </CardDescription>
          </CardHeader>
          <CardContent className="grid gap-4 md:grid-cols-2">
            <div className="space-y-2">
              <Label htmlFor="base">Base name</Label>
              <Input
                id="base"
                value={baseName}
                onChange={(e) => setBaseName(e.target.value)}
                placeholder="city_objects"
                pattern="[A-Za-z0-9_]*"
              />
              <p className="text-xs text-muted-foreground">
                Tables will be named <code>{`${baseName || "city_objects"}_lod_X_Y`}</code>.
              </p>
            </div>
            <div className="space-y-2">
              <Label htmlFor="lod">LOD (optional)</Label>
              <Input
                id="lod"
                value={lod}
                onChange={(e) => setLod(e.target.value)}
                placeholder="2.2"
              />
              <p className="text-xs text-muted-foreground">
                Pin a single LOD (e.g. <code>2.2</code>) to load only that one;
                leave blank to fan out across every LOD in the file.
              </p>
            </div>
          </CardContent>
        </Card>

        <div className="flex items-center justify-between">
          <Button asChild variant="ghost">
            <Link to="/datasets">Cancel</Link>
          </Button>
          <Button type="submit" disabled={!file || mutation.isPending}>
            {mutation.isPending ? "Uploading…" : "Upload"}
          </Button>
        </div>

        {mutation.error && (
          <Card className="border-destructive/50">
            <CardContent className="pt-6 text-sm text-destructive">
              {mutation.error.message}
            </CardContent>
          </Card>
        )}

        {mutation.data && (
          <Card>
            <CardHeader>
              <CardTitle className="text-base">Upload complete</CardTitle>
              <CardDescription>{mutation.data.message}</CardDescription>
            </CardHeader>
            <CardContent>
              <p className="text-sm">Created tables:</p>
              <ul className="mt-2 list-disc pl-5 text-sm font-mono">
                {mutation.data.tables.map((t) => (
                  <li key={t}>
                    <Link to={`/tables/${t}`} className="underline">
                      {t}
                    </Link>
                  </li>
                ))}
              </ul>
            </CardContent>
          </Card>
        )}
      </form>
    </div>
  );
}
