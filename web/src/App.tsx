import { Link } from "react-router-dom";

export default function App() {
  return (
    <div className="min-h-screen flex flex-col items-center justify-center gap-6 p-8">
      <h1 className="text-3xl font-semibold">CityLake</h1>
      <p className="text-muted-foreground max-w-md text-center">
        A web UI for managing 3D city models in the CityLake datalake. This is a
        scaffold — pages and auth will be implemented in follow-up sessions per
        <code className="px-1">design/PLAN.md</code>.
      </p>
      <Link
        to="/login"
        className="rounded-md bg-primary px-4 py-2 text-primary-foreground"
      >
        Log in
      </Link>
    </div>
  );
}
