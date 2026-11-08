import type {
  AuthConfigResponse,
  CodeAnalysisAcceptedResponse,
  MemberListResponse,
  RevokeOtherSessionsResponse,
  SessionListResponse,
  CodeAnalysisRunListResponse,
  CodeFactListResponse,
  CodeRepositoryListResponse,
  CodeRepositorySummary,
  CodeRevisionListResponse,
  CodeSourceFileListResponse,
  CodeSymbolListResponse,
  CreateIndexBuildResponse,
  CreateCodeRepositoryRequest,
  DocumentChunkListResponse,
  DocumentDetail,
  RevisionListResponse,
  DocumentListResponse,
  EvidenceResponse,
  GraphEntitySearchResponse,
  GraphNeighborsResponse,
  GraphPathResponse,
  GraphRebuildResponse,
  GraphStatsResponse,
  GraphSubgraphResponse,
  IndexBuildListResponse,
  IndexBuildSummary,
  CreateImpactAssessmentRequest,
  ImpactAssessmentAcceptedResponse,
  ImpactAssessmentListResponse,
  ImpactAssessmentResponse,
  JobResponse,
  LoginResponse,
  Me,
  OntologyResponse,
  ParseAcceptedResponse,
  Problem,
  PublishOntologyResponse,
  RegisterDocumentRequest,
  RegisterDocumentResponse,
  SearchRequest,
  SearchResponse,
  TraceabilityResponse,
  TreeResponse,
  UploadUrlRequest,
  UploadUrlResponse,
  SyncCodeRevisionRequest,
  SyncCodeRevisionResponse,
  ApproveTestCaseResponse,
  ArtifactCleanupResponse,
  CreateTestRunRequest,
  CreateTestRunResponse,
  DeriveTestPlanRequest,
  DeriveTestPlanResponse,
  ExpireArtifactsRequest,
  ExtractRulesRequest,
  ExtractRulesResponse,
  RecomputeCoverageRequest,
  CoverageView,
  TestOverviewResponse,
  AiOverviewResponse,
  AgentView,
  AgentRunView,
  McpServerView,
  McpToolsResponse,
  WorkflowView,
  ApprovalView,
  RegisterAgentRequest,
  CreateAgentRunRequest,
  CreateWorkflowRequest,
  DecideApprovalRequest,
  RegisterMcpServerRequest,
  InquiryResponse,
  InquiryAcceptedResponse,
  CreateInquiryRequest,
  DeriveInquiryPlanRequest,
  WorkflowDetailResponse,
  NarrateInquiryResponse,
  NarrateImpactResponse,
} from "./types";

export class ApiError extends Error {
  readonly status: number;
  readonly code: string;
  readonly traceId: string;
  readonly detail?: string;

  constructor(status: number, code: string, traceId: string, detail?: string) {
    super(detail ?? code);
    this.status = status;
    this.code = code;
    this.traceId = traceId;
    this.detail = detail;
  }
}

type RequestInitWithJson = Omit<RequestInit, "body"> & { body?: unknown };

async function request<T>(path: string, init: RequestInitWithJson = {}): Promise<T> {
  const headers = new Headers(init.headers);
  let body: BodyInit | undefined;
  if (init.body !== undefined) {
    if (
      typeof init.body === "string" ||
      init.body instanceof Blob ||
      init.body instanceof FormData ||
      init.body instanceof URLSearchParams
    ) {
      body = init.body;
    } else {
      headers.set("content-type", "application/json");
      body = JSON.stringify(init.body);
    }
  }
  const response = await fetch(`/api/v1${path}`, {
    ...init,
    headers,
    body,
    credentials: "include",
  });
  if (response.status === 204) {
    return undefined as T;
  }
  const text = await response.text();
  const data = text ? (JSON.parse(text) as unknown) : undefined;
  if (!response.ok) {
    const problem = data as Problem | undefined;
    throw new ApiError(
      response.status,
      problem?.code ?? `HTTP_${response.status}`,
      problem?.trace_id ?? "",
      problem?.detail ?? undefined,
    );
  }
  return data as T;
}

export function login(loginId: string, password: string): Promise<LoginResponse> {
  return request<LoginResponse>("/auth/login", {
    method: "POST",
    body: { login_id: loginId, password },
  });
}

export function logout(): Promise<void> {
  return request<void>("/auth/logout", { method: "POST" });
}

export function authConfig(): Promise<AuthConfigResponse> {
  return request<AuthConfigResponse>("/auth/config");
}

export function listSessions(): Promise<SessionListResponse> {
  return request<SessionListResponse>("/auth/sessions");
}

export function revokeSession(sessionId: string): Promise<void> {
  return request<void>(`/auth/sessions/${sessionId}`, { method: "DELETE" });
}

export function logoutAll(): Promise<RevokeOtherSessionsResponse> {
  return request<RevokeOtherSessionsResponse>("/auth/logout-all", { method: "POST" });
}

export function listMembers(projectId: string): Promise<MemberListResponse> {
  return request<MemberListResponse>(`/projects/${projectId}/members`);
}

export function updateMemberRole(
  projectId: string,
  userId: string,
  role: string,
): Promise<void> {
  return request<void>(`/projects/${projectId}/members/${userId}`, {
    method: "PATCH",
    body: { role },
  });
}

export function me(): Promise<Me> {
  return request<Me>("/auth/me");
}

export function listDocuments(
  projectId: string,
  options: { q?: string; cursor?: string; limit?: number; docType?: string } = {},
): Promise<DocumentListResponse> {
  const params = new URLSearchParams();
  if (options.q) params.set("q", options.q);
  if (options.cursor) params.set("cursor", options.cursor);
  if (options.docType) params.set("doc_type", options.docType);
  params.set("limit", String(options.limit ?? 50));
  return request<DocumentListResponse>(`/projects/${projectId}/documents?${params.toString()}`);
}

export function createUploadUrl(
  projectId: string,
  body: UploadUrlRequest,
): Promise<UploadUrlResponse> {
  return request<UploadUrlResponse>(`/projects/${projectId}/documents:upload-url`, {
    method: "POST",
    body,
  });
}

export async function putPdf(uploadUrl: string, file: File, mimeType: string): Promise<void> {
  const url = new URL(uploadUrl, window.location.origin);
  const response = await fetch(`${url.pathname}${url.search}`, {
    method: "PUT",
    body: file,
    headers: { "content-type": mimeType },
    credentials: "include",
  });
  if (!response.ok) {
    throw new ApiError(response.status, "UPLOAD_FAILED", "", await response.text());
  }
}

export function registerDocument(
  projectId: string,
  body: RegisterDocumentRequest,
): Promise<RegisterDocumentResponse> {
  return request<RegisterDocumentResponse>(`/projects/${projectId}/documents`, {
    method: "POST",
    body,
  });
}

export function getDocument(documentId: string): Promise<DocumentDetail> {
  return request<DocumentDetail>(`/documents/${documentId}`);
}

export function deleteDocument(documentId: string): Promise<void> {
  return request<void>(`/documents/${documentId}`, { method: "DELETE" });
}

export function parseDocument(
  documentId: string,
  revisionId: string,
  idempotencyKey: string,
): Promise<ParseAcceptedResponse> {
  return request<ParseAcceptedResponse>(
    `/documents/${documentId}/revisions/${revisionId}:parse`,
    { method: "POST", headers: { "idempotency-key": idempotencyKey } },
  );
}

export function getJob(jobId: string): Promise<JobResponse> {
  return request<JobResponse>(`/jobs/${jobId}`);
}

export function search(projectId: string, body: SearchRequest): Promise<SearchResponse> {
  return request<SearchResponse>(`/projects/${projectId}/knowledge/search`, {
    method: "POST",
    body,
  });
}

export function getEvidence(evidenceId: string): Promise<EvidenceResponse> {
  return request<EvidenceResponse>(`/evidence/${evidenceId}`);
}

export function documentContentUrl(documentId: string): string {
  return `/api/v1/documents/${documentId}/content`;
}

export function documentRevisionContentUrl(documentId: string, revisionId: string): string {
  return `/api/v1/documents/${documentId}/revisions/${revisionId}/content`;
}

export function listDocumentRevisions(documentId: string): Promise<RevisionListResponse> {
  return request<RevisionListResponse>(`/documents/${documentId}/revisions`);
}

export function activateDocumentRevision(
  projectId: string,
  documentId: string,
  revisionId: string,
): Promise<RevisionListResponse> {
  return request<RevisionListResponse>(
    `/projects/${projectId}/documents/${documentId}/revisions/${revisionId}:activate`,
    { method: "POST" },
  );
}

export function jobEventsUrl(jobId: string): string {
  return `/api/v1/jobs/${jobId}/events`;
}

export function getDocumentTree(
  documentId: string,
  options: { revisionId?: string; parent?: string; limit?: number; cursor?: string } = {},
): Promise<TreeResponse> {
  const params = new URLSearchParams();
  if (options.revisionId) params.set("revision_id", options.revisionId);
  if (options.parent) params.set("parent", options.parent);
  params.set("limit", String(options.limit ?? 100));
  if (options.cursor) params.set("cursor", options.cursor);
  return request<TreeResponse>(`/documents/${documentId}/tree?${params.toString()}`);
}

export function getDocumentChunks(
  documentId: string,
  options: { revisionId?: string; limit?: number; cursor?: string } = {},
): Promise<DocumentChunkListResponse> {
  const params = new URLSearchParams();
  if (options.revisionId) params.set("revision_id", options.revisionId);
  params.set("limit", String(options.limit ?? 50));
  if (options.cursor) params.set("cursor", options.cursor);
  return request<DocumentChunkListResponse>(`/documents/${documentId}/chunks?${params.toString()}`);
}

export function listIndexBuilds(projectId: string): Promise<IndexBuildListResponse> {
  return request<IndexBuildListResponse>(`/projects/${projectId}/index-builds`);
}

export function createIndexBuild(
  projectId: string,
  model: string,
): Promise<CreateIndexBuildResponse> {
  return request<CreateIndexBuildResponse>(`/projects/${projectId}/index-builds`, {
    method: "POST",
    body: { model },
  });
}

export function activateIndexBuild(
  projectId: string,
  buildId: string,
): Promise<IndexBuildSummary> {
  return request<IndexBuildSummary>(`/projects/${projectId}/index-builds/${buildId}:activate`, {
    method: "POST",
  });
}

export function getOntology(projectId: string): Promise<OntologyResponse> {
  return request<OntologyResponse>(`/projects/${projectId}/graph/ontology`);
}

export function publishOntology(
  projectId: string,
  version: string,
): Promise<PublishOntologyResponse> {
  return request<PublishOntologyResponse>(`/projects/${projectId}/graph/ontology/publish`, {
    method: "POST",
    body: { version },
  });
}

export function searchGraphEntities(
  projectId: string,
  options: { q?: string; entityType?: string; limit?: number } = {},
): Promise<GraphEntitySearchResponse> {
  const params = new URLSearchParams();
  if (options.q) params.set("q", options.q);
  if (options.entityType) params.set("entity_type", options.entityType);
  params.set("limit", String(options.limit ?? 50));
  return request<GraphEntitySearchResponse>(
    `/projects/${projectId}/graph/entities?${params.toString()}`,
  );
}

export function getGraphSubgraph(
  projectId: string,
  root: string,
  depth = 2,
): Promise<GraphSubgraphResponse> {
  return request<GraphSubgraphResponse>(
    `/projects/${projectId}/graph/subgraph?root=${root}&depth=${depth}`,
  );
}

export function getGraphNeighbors(
  projectId: string,
  entityId: string,
): Promise<GraphNeighborsResponse> {
  return request<GraphNeighborsResponse>(
    `/projects/${projectId}/graph/entities/${entityId}/neighbors`,
  );
}

export function getGraphPath(
  projectId: string,
  src: string,
  dst: string,
): Promise<GraphPathResponse> {
  return request<GraphPathResponse>(
    `/projects/${projectId}/graph/path?src=${src}&dst=${dst}&max_depth=3`,
  );
}

export function getTraceability(
  projectId: string,
  entityId: string,
): Promise<TraceabilityResponse> {
  return request<TraceabilityResponse>(
    `/projects/${projectId}/graph/traceability/${entityId}`,
  );
}

export function getGraphStats(projectId: string): Promise<GraphStatsResponse> {
  return request<GraphStatsResponse>(`/projects/${projectId}/graph/stats`);
}

export function rebuildGraph(projectId: string): Promise<GraphRebuildResponse> {
  return request<GraphRebuildResponse>(`/projects/${projectId}/graph/rebuild`, {
    method: "POST",
  });
}

export function listCodeRepositories(projectId: string): Promise<CodeRepositoryListResponse> {
  return request<CodeRepositoryListResponse>(`/projects/${projectId}/code/repositories`);
}

export function createCodeRepository(
  projectId: string,
  body: CreateCodeRepositoryRequest,
): Promise<CodeRepositorySummary> {
  return request<CodeRepositorySummary>(`/projects/${projectId}/code/repositories`, {
    method: "POST",
    body,
  });
}

export function syncCodeRevision(
  projectId: string,
  body: SyncCodeRevisionRequest,
): Promise<SyncCodeRevisionResponse> {
  return request<SyncCodeRevisionResponse>(`/projects/${projectId}/code/revisions:sync`, {
    method: "POST",
    body,
  });
}

export function listCodeRevisions(
  projectId: string,
  repositoryId?: string,
): Promise<CodeRevisionListResponse> {
  const params = new URLSearchParams();
  if (repositoryId) params.set("repository_id", repositoryId);
  params.set("limit", "50");
  return request<CodeRevisionListResponse>(
    `/projects/${projectId}/code/revisions?${params.toString()}`,
  );
}

export function listCodeAnalysisRuns(projectId: string): Promise<CodeAnalysisRunListResponse> {
  return request<CodeAnalysisRunListResponse>(
    `/projects/${projectId}/code/analysis-runs?limit=20`,
  );
}

export function createCodeAnalysis(
  projectId: string,
  revisionId: string,
): Promise<CodeAnalysisAcceptedResponse> {
  return request<CodeAnalysisAcceptedResponse>(`/projects/${projectId}/code/analysis-runs`, {
    method: "POST",
    body: { revision_id: revisionId },
  });
}

export function listCodeSymbols(
  projectId: string,
  revisionId: string,
  fqn = "",
): Promise<CodeSymbolListResponse> {
  const params = new URLSearchParams({ revision_id: revisionId, fqn, limit: "100" });
  return request<CodeSymbolListResponse>(
    `/projects/${projectId}/code/symbols?${params.toString()}`,
  );
}

export function listCodeFacts(
  projectId: string,
  revisionId: string,
  subject = "",
  relation = "",
): Promise<CodeFactListResponse> {
  const params = new URLSearchParams({
    revision_id: revisionId,
    subject,
    relation,
    limit: "100",
  });
  return request<CodeFactListResponse>(`/projects/${projectId}/code/facts?${params.toString()}`);
}

export function listCodeFiles(
  projectId: string,
  revisionId: string,
): Promise<CodeSourceFileListResponse> {
  return request<CodeSourceFileListResponse>(
    `/projects/${projectId}/code/revisions/${revisionId}/files`,
  );
}

export function codeFileContentUrl(fileId: string): string {
  return `/api/v1/code/files/${fileId}/content`;
}

export function createImpactAssessment(
  projectId: string,
  body: CreateImpactAssessmentRequest,
): Promise<ImpactAssessmentAcceptedResponse> {
  return request<ImpactAssessmentAcceptedResponse>(`/projects/${projectId}/impacts`, {
    method: "POST",
    body,
  });
}

export function getImpactAssessment(impactId: string): Promise<ImpactAssessmentResponse> {
  return request<ImpactAssessmentResponse>(`/impacts/${impactId}`);
}

export function listImpactAssessments(projectId: string): Promise<ImpactAssessmentListResponse> {
  return request<ImpactAssessmentListResponse>(`/projects/${projectId}/impacts?limit=20`);
}


// ── Phase 6: Test Engineering ──────────────────────────────

export function getTestOverview(projectId: string): Promise<TestOverviewResponse> {
  return request<TestOverviewResponse>(`/projects/${projectId}/test/overview`);
}

export function extractTestRules(
  projectId: string,
  body: ExtractRulesRequest,
): Promise<ExtractRulesResponse> {
  return request<ExtractRulesResponse>(`/projects/${projectId}/test/rules:extract`, {
    method: "POST",
    body,
  });
}

export function deriveTestPlan(
  projectId: string,
  body: DeriveTestPlanRequest,
): Promise<DeriveTestPlanResponse> {
  return request<DeriveTestPlanResponse>(`/projects/${projectId}/test/plans:derive`, {
    method: "POST",
    body,
  });
}

export function approveTestCase(
  projectId: string,
  caseId: string,
): Promise<ApproveTestCaseResponse> {
  return request<ApproveTestCaseResponse>(
    `/projects/${projectId}/test/cases/${caseId}/approve`,
    { method: "POST" },
  );
}

export function createTestRun(
  projectId: string,
  body: CreateTestRunRequest,
): Promise<CreateTestRunResponse> {
  return request<CreateTestRunResponse>(`/projects/${projectId}/test/runs`, {
    method: "POST",
    body,
  });
}

export function recomputeTestCoverage(
  projectId: string,
  body: RecomputeCoverageRequest,
): Promise<CoverageView> {
  return request<CoverageView>(`/projects/${projectId}/test/coverage:recompute`, {
    method: "POST",
    body,
  });
}

export function expireTestArtifacts(
  projectId: string,
  body: ExpireArtifactsRequest,
): Promise<ArtifactCleanupResponse> {
  return request<ArtifactCleanupResponse>(`/projects/${projectId}/test/artifacts:expire`, {
    method: "POST",
    body,
  });
}

export function cleanupRunArtifacts(
  projectId: string,
  runId: string,
): Promise<ArtifactCleanupResponse> {
  return request<ArtifactCleanupResponse>(
    `/projects/${projectId}/test/runs/${runId}/artifacts:cleanup`,
    { method: "POST" },
  );
}

// ── Phase 7: Agent / MCP / Workflow ────────────────────────

export function getAiOverview(projectId: string): Promise<AiOverviewResponse> {
  return request<AiOverviewResponse>(`/projects/${projectId}/ai/overview`);
}

export function registerAgent(
  projectId: string,
  body: RegisterAgentRequest,
): Promise<AgentView> {
  return request<AgentView>(`/projects/${projectId}/agents`, { method: "POST", body });
}

export function createAgentRun(
  projectId: string,
  code: string,
  body: CreateAgentRunRequest,
): Promise<{ run_id: string; job_id: string; status: string }> {
  return request(`/projects/${projectId}/agents/${code}/runs`, { method: "POST", body });
}

export function listAgentRuns(projectId: string): Promise<{ items: AgentRunView[] }> {
  return request(`/projects/${projectId}/agent-runs`);
}

export function registerMcpServer(
  projectId: string,
  body: RegisterMcpServerRequest,
): Promise<McpServerView> {
  return request<McpServerView>(`/projects/${projectId}/mcp/servers`, {
    method: "POST",
    body,
  });
}

export function discoverMcpServer(
  projectId: string,
  name: string,
): Promise<McpServerView> {
  return request<McpServerView>(
    `/projects/${projectId}/mcp/servers/${encodeURIComponent(name)}/discover`,
    { method: "POST" },
  );
}

export function getMcpTools(projectId: string): Promise<McpToolsResponse> {
  return request<McpToolsResponse>(`/projects/${projectId}/mcp/tools`);
}

export function createWorkflow(
  projectId: string,
  body: CreateWorkflowRequest,
): Promise<WorkflowView> {
  return request<WorkflowView>(`/projects/${projectId}/workflows`, {
    method: "POST",
    body,
  });
}

export function createWorkflowRun(
  projectId: string,
  workflowId: string,
): Promise<{ run_id: string; job_id: string; status: string }> {
  return request(`/projects/${projectId}/workflows/${workflowId}/runs`, {
    method: "POST",
    body: { input: {} },
  });
}

export function decideApproval(
  projectId: string,
  approvalId: string,
  body: DecideApprovalRequest,
): Promise<ApprovalView> {
  return request<ApprovalView>(
    `/projects/${projectId}/approvals/${approvalId}/decide`,
    { method: "POST", body },
  );
}

// ── Q-01: Change Inquiry ──────────────────────────────────

export function createInquiry(
  projectId: string,
  body: CreateInquiryRequest,
): Promise<InquiryAcceptedResponse> {
  return request<InquiryAcceptedResponse>(`/projects/${projectId}/change-inquiries`, {
    method: "POST",
    body,
  });
}

export function getInquiry(projectId: string, inquiryId: string): Promise<InquiryResponse> {
  return request<InquiryResponse>(
    `/projects/${projectId}/change-inquiries/${inquiryId}`,
  );
}

export async function getInquiryEvents(
  projectId: string,
  inquiryId: string,
): Promise<{ event: string; data: unknown }[]> {
  const response = await fetch(
    `/api/v1/projects/${projectId}/change-inquiries/${inquiryId}/events`,
    { credentials: "include" },
  );
  if (!response.ok) throw new Error(`events: HTTP ${response.status}`);
  const text = await response.text();
  return text
    .split(/\n\n/)
    .map((block) => {
      let event = "message";
      let id: string | undefined;
      const data: string[] = [];
      for (const line of block.split("\n")) {
        if (!line || line.startsWith(":")) continue;
        if (line.startsWith("event:")) event = line.slice(6).trim();
        else if (line.startsWith("id:")) id = line.slice(3).trim();
        else if (line.startsWith("data:")) data.push(line.slice(5).trimStart());
      }
      if (!data.length) return null;
      return {
        id,
        event,
        data: JSON.parse(data.join("\n")),
      };
    })
    .filter((item): item is NonNullable<typeof item> => item !== null);
}

export function deriveInquiryPlan(
  projectId: string,
  inquiryId: string,
  body: DeriveInquiryPlanRequest,
): Promise<DeriveTestPlanResponse> {
  return request<DeriveTestPlanResponse>(
    `/projects/${projectId}/change-inquiries/${inquiryId}/test-plans:derive`,
    { method: "POST", body },
  );
}

export function getWorkflowDetail(
  projectId: string,
  workflowId: string,
): Promise<WorkflowDetailResponse> {
  return request<WorkflowDetailResponse>(
    `/projects/${projectId}/workflows/${workflowId}`,
  );
}

export function narrateInquiry(
  projectId: string,
  inquiryId: string,
): Promise<NarrateInquiryResponse> {
  return request<NarrateInquiryResponse>(
    `/projects/${projectId}/change-inquiries/${inquiryId}/narrate`,
    { method: "POST" },
  );
}

export function narrateImpact(impactId: string): Promise<NarrateImpactResponse> {
  return request<NarrateImpactResponse>(`/impacts/${impactId}/narrate`, {
    method: "POST",
  });
}
