#![forbid(unsafe_code)]

pub mod ai;
pub mod artifacts;
pub mod auth;
pub mod code;
pub mod document;
pub mod error;
pub mod explorer;
pub mod graph;
pub mod health;
pub mod impact;
pub mod index_build;
pub mod inquiry;
pub mod job;
pub mod knowledge;
mod mcp_client;
pub mod parse;
pub mod report;
pub mod scim;
pub mod test;

pub use auth::{AuthAudit, AuthError, Authenticated, Credential};
pub use error::AppError;
pub use health::Readiness;
pub use parse::{ParsePage, ParseTarget};
pub use store::commit as commit_tx;
pub use store::{
    AuditDraft, ChunkDraft, CodeAnalysisRunRow, CodeFactRow, CodeRepositoryRow, CodeRevisionRow,
    CodeSourceFileRow, CodeSymbolRow, DocumentFilter, EvidenceDraft, JobEvent, JobRow, MemberRow,
    NewCodeAnalysisRun, NewCodeEvidence, NewCodeFact, NewCodeRepository, NewCodeRevision,
    NewCodeSymbol, NewDocument, NewJob, NewRevision, NewSourceFile, NodeDraft, RevisionRow,
    ScimGroupRow, ScimUserRow, SearchFilter, SessionIdentity, SessionRow,
};

use std::path::{Path, PathBuf};

use contracts::ErrorCode;
use domain::{ProjectId, Role, UserId};
use store::Store;

pub const MAX_UPLOAD_BYTES: u64 = 100 * 1024 * 1024;
pub const DEFAULT_UPLOAD_TTL_SECONDS: i64 = 900;

#[derive(Clone)]
pub struct App {
    store: Store,
    data_dir: PathBuf,
    /// 未設定なら `DisabledAiProvider`。生成物を捏造しないための境界。
    ai: std::sync::Arc<dyn providers::AiProvider>,
}

impl App {
    pub fn new(store: Store) -> Self {
        let data_dir = std::env::var("SYSTIVRA_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("data"));
        Self {
            store,
            data_dir,
            ai: std::sync::Arc::from(providers::ai_from_env()),
        }
    }

    pub fn with_ai_provider(mut self, ai: std::sync::Arc<dyn providers::AiProvider>) -> Self {
        self.ai = ai;
        self
    }

    pub fn ai(&self) -> &dyn providers::AiProvider {
        self.ai.as_ref()
    }

    pub fn with_data_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.data_dir = dir.into();
        self
    }

    pub fn from_database_url(url: &str) -> Result<Self, store::StoreError> {
        Ok(Self::new(Store::connect_lazy(url)?))
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub async fn readiness(&self) -> Readiness {
        Readiness::check(&self.store).await
    }

    pub async fn project_for(
        &self,
        user: UserId,
        kind: &str,
        id: uuid::Uuid,
    ) -> Result<ProjectId, AppError> {
        let mut tx = self.store().begin_unscoped().await?;
        let project = self.store().project_of(&mut tx, kind, id).await?;
        store::commit(tx).await?;
        let Some(project) = project else {
            return Err(AppError::NotFound(ErrorCode::NotFound));
        };
        let project = ProjectId(project);
        match self.role_in(user, project).await? {
            Some(_) => Ok(project),
            None => Err(AppError::NotFound(ErrorCode::NotFound)),
        }
    }

    pub async fn scope_for_resource(
        &self,
        user: UserId,
        kind: &str,
        id: uuid::Uuid,
    ) -> Result<(ProjectId, Role), AppError> {
        let project = self.project_for(user, kind, id).await?;
        let role = self
            .role_in(user, project)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFound))?;
        Ok((project, role))
    }
}
