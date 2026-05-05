import { Navigate, Route, Routes } from "react-router-dom";

import { ProtectedRoute } from "@/auth/ProtectedRoute";
import AppShell from "@/components/AppShell";
import AuthCallbackPage from "@/pages/AuthCallbackPage";
import DatasetDetailPage from "@/pages/DatasetDetailPage";
import DatasetsPage from "@/pages/DatasetsPage";
import LodTablePage from "@/pages/LodTablePage";
import UploadPage from "@/pages/UploadPage";

export default function App() {
  return (
    <Routes>
      {/* OAuth landing — public, surfaces callback errors clearly */}
      <Route path="auth/callback" element={<AuthCallbackPage />} />

      <Route
        element={
          <ProtectedRoute>
            <AppShell />
          </ProtectedRoute>
        }
      >
        <Route index element={<Navigate to="/datasets" replace />} />
        <Route path="datasets" element={<DatasetsPage />} />
        <Route path="datasets/:base" element={<DatasetDetailPage />} />
        <Route path="tables/:tableName" element={<LodTablePage />} />
        <Route path="upload" element={<UploadPage />} />
        <Route path="*" element={<Navigate to="/datasets" replace />} />
      </Route>
    </Routes>
  );
}
