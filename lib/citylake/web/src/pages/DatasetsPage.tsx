import { useQuery } from "@tanstack/react-query";
import { ChevronRight } from "lucide-react";
import { Link } from "react-router-dom";

import { Eyebrow } from "@/components/Eyebrow";
import { StatusDot } from "@/components/StatusDot";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { listDatasets } from "@/lib/api";

export default function DatasetsPage() {
  const { data, isLoading, error } = useQuery({
    queryKey: ["datasets"],
    queryFn: listDatasets,
  });

  const datasets = data ?? [];

  return (
    <div className="space-y-8">
      <header className="space-y-1.5">
        <Eyebrow>Datasets · {datasets.length}</Eyebrow>
        <h1 className="text-[40px] font-semibold leading-tight tracking-tight text-ink-900 font-sans">
          Datasets
        </h1>
        <p className="text-[14px] text-ink-500 max-w-prose">
          CityJSON datasets ingested into CityLake. Each dataset is a CityParquet package, one table
          per CityGML module.
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
            Failed to load datasets: {(error as Error).message}
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
          {datasets.map((name) => (
            <Link key={name} to={`/datasets/${name}`} className="block">
              <Card className="transition-colors duration-150 ease-cl hover:border-paper-300/0 hover:bg-white hover:shadow-cl-2">
                <CardHeader className="pb-2">
                  <div className="flex items-center justify-between">
                    <Eyebrow className="flex items-center gap-1.5">
                      <span>Dataset</span>
                      <span className="text-paper-300">·</span>
                      <StatusDot tone="ok" />
                      <span className="text-moss-700">READY</span>
                    </Eyebrow>
                    <ChevronRight className="h-4 w-4 text-ink-400" />
                  </div>
                  <CardTitle className="font-mono text-[18px] mt-2">{name}</CardTitle>
                </CardHeader>
              </Card>
            </Link>
          ))}
        </div>
      )}
    </div>
  );
}
