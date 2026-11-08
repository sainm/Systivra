import type { components } from "../generated/api";

export type Schemas = components["schemas"];
export type Me = Schemas["MeResponse"];
export type AuthConfigResponse = Schemas["AuthConfigResponse"];
export type SessionSummary = Schemas["SessionSummary"];
export type SessionListResponse = Schemas["SessionListResponse"];
export type RevokeOtherSessionsResponse = Schemas["RevokeOtherSessionsResponse"];
export type MemberSummary = Schemas["MemberSummary"];
export type MemberListResponse = Schemas["MemberListResponse"];
export type Membership = Schemas["MembershipSummary"];
export type LoginResponse = Schemas["LoginResponse"];
export type DocType = Schemas["DocType"];
export type DocumentSummary = Schemas["DocumentSummary"];
export type DocumentListResponse = Schemas["DocumentListResponse"];
export type DocumentDetail = Schemas["DocumentDetail"];
export type RevisionSummary = Schemas["RevisionSummary"];
export type RegisterDocumentRequest = Schemas["RegisterDocumentRequest"];
export type RegisterDocumentResponse = Schemas["RegisterDocumentResponse"];
export type UploadUrlRequest = Schemas["UploadUrlRequest"];
export type UploadUrlResponse = Schemas["UploadUrlResponse"];
export type ParseAcceptedResponse = Schemas["ParseAcceptedResponse"];
export type JobResponse = Schemas["JobResponse"];
export type JobStatus = Schemas["JobStatus"];
export type SearchRequest = Schemas["SearchRequest"];
export type SearchResponse = Schemas["SearchResponse"];
export type SearchResult = Schemas["SearchResult"];
export type EvidenceResponse = Schemas["EvidenceResponse"];
export type Problem = Schemas["Problem"];
export type TreeResponse = Schemas["TreeResponse"];
export type TreeNode = Schemas["TreeNode"];
export type DocumentChunkListResponse = Schemas["DocumentChunkListResponse"];
export type DocumentChunkSummary = Schemas["DocumentChunkSummary"];
export type IndexBuildSummary = Schemas["IndexBuildSummary"];
export type IndexBuildListResponse = Schemas["IndexBuildListResponse"];
export type CreateIndexBuildResponse = Schemas["CreateIndexBuildResponse"];
export type GraphEntity = Schemas["GraphEntity"];
export type GraphEdge = Schemas["GraphEdge"];
export type GraphEntitySearchResponse = Schemas["GraphEntitySearchResponse"];
export type GraphNeighborsResponse = Schemas["GraphNeighborsResponse"];
export type GraphSubgraphResponse = Schemas["GraphSubgraphResponse"];
export type GraphPathResponse = Schemas["GraphPathResponse"];
export type TraceabilityResponse = Schemas["TraceabilityResponse"];
export type OntologyResponse = Schemas["OntologyResponse"];
export type GraphStatsResponse = Schemas["GraphStatsResponse"];
export type GraphRebuildResponse = Schemas["GraphRebuildResponse"];
export type PublishOntologyResponse = Schemas["PublishOntologyResponse"];
export type CodeRepositorySummary = Schemas["CodeRepositorySummary"];
export type CodeRepositoryListResponse = Schemas["CodeRepositoryListResponse"];
export type CreateCodeRepositoryRequest = Schemas["CreateCodeRepositoryRequest"];
export type CodeRevisionSummary = Schemas["CodeRevisionSummary"];
export type CodeRevisionListResponse = Schemas["CodeRevisionListResponse"];
export type SyncCodeRevisionRequest = Schemas["SyncCodeRevisionRequest"];
export type SyncCodeRevisionResponse = Schemas["SyncCodeRevisionResponse"];
export type CodeAnalysisAcceptedResponse = Schemas["CodeAnalysisAcceptedResponse"];
export type CodeAnalysisRunResponse = Schemas["CodeAnalysisRunResponse"];
export type CodeAnalysisRunListResponse = Schemas["CodeAnalysisRunListResponse"];
export type CodeSymbolSummary = Schemas["CodeSymbolSummary"];
export type CodeSymbolListResponse = Schemas["CodeSymbolListResponse"];
export type CodeFactSummary = Schemas["CodeFactSummary"];
export type CodeFactListResponse = Schemas["CodeFactListResponse"];
export type CodeSourceFileSummary = Schemas["CodeSourceFileSummary"];
export type CodeSourceFileListResponse = Schemas["CodeSourceFileListResponse"];
export type CreateImpactAssessmentRequest = Schemas["CreateImpactAssessmentRequest"];
export type ImpactAssessmentAcceptedResponse = Schemas["ImpactAssessmentAcceptedResponse"];
export type ImpactAssessmentResponse = Schemas["ImpactAssessmentResponse"];
export type ImpactAssessmentDraft = Schemas["ImpactAssessmentDraft"];
export type ImpactAssessmentListResponse = Schemas["ImpactAssessmentListResponse"];
export type ImpactItem = Schemas["ImpactItem"];
export type ImpactEvidence = Schemas["ImpactEvidence"];

export interface PdfLocator {
  kind?: string;
  page?: number;
  page_end?: number;
  char_range?: [number, number];
  bbox?: number[] | null;
  sheet?: string;
  range?: string;
  anchor_cell?: string;
  region_id?: string;
  rows?: [number, number];
  section_path?: string[];
  paragraph?: number;
  table?: number;
  slide?: number;
  title?: string;
  shape_id?: string;
  shape_ids?: string[];
}

export function asLocator(value: unknown): PdfLocator {
  return (typeof value === "object" && value !== null ? value : {}) as PdfLocator;
}

// Phase 6: Test Engineering
export type TestOverviewResponse = Schemas["TestOverviewResponse"];
export type ValidationRuleView = Schemas["ValidationRuleView"];
export type TestPlanView = Schemas["TestPlanView"];
export type TestCaseView = Schemas["TestCaseView"];
export type TestDataView = Schemas["TestDataView"];
export type TestRunView = Schemas["TestRunView"];
export type TestExecutionView = Schemas["TestExecutionView"];
export type DefectCandidateView = Schemas["DefectCandidateView"];
export type CoverageView = Schemas["CoverageView"];
export type ExtractRulesRequest = Schemas["ExtractRulesRequest"];
export type ExtractRulesResponse = Schemas["ExtractRulesResponse"];
export type DeriveTestPlanRequest = Schemas["DeriveTestPlanRequest"];
export type DeriveTestPlanResponse = Schemas["DeriveTestPlanResponse"];
export type CreateTestRunRequest = Schemas["CreateTestRunRequest"];
export type CreateTestRunResponse = Schemas["CreateTestRunResponse"];
export type ApproveTestCaseResponse = Schemas["ApproveTestCaseResponse"];
export type RecomputeCoverageRequest = Schemas["RecomputeCoverageRequest"];
export type ArtifactCleanupResponse = Schemas["ArtifactCleanupResponse"];
export type ExpireArtifactsRequest = Schemas["ExpireArtifactsRequest"];
export type TestArtifact = Schemas["TestArtifact"];

// Phase 7: Agent / MCP / Workflow
export type AiOverviewResponse = Schemas["AiOverviewResponse"];
export type AgentView = Schemas["AgentView"];
export type AgentRunView = Schemas["AgentRunView"];
export type McpServerView = Schemas["McpServerView"];
export type McpToolsResponse = Schemas["McpToolsResponse"];
export type WorkflowView = Schemas["WorkflowView"];
export type WorkflowRunView = Schemas["WorkflowRunView"];
export type ApprovalView = Schemas["ApprovalView"];
export type RegisterAgentRequest = Schemas["RegisterAgentRequest"];
export type CreateAgentRunRequest = Schemas["CreateAgentRunRequest"];
export type CreateWorkflowRequest = Schemas["CreateWorkflowRequest"];
export type DecideApprovalRequest = Schemas["DecideApprovalRequest"];
export type RegisterMcpServerRequest = Schemas["RegisterMcpServerRequest"];

// Q-01: Change Inquiry
export type InquiryResponse = Schemas["InquiryResponse"];
export type InquiryAcceptedResponse = Schemas["InquiryAcceptedResponse"];
export type InquiryCandidate = Schemas["InquiryCandidate"];
export type InquirySummary = Schemas["InquirySummary"];
export type InquirySection = Schemas["InquirySection"];
export type InquiryItem = Schemas["InquiryItem"];
export type InquiryHypothesis = Schemas["InquiryHypothesis"];
export type InquiryHypothesisBasis = Schemas["InquiryHypothesisBasis"];
export type InquiryTests = Schemas["InquiryTests"];
export type CreateInquiryRequest = Schemas["CreateInquiryRequest"];
export type DeriveInquiryPlanRequest = Schemas["DeriveInquiryPlanRequest"];

export type WorkflowDetailResponse = Schemas["WorkflowDetailResponse"];
export type WorkflowDefinition = Schemas["WorkflowDefinition"];
export type WorkflowNode = Schemas["WorkflowNode"];

export type NarrateInquiryResponse = Schemas["NarrateInquiryResponse"];

export type NarrateImpactResponse = Schemas["NarrateImpactResponse"];
