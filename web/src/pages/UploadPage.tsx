import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Upload as UploadIcon } from "lucide-react";
import { useRef, useState, type FormEvent } from "react";
import { Link } from "react-router-dom";

import { Eyebrow } from "@/components/Eyebrow";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
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
    <div className="space-y-8">
      <header className="space-y-1.5">
        <Eyebrow>Upload</Eyebrow>
        <h1 className="text-[40px] font-semibold leading-tight tracking-tight text-ink-900 font-sans">
          Upload CityJSON
        </h1>
        <p className="text-[14px] text-ink-500 max-w-prose">
          Send a <code className="cl-code">.city.json</code>,{" "}
          <code className="cl-code">.city.jsonl</code>, or{" "}
          <code className="cl-code">.fcb</code> file to CityLake. One table per
          LOD will be created — or just the LOD you pin below.
        </p>
      </header>

      <form onSubmit={onSubmit} className="space-y-5">
        <Card>
          <CardHeader>
            <Eyebrow>File</Eyebrow>
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
                "flex flex-col items-center justify-center gap-2 rounded-md border border-dashed border-paper-300 bg-paper-50 p-8 text-center transition-colors duration-150 ease-cl cursor-pointer",
                dragActive && "border-lake-500 bg-lake-50",
              )}
            >
              <UploadIcon className="h-7 w-7 text-ink-500" />
              {file ? (
                <p className="font-mono text-[13px] text-ink-900">
                  {file.name}
                </p>
              ) : (
                <p className="text-[13px] text-ink-500">
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
            <Eyebrow>Options</Eyebrow>
            <CardTitle className="text-[14px] mt-1 font-sans">
              Defaults work for most uploads
            </CardTitle>
          </CardHeader>
          <CardContent className="grid gap-5 md:grid-cols-2">
            <div className="space-y-2">
              <Label htmlFor="base">Base name</Label>
              <Input
                id="base"
                mono
                value={baseName}
                onChange={(e) => setBaseName(e.target.value)}
                placeholder="city_objects"
                pattern="[A-Za-z0-9_]*"
              />
              <p className="font-mono text-[11px] text-ink-500">
                Tables: <code className="cl-code">{`${baseName || "city_objects"}_lod_X_Y`}</code>
              </p>
            </div>
            <div className="space-y-2">
              <Label htmlFor="lod">LOD (optional)</Label>
              <Input
                id="lod"
                mono
                value={lod}
                onChange={(e) => setLod(e.target.value)}
                placeholder="2.2"
              />
              <p className="font-mono text-[11px] text-ink-500">
                Pin a single LOD to load only that one. Leave blank to fan out
                across every LOD in the file.
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
          <Card accent="error">
            <CardContent className="pt-5 font-mono text-[12px] text-roof-700">
              {mutation.error.message}
            </CardContent>
          </Card>
        )}

        {mutation.data && (
          <Card accent="info">
            <CardHeader>
              <Eyebrow>Upload complete</Eyebrow>
              <CardTitle className="text-[14px] mt-1 font-sans">
                {mutation.data.message}
              </CardTitle>
            </CardHeader>
            <CardContent>
              <p className="font-mono text-[11px] text-ink-500 mb-2">
                Tables created
              </p>
              <ul className="space-y-1 font-mono text-[13px]">
                {mutation.data.tables.map((t) => (
                  <li key={t}>
                    <Link
                      to={`/tables/${t}`}
                      className="text-lake-700 hover:text-lake-900 underline"
                    >
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
