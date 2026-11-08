import { useEffect, useState } from "react";
import { Navigate, Route, Routes, useParams } from "react-router-dom";
import { useT } from "./i18n";
import { me as fetchMe } from "./lib/api";
import type { Me } from "./lib/types";
import { LoginPage } from "./pages/LoginPage";
import { ProjectsPage } from "./pages/ProjectsPage";
import { DocumentsPage } from "./pages/DocumentsPage";
import { DocumentPage } from "./pages/DocumentPage";
import { SearchPage, clearSearchCache } from "./pages/SearchPage";
import { ImpactPage } from "./pages/ImpactPage";
import { ChunkExplorerPage } from "./pages/ChunkExplorerPage";
import { IndexesPage } from "./pages/IndexesPage";
import { GraphPage } from "./pages/GraphPage";
import { OntologyPage } from "./pages/OntologyPage";
import { CodePage } from "./pages/CodePage";
import { TestPage } from "./pages/TestPage";
import { AiPage } from "./pages/AiPage";
import { InquiryPage } from "./pages/InquiryPage";
import { WorkflowEditorPage } from "./pages/WorkflowEditorPage";
import { SecurityPage } from "./pages/SecurityPage";
import { UsersPage } from "./pages/UsersPage";

export default function App() {
  const [me, setMe] = useState<Me | null>(null);
  const [loading, setLoading] = useState(true);
  const { t } = useT();

  useEffect(() => {
    fetchMe()
      .then(setMe)
      .catch(() => setMe(null))
      .finally(() => setLoading(false));
  }, []);

  if (loading) {
    return (
      <div className="auth-screen">
        <div className="panel panel-body stack splash-panel">
          <div className="skeleton skeleton-60" />
          <div className="skeleton skeleton-90" />
          <span className="faint">{t("common.loading")}</span>
        </div>
      </div>
    );
  }

  const onLoggedOut = () => {
    clearSearchCache();
    setMe(null);
  };

  return (
    <Routes>
      <Route
        path="/login"
        element={me ? <Navigate to="/" replace /> : <LoginPage onLogin={setMe} />}
      />
      <Route
        path="/"
        element={
          me ? <ProjectsPage me={me} onLoggedOut={onLoggedOut} /> : <Navigate to="/login" replace />
        }
      />
      <Route
        path="/security"
        element={
          me ? <SecurityPage me={me} onLoggedOut={onLoggedOut} /> : <Navigate to="/login" replace />
        }
      />
      <Route
        path="/users"
        element={
          me ? <UsersPage me={me} onLoggedOut={onLoggedOut} /> : <Navigate to="/login" replace />
        }
      />
      <Route path="/projects/:pid/documents" element={<DocumentsRoute me={me} onLoggedOut={onLoggedOut} />} />
      <Route
        path="/projects/:pid/documents/:did"
        element={<DocumentRoute me={me} onLoggedOut={onLoggedOut} />}
      />
      <Route
        path="/projects/:pid/documents/:did/explorer"
        element={<ExplorerRoute me={me} onLoggedOut={onLoggedOut} />}
      />
      <Route path="/projects/:pid/indexes" element={<IndexesRoute me={me} onLoggedOut={onLoggedOut} />} />
      <Route path="/projects/:pid/graph" element={<GraphRoute me={me} onLoggedOut={onLoggedOut} />} />
      <Route path="/projects/:pid/code" element={<CodeRoute me={me} onLoggedOut={onLoggedOut} />} />
      <Route path="/projects/:pid/ontology" element={<OntologyRoute me={me} onLoggedOut={onLoggedOut} />} />
      <Route path="/projects/:pid/search" element={<SearchRoute me={me} onLoggedOut={onLoggedOut} />} />
      <Route path="/projects/:pid/impact" element={<ImpactRoute me={me} onLoggedOut={onLoggedOut} />} />
      <Route path="/projects/:pid/tests" element={<TestRoute me={me} onLoggedOut={onLoggedOut} />} />
      <Route path="/projects/:pid/ai" element={<AiRoute me={me} onLoggedOut={onLoggedOut} />} />
      <Route path="/projects/:pid/inquiry" element={<InquiryRoute me={me} onLoggedOut={onLoggedOut} />} />
      <Route
        path="/projects/:pid/workflows/:wid/edit"
        element={<WorkflowEditorRoute me={me} onLoggedOut={onLoggedOut} />}
      />
      <Route path="*" element={<Navigate to="/" replace />} />
    </Routes>
  );
}

function DocumentsRoute({ me, onLoggedOut }: { me: Me | null; onLoggedOut: () => void }) {
  const { pid = "" } = useParams();
  if (!me) return <Navigate to="/login" replace />;
  const membership = me.memberships.find((item) => item.project_id === pid);
  if (!membership) return <Navigate to="/" replace />;
  return <DocumentsPage me={me} membership={membership} onLoggedOut={onLoggedOut} />;
}

function DocumentRoute({ me, onLoggedOut }: { me: Me | null; onLoggedOut: () => void }) {
  const { pid = "" } = useParams();
  if (!me) return <Navigate to="/login" replace />;
  const membership = me.memberships.find((item) => item.project_id === pid);
  if (!membership) return <Navigate to="/" replace />;
  return <DocumentPage me={me} membership={membership} onLoggedOut={onLoggedOut} />;
}

function ImpactRoute({ me, onLoggedOut }: { me: Me | null; onLoggedOut: () => void }) {
  const { pid = "" } = useParams();
  if (!me) return <Navigate to="/login" replace />;
  const membership = me.memberships.find((item) => item.project_id === pid);
  if (!membership) return <Navigate to="/" replace />;
  return <ImpactPage me={me} membership={membership} onLoggedOut={onLoggedOut} />;
}

function TestRoute({ me, onLoggedOut }: { me: Me | null; onLoggedOut: () => void }) {
  const { pid = "" } = useParams();
  if (!me) return <Navigate to="/login" replace />;
  const membership = me.memberships.find((item) => item.project_id === pid);
  if (!membership) return <Navigate to="/" replace />;
  return <TestPage me={me} membership={membership} onLoggedOut={onLoggedOut} />;
}

function AiRoute({ me, onLoggedOut }: { me: Me | null; onLoggedOut: () => void }) {
  const { pid = "" } = useParams();
  if (!me) return <Navigate to="/login" replace />;
  const membership = me.memberships.find((item) => item.project_id === pid);
  if (!membership) return <Navigate to="/" replace />;
  return <AiPage me={me} membership={membership} onLoggedOut={onLoggedOut} />;
}

function WorkflowEditorRoute({ me, onLoggedOut }: { me: Me | null; onLoggedOut: () => void }) {
  const { pid = "" } = useParams();
  if (!me) return <Navigate to="/login" replace />;
  const membership = me.memberships.find((item) => item.project_id === pid);
  if (!membership) return <Navigate to="/" replace />;
  return <WorkflowEditorPage me={me} membership={membership} onLoggedOut={onLoggedOut} />;
}

function InquiryRoute({ me, onLoggedOut }: { me: Me | null; onLoggedOut: () => void }) {
  const { pid = "" } = useParams();
  if (!me) return <Navigate to="/login" replace />;
  const membership = me.memberships.find((item) => item.project_id === pid);
  if (!membership) return <Navigate to="/" replace />;
  return <InquiryPage me={me} membership={membership} onLoggedOut={onLoggedOut} />;
}

function ExplorerRoute({ me, onLoggedOut }: { me: Me | null; onLoggedOut: () => void }) {
  const { pid = "" } = useParams();
  if (!me) return <Navigate to="/login" replace />;
  const membership = me.memberships.find((item) => item.project_id === pid);
  if (!membership) return <Navigate to="/" replace />;
  return <ChunkExplorerPage me={me} membership={membership} onLoggedOut={onLoggedOut} />;
}

function IndexesRoute({ me, onLoggedOut }: { me: Me | null; onLoggedOut: () => void }) {
  const { pid = "" } = useParams();
  if (!me) return <Navigate to="/login" replace />;
  const membership = me.memberships.find((item) => item.project_id === pid);
  if (!membership) return <Navigate to="/" replace />;
  return <IndexesPage me={me} membership={membership} onLoggedOut={onLoggedOut} />;
}

function GraphRoute({ me, onLoggedOut }: { me: Me | null; onLoggedOut: () => void }) {
  const { pid = "" } = useParams();
  if (!me) return <Navigate to="/login" replace />;
  const membership = me.memberships.find((item) => item.project_id === pid);
  if (!membership) return <Navigate to="/" replace />;
  return <GraphPage me={me} membership={membership} onLoggedOut={onLoggedOut} />;
}

function CodeRoute({ me, onLoggedOut }: { me: Me | null; onLoggedOut: () => void }) {
  const { pid = "" } = useParams();
  if (!me) return <Navigate to="/login" replace />;
  const membership = me.memberships.find((item) => item.project_id === pid);
  if (!membership) return <Navigate to="/" replace />;
  return <CodePage me={me} membership={membership} onLoggedOut={onLoggedOut} />;
}

function OntologyRoute({ me, onLoggedOut }: { me: Me | null; onLoggedOut: () => void }) {
  const { pid = "" } = useParams();
  if (!me) return <Navigate to="/login" replace />;
  const membership = me.memberships.find((item) => item.project_id === pid);
  if (!membership) return <Navigate to="/" replace />;
  return <OntologyPage me={me} membership={membership} onLoggedOut={onLoggedOut} />;
}

function SearchRoute({ me, onLoggedOut }: { me: Me | null; onLoggedOut: () => void }) {  const { pid = "" } = useParams();
  if (!me) return <Navigate to="/login" replace />;
  const membership = me.memberships.find((item) => item.project_id === pid);
  if (!membership) return <Navigate to="/" replace />;
  return <SearchPage me={me} membership={membership} onLoggedOut={onLoggedOut} />;
}
