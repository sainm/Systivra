use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::{DateTime, Utc};
use contracts::{
    BulkDuplicate, BulkImportMode, BulkImportRequest, BulkImportResponse, DocType, DocumentDetail,
    DocumentListQuery, DocumentListResponse, DocumentSummary, ErrorCode, JobLinks, JobStatus,
    NextLinks, ParseAcceptedResponse, RegisterDocumentRequest, RegisterDocumentResponse,
    RegisterRevisionResponse, RevisionListResponse, RevisionSummary,
};
use domain::{DocumentId, JobId, Permission, ProjectId, RevisionId};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::Authenticated;
use crate::error::AppError;
use crate::{App, MAX_UPLOAD_BYTES};

pub const PDF_MIME: &str = "application/pdf";
pub const XLSX_MIME: &str = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet";
pub const XLSM_MIME: &str = "application/vnd.ms-excel.sheet.macroEnabled.12";
pub const DOCX_MIME: &str =
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
pub const PPTX_MIME: &str =
    "application/vnd.openxmlformats-officedocument.presentationml.presentation";

pub fn is_supported_mime(mime: &str) -> bool {
    mime.eq_ignore_ascii_case(PDF_MIME)
        || mime.eq_ignore_ascii_case(XLSX_MIME)
        || mime.eq_ignore_ascii_case(XLSM_MIME)
        || mime.eq_ignore_ascii_case(DOCX_MIME)
        || mime.eq_ignore_ascii_case(PPTX_MIME)
}

impl App {
    pub fn new_object_key(&self, project: ProjectId, filename: &str) -> String {
        let safe: String = filename
            .chars()
            .map(|c| {
                if c.is_alphanumeric()
                    || matches!(c, '-' | '_' | '.' | ' ' | '（' | '）' | '(' | ')')
                {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let safe = safe.trim().trim_matches('.').to_string();
        let safe = if safe.is_empty() {
            "document.pdf".to_string()
        } else {
            safe
        };
        format!("raw/{project}/{}/{safe}", Uuid::new_v4())
    }

    pub async fn put_object(&self, object_key: &str, bytes: &[u8]) -> Result<(), AppError> {
        let path = self.object_path(object_key)?;
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::write(path, bytes).await?;
        Ok(())
    }

    pub async fn read_object(&self, object_key: &str) -> Result<Vec<u8>, AppError> {
        let path = self.object_path(object_key)?;
        Ok(tokio::fs::read(path).await?)
    }

    pub fn object_path(&self, object_key: &str) -> Result<std::path::PathBuf, AppError> {
        if object_key.contains("..") || object_key.starts_with('/') {
            return Err(AppError::Validation("object_key が不正です".into()));
        }
        Ok(self.data_dir().join(object_key))
    }

    pub async fn register_document(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        req: RegisterDocumentRequest,
        trace: Option<String>,
    ) -> Result<RegisterDocumentResponse, AppError> {
        if req.title.trim().is_empty() || req.title.chars().count() > 500 {
            return Err(AppError::Validation("title は 1〜500 文字です".into()));
        }
        let mime_type = req
            .mime_type
            .clone()
            .unwrap_or_else(|| "application/pdf".to_string());
        if !is_supported_mime(&mime_type) {
            return Err(AppError::Validation(
                "登録できるのは PDF / XLSX / DOCX / PPTX です".into(),
            ));
        }
        let prefix = format!("raw/{project}/");
        if !req.object_key.starts_with(&prefix) || req.object_key.contains("..") {
            return Err(AppError::Validation(
                "object_key がこのプロジェクトの範囲外です".into(),
            ));
        }

        let path = self.object_path(&req.object_key)?;
        let meta = tokio::fs::metadata(&path).await.map_err(|_| {
            AppError::Validation("アップロード済みのオブジェクトがありません".into())
        })?;
        if meta.len() == 0 {
            return Err(AppError::Validation("空のファイルは登録できません".into()));
        }
        if meta.len() > MAX_UPLOAD_BYTES {
            return Err(AppError::Validation(
                "ファイルサイズが上限を超えています".into(),
            ));
        }
        let bytes = tokio::fs::read(&path).await?;
        let content_hash = engine_rag::sha256(&bytes);
        if mime_type.eq_ignore_ascii_case(PDF_MIME) {
            if !bytes.starts_with(b"%PDF-") {
                return Err(AppError::Validation("PDF のシグネチャがありません".into()));
            }
        } else if !bytes.starts_with(b"PK\x03\x04") && !bytes.starts_with(b"PK\x05\x06") {
            return Err(AppError::Validation(
                "XLSX の ZIP シグネチャがありません".into(),
            ));
        }

        let document_id = DocumentId::new();
        let revision_id = RevisionId::new();
        let now = Utc::now();

        let mut tx = self.store().begin_scoped(project).await?;
        if let Some((existing_document, _)) = self
            .store()
            .find_revision_by_hash(&mut tx, &content_hash)
            .await?
        {
            let _ = existing_document;
            return Err(AppError::Conflict(ErrorCode::ConflictAlreadyImported));
        }

        let doc = self
            .store()
            .insert_document(
                &mut tx,
                &store::NewDocument {
                    id: document_id.0,
                    project_id: project.0,
                    doc_type: req.doc_type.as_str().to_string(),
                    title: engine_rag::normalize_nfc(req.title.trim()),
                    business_domain: req.business_domain.clone(),
                    acl_groups: req.acl_groups.clone(),
                },
            )
            .await?;

        self.store()
            .insert_revision(
                &mut tx,
                &store::NewRevision {
                    id: revision_id.0,
                    document_id: document_id.0,
                    source_revision: req.source_revision.clone(),
                    content_hash: content_hash.clone(),
                    object_key: req.object_key.clone(),
                    file_size: meta.len() as i64,
                    mime_type: mime_type.clone(),
                    acquired_at: now,
                },
            )
            .await?;
        self.store()
            .set_current_revision(&mut tx, document_id.0, revision_id.0)
            .await?;

        self.store()
            .append_audit(
                &mut tx,
                &store::AuditDraft {
                    project_id: Some(project.0),
                    actor_type: "USER".into(),
                    actor_id: auth.user_id.to_string(),
                    action: "document.register".into(),
                    target_type: Some("document".into()),
                    target_id: Some(document_id.to_string()),
                    input_digest: Some(content_hash.clone()),
                    result: "SUCCESS".into(),
                    detail: json!({
                        "title": doc.title,
                        "object_key": req.object_key,
                        "size_bytes": meta.len(),
                        "mime_type": mime_type.clone(),
                        "doc_type": req.doc_type.as_str(),
                        "source_revision": req.source_revision,
                    }),
                    trace_id: trace,
                },
            )
            .await?;
        store::commit(tx).await?;

        let detail = DocumentDetail {
            id: document_id,
            project_id: project,
            doc_type: req.doc_type,
            title: doc.title,
            business_domain: doc.business_domain,
            acl_groups: doc.acl_groups,
            current_revision: Some(RevisionSummary {
                id: revision_id,
                source_revision: req.source_revision,
                content_hash: engine_rag::hash_to_sha256(&content_hash),
                status: "CURRENT".into(),
                acquired_at: now,
                parsed_at: None,
                file_size: meta.len() as i64,
                mime_type,
                page_count: 0,
                chunk_count: 0,
                parse_status: "NOT_PARSED".into(),
            }),
            created_at: doc.created_at,
            updated_at: doc.created_at,
        };

        Ok(RegisterDocumentResponse {
            next: NextLinks {
                parse: format!("/api/v1/documents/{document_id}/revisions/{revision_id}:parse"),
            },
            document: detail,
        })
    }

    fn revision_summary(row: &store::RevisionRow) -> RevisionSummary {
        RevisionSummary {
            id: RevisionId(row.id),
            source_revision: row.source_revision.clone(),
            content_hash: engine_rag::hash_to_sha256(&row.content_hash),
            status: row.status.clone(),
            acquired_at: row.acquired_at,
            parsed_at: row.parsed_at,
            file_size: row.file_size,
            mime_type: row.mime_type.clone(),
            page_count: row.page_count,
            chunk_count: row.chunk_count,
            parse_status: row.parse_status.clone(),
        }
    }

    pub async fn register_revision(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        document_id: DocumentId,
        req: RegisterDocumentRequest,
        trace: Option<String>,
    ) -> Result<RegisterRevisionResponse, AppError> {
        let role = self
            .role_in(auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFoundDocument))?;
        if !Permission::DocumentIngest.access_for(role).can_write() {
            return Err(AppError::Forbidden("document:write がありません".into()));
        }
        let mime_type = req
            .mime_type
            .clone()
            .unwrap_or_else(|| PDF_MIME.to_string());
        if !is_supported_mime(&mime_type) {
            return Err(AppError::Validation(
                "登録できるのは PDF / XLSX / DOCX / PPTX です".into(),
            ));
        }
        let prefix = format!("raw/{project}/");
        if !req.object_key.starts_with(&prefix) || req.object_key.contains("..") {
            return Err(AppError::Validation(
                "object_key がこのプロジェクトの範囲外です".into(),
            ));
        }
        let path = self.object_path(&req.object_key)?;
        let meta = tokio::fs::metadata(&path).await.map_err(|_| {
            AppError::Validation("アップロード済みのオブジェクトがありません".into())
        })?;
        if meta.len() == 0 {
            return Err(AppError::Validation("空のファイルは登録できません".into()));
        }
        if meta.len() > MAX_UPLOAD_BYTES {
            return Err(AppError::Validation(
                "ファイルサイズが上限を超えています".into(),
            ));
        }
        let bytes = tokio::fs::read(&path).await?;
        let content_hash = engine_rag::sha256(&bytes);
        if mime_type.eq_ignore_ascii_case(PDF_MIME) {
            if !bytes.starts_with(b"%PDF-") {
                return Err(AppError::Validation("PDF のシグネチャがありません".into()));
            }
        } else if !bytes.starts_with(b"PK\x03\x04") && !bytes.starts_with(b"PK\x05\x06") {
            return Err(AppError::Validation(
                "XLSX の ZIP シグネチャがありません".into(),
            ));
        }

        let revision_id = RevisionId::new();
        let now = Utc::now();
        let mut tx = self.store().begin_scoped(project).await?;
        let doc = self
            .store()
            .find_document(&mut tx, document_id.0)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFoundDocument))?;
        if doc.project_id != project.0 {
            return Err(AppError::NotFound(ErrorCode::NotFoundDocument));
        }
        if let Some((_existing_document, _)) = self
            .store()
            .find_revision_by_hash(&mut tx, &content_hash)
            .await?
        {
            return Err(AppError::Conflict(ErrorCode::ConflictAlreadyImported));
        }
        self.store()
            .demote_current_revision(&mut tx, document_id.0)
            .await?;
        self.store()
            .insert_revision(
                &mut tx,
                &store::NewRevision {
                    id: revision_id.0,
                    document_id: document_id.0,
                    source_revision: req.source_revision.clone(),
                    content_hash: content_hash.clone(),
                    object_key: req.object_key.clone(),
                    file_size: meta.len() as i64,
                    mime_type: mime_type.clone(),
                    acquired_at: now,
                },
            )
            .await?;
        self.store()
            .activate_revision(&mut tx, document_id.0, revision_id.0)
            .await?;
        self.store()
            .append_audit(
                &mut tx,
                &store::AuditDraft {
                    project_id: Some(project.0),
                    actor_type: "USER".into(),
                    actor_id: auth.user_id.to_string(),
                    action: "document.revision.register".into(),
                    target_type: Some("document".into()),
                    target_id: Some(document_id.to_string()),
                    input_digest: Some(content_hash.clone()),
                    result: "SUCCESS".into(),
                    detail: json!({
                        "revision_id": revision_id.to_string(),
                        "source_revision": req.source_revision,
                        "object_key": req.object_key,
                        "mime_type": mime_type,
                    }),
                    trace_id: trace,
                },
            )
            .await?;
        store::commit(tx).await?;

        Ok(RegisterRevisionResponse {
            revision: RevisionSummary {
                id: revision_id,
                source_revision: req.source_revision,
                content_hash: engine_rag::hash_to_sha256(&content_hash),
                status: "CURRENT".into(),
                acquired_at: now,
                parsed_at: None,
                file_size: meta.len() as i64,
                mime_type,
                page_count: 0,
                chunk_count: 0,
                parse_status: "NOT_PARSED".into(),
            },
            next: NextLinks {
                parse: format!("/api/v1/documents/{document_id}/revisions/{revision_id}:parse"),
            },
        })
    }

    pub async fn list_document_revisions(
        &self,
        auth: &Authenticated,
        document_id: DocumentId,
    ) -> Result<RevisionListResponse, AppError> {
        let project = self
            .project_for(auth.user_id, "document", document_id.0)
            .await?;
        let role = self
            .role_in(auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFoundDocument))?;
        if !Permission::DocumentIngest.access_for(role).can_read() {
            return Err(AppError::NotFound(ErrorCode::NotFoundDocument));
        }
        let mut tx = self.store().begin_scoped(project).await?;
        let doc = self
            .store()
            .find_document(&mut tx, document_id.0)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFoundDocument))?;
        if doc.project_id != project.0 {
            return Err(AppError::NotFound(ErrorCode::NotFoundDocument));
        }
        let rows = self.store().list_revisions(&mut tx, document_id.0).await?;
        store::commit(tx).await?;
        Ok(RevisionListResponse {
            items: rows.iter().map(Self::revision_summary).collect(),
        })
    }

    pub async fn document_revision_content(
        &self,
        auth: &Authenticated,
        document_id: DocumentId,
        revision_id: RevisionId,
    ) -> Result<(Vec<u8>, String), AppError> {
        let project = self
            .project_for(auth.user_id, "document", document_id.0)
            .await?;
        let role = self
            .role_in(auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFoundDocument))?;
        if !Permission::DocumentIngest.access_for(role).can_read() {
            return Err(AppError::NotFound(ErrorCode::NotFoundDocument));
        }
        let mut tx = self.store().begin_scoped(project).await?;
        let revision = self
            .store()
            .find_revision(&mut tx, revision_id.0)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFoundDocument))?;
        store::commit(tx).await?;
        if revision.document_id != document_id.0 {
            return Err(AppError::NotFound(ErrorCode::NotFoundDocument));
        }
        let bytes = self.read_object(&revision.object_key).await?;
        Ok((bytes, revision.mime_type))
    }

    pub async fn activate_document_revision(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        document_id: DocumentId,
        revision_id: RevisionId,
        trace: Option<String>,
    ) -> Result<RevisionListResponse, AppError> {
        let role = self
            .role_in(auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFoundDocument))?;
        if !Permission::DocumentIngest.access_for(role).can_write() {
            return Err(AppError::Forbidden("document:write がありません".into()));
        }
        let mut tx = self.store().begin_scoped(project).await?;
        let doc = self
            .store()
            .find_document(&mut tx, document_id.0)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFoundDocument))?;
        if doc.project_id != project.0 {
            return Err(AppError::NotFound(ErrorCode::NotFoundDocument));
        }
        let revision = self
            .store()
            .find_revision(&mut tx, revision_id.0)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFoundDocument))?;
        if revision.document_id != document_id.0 {
            return Err(AppError::NotFound(ErrorCode::NotFoundDocument));
        }
        let previous = self
            .store()
            .current_revision_of(&mut tx, document_id.0)
            .await?;
        self.store()
            .activate_revision(&mut tx, document_id.0, revision_id.0)
            .await?;
        if let Some(previous) = previous {
            if previous != revision_id.0 {
                self.store()
                    .supersede_document_revision(
                        &mut tx,
                        project.0,
                        document_id.0,
                        previous,
                        revision_id.0,
                    )
                    .await?;
            }
        }
        let drafts = self
            .store()
            .document_node_drafts(&mut tx, revision_id.0)
            .await?;
        if !drafts.is_empty() {
            self.store()
                .sync_document_nodes(&mut tx, project.0, document_id.0, revision_id.0, &drafts)
                .await?;
        }
        self.store()
            .append_audit(
                &mut tx,
                &store::AuditDraft {
                    project_id: Some(project.0),
                    actor_type: "USER".into(),
                    actor_id: auth.user_id.to_string(),
                    action: "document.revision.activate".into(),
                    target_type: Some("document".into()),
                    target_id: Some(document_id.to_string()),
                    input_digest: None,
                    result: "SUCCESS".into(),
                    detail: json!({
                        "from": previous.map(|id| id.to_string()),
                        "to": revision_id.to_string(),
                    }),
                    trace_id: trace,
                },
            )
            .await?;
        let rows = self.store().list_revisions(&mut tx, document_id.0).await?;
        store::commit(tx).await?;
        Ok(RevisionListResponse {
            items: rows.iter().map(Self::revision_summary).collect(),
        })
    }

    pub async fn list_documents(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        query: DocumentListQuery,
    ) -> Result<DocumentListResponse, AppError> {
        let role = match self.role_in(auth.user_id, project).await? {
            Some(role) => role,
            None => return Err(AppError::NotFound(ErrorCode::NotFoundDocument)),
        };
        if !Permission::DocumentIngest.access_for(role).can_read() {
            return Err(AppError::Forbidden("document:read がありません".into()));
        }

        let limit = query.limit.unwrap_or(50).clamp(1, 100);
        let (cursor_created_at, cursor_id) = match &query.cursor {
            Some(cursor) => decode_cursor(cursor)?,
            None => (None, None),
        };
        let filter = store::DocumentFilter {
            doc_type: query.doc_type.map(|d| d.as_str().to_string()),
            business_domain: query.business_domain,
            parse_status: query.status,
            query: query.q,
            cursor_created_at,
            cursor_id,
        };

        let mut tx = self.store().begin_scoped(project).await?;
        let mut rows = self
            .store()
            .list_documents(&mut tx, &filter, limit + 1)
            .await?;
        store::commit(tx).await?;

        let has_more = rows.len() as i64 > limit;
        rows.truncate(limit as usize);
        let next_cursor = if has_more {
            rows.last()
                .map(|row| encode_cursor(row.updated_at, row.id))
                .transpose()?
        } else {
            None
        };

        Ok(DocumentListResponse {
            items: rows
                .into_iter()
                .map(|r| DocumentSummary {
                    id: DocumentId(r.id),
                    title: r.title,
                    doc_type: parse_doc_type(&r.doc_type),
                    business_domain: r.business_domain,
                    current_revision_id: r.current_revision_id.map(RevisionId),
                    sheet_count: 0,
                    page_count: r.page_count,
                    chunk_count: r.chunk_count,
                    parse_status: r.parse_status,
                    updated_at: r.updated_at,
                })
                .collect(),
            next_cursor,
            has_more,
        })
    }

    pub async fn document_detail(
        &self,
        auth: &Authenticated,
        document_id: DocumentId,
    ) -> Result<DocumentDetail, AppError> {
        let project = self
            .project_for(auth.user_id, "document", document_id.0)
            .await?;
        let role = self
            .role_in(auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFoundDocument))?;
        if !Permission::DocumentIngest.access_for(role).can_read() {
            return Err(AppError::NotFound(ErrorCode::NotFoundDocument));
        }

        let mut tx = self.store().begin_scoped(project).await?;
        let doc = self
            .store()
            .find_document(&mut tx, document_id.0)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFoundDocument))?;
        let revision = self
            .store()
            .find_current_revision(&mut tx, document_id.0)
            .await?;
        store::commit(tx).await?;

        Ok(DocumentDetail {
            id: DocumentId(doc.id),
            project_id: ProjectId(doc.project_id),
            doc_type: parse_doc_type(&doc.doc_type),
            title: doc.title,
            business_domain: doc.business_domain,
            acl_groups: doc.acl_groups,
            current_revision: revision.map(|r| RevisionSummary {
                id: RevisionId(r.id),
                source_revision: r.source_revision,
                content_hash: engine_rag::hash_to_sha256(&r.content_hash),
                status: r.status,
                acquired_at: r.acquired_at,
                parsed_at: r.parsed_at,
                file_size: r.file_size,
                mime_type: r.mime_type,
                page_count: r.page_count,
                chunk_count: r.chunk_count,
                parse_status: r.parse_status,
            }),
            created_at: doc.created_at,
            updated_at: doc.created_at,
        })
    }

    pub async fn document_content(
        &self,
        auth: &Authenticated,
        document_id: DocumentId,
    ) -> Result<(Vec<u8>, String), AppError> {
        let project = self
            .project_for(auth.user_id, "document", document_id.0)
            .await?;
        let role = self
            .role_in(auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFoundDocument))?;
        if !Permission::DocumentIngest.access_for(role).can_read() {
            return Err(AppError::NotFound(ErrorCode::NotFoundDocument));
        }
        let mut tx = self.store().begin_scoped(project).await?;
        let revision = self
            .store()
            .find_current_revision(&mut tx, document_id.0)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFoundDocument))?;
        store::commit(tx).await?;
        let bytes = self.read_object(&revision.object_key).await?;
        Ok((bytes, revision.mime_type))
    }

    pub async fn delete_document(
        &self,
        auth: &Authenticated,
        document_id: DocumentId,
        trace: Option<String>,
    ) -> Result<(), AppError> {
        let project = self
            .project_for(auth.user_id, "document", document_id.0)
            .await?;
        let role = self
            .role_in(auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFoundDocument))?;
        if !Permission::DocumentIngest.access_for(role).can_write() {
            return Err(AppError::Forbidden("document:write がありません".into()));
        }

        let mut tx = self.store().begin_scoped(project).await?;
        if !self.store().logical_delete(&mut tx, document_id.0).await? {
            return Err(AppError::NotFound(ErrorCode::NotFoundDocument));
        }
        self.store()
            .tombstone_document(&mut tx, project.0, document_id.0)
            .await?;
        self.store()
            .append_audit(
                &mut tx,
                &store::AuditDraft {
                    project_id: Some(project.0),
                    actor_type: "USER".into(),
                    actor_id: auth.user_id.to_string(),
                    action: "document.delete".into(),
                    target_type: Some("document".into()),
                    target_id: Some(document_id.to_string()),
                    input_digest: None,
                    result: "SUCCESS".into(),
                    detail: json!({"mode": "logical"}),
                    trace_id: trace,
                },
            )
            .await?;
        store::commit(tx).await?;
        Ok(())
    }

    pub async fn enqueue_parse(
        &self,
        auth: &Authenticated,
        document_id: DocumentId,
        revision_id: RevisionId,
        idempotency_key: Option<String>,
        trace: Option<String>,
    ) -> Result<ParseAcceptedResponse, AppError> {
        let project = self
            .project_for(auth.user_id, "document", document_id.0)
            .await?;
        let role = self
            .role_in(auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFoundDocument))?;
        if !Permission::DocumentIngest.access_for(role).can_write() {
            return Err(AppError::Forbidden("document:write がありません".into()));
        }

        let mut tx = self.store().begin_scoped(project).await?;
        let doc = self
            .store()
            .find_document(&mut tx, document_id.0)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFoundDocument))?;
        let revision = self
            .store()
            .find_revision(&mut tx, revision_id.0)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFoundDocument))?;
        if revision.document_id != document_id.0 {
            return Err(AppError::NotFound(ErrorCode::NotFoundDocument));
        }

        let key = idempotency_key.unwrap_or_else(|| format!("parse:{}", revision_id.0));
        if let Some(existing) = self
            .store()
            .find_job_by_idempotency(&mut tx, project.0, &key)
            .await?
        {
            store::commit(tx).await?;
            return Ok(ParseAcceptedResponse {
                job_id: domain::JobId(existing.id),
                status: JobStatus::parse(&existing.status).unwrap_or(JobStatus::Queued),
                mode: "FULL".into(),
            });
        }

        let parser = if revision.mime_type.eq_ignore_ascii_case(XLSX_MIME)
            || revision.mime_type.eq_ignore_ascii_case(XLSM_MIME)
        {
            "xlsx-native"
        } else if revision.mime_type.eq_ignore_ascii_case(DOCX_MIME) {
            "docx-native"
        } else if revision.mime_type.eq_ignore_ascii_case(PPTX_MIME) {
            "pptx-native"
        } else {
            "in-process-pdf"
        };
        let parse_run_id = self
            .store()
            .insert_parse_run(&mut tx, revision_id.0, parser, "0.1.0")
            .await?;
        let input = json!({
            "project_id": project,
            "document_id": document_id,
            "revision_id": revision_id,
            "parse_run_id": parse_run_id,
            "object_key": revision.object_key,
            "mime_type": revision.mime_type,
            "file_size": revision.file_size,
        });
        let enqueued = self
            .store()
            .enqueue_job(
                &mut tx,
                &store::NewJob {
                    project_id: project.0,
                    kind: "DOCUMENT_PARSE".into(),
                    idempotency_key: key,
                    input,
                },
            )
            .await?;

        self.store()
            .append_audit(
                &mut tx,
                &store::AuditDraft {
                    project_id: Some(project.0),
                    actor_type: "USER".into(),
                    actor_id: auth.user_id.to_string(),
                    action: "document.parse".into(),
                    target_type: Some("document".into()),
                    target_id: Some(document_id.to_string()),
                    input_digest: Some(revision.content_hash),
                    result: "QUEUED".into(),
                    detail: json!({
                        "title": doc.title,
                        "revision_id": revision_id,
                        "parse_run_id": parse_run_id,
                        "job_id": enqueued.job.id,
                    }),
                    trace_id: trace,
                },
            )
            .await?;
        store::commit(tx).await?;

        Ok(ParseAcceptedResponse {
            job_id: domain::JobId(enqueued.job.id),
            status: JobStatus::parse(&enqueued.job.status).unwrap_or(JobStatus::Queued),
            mode: "FULL".into(),
        })
    }

    pub async fn enqueue_bulk_import(
        &self,
        auth: &Authenticated,
        project: ProjectId,
        req: BulkImportRequest,
        trace: Option<String>,
    ) -> Result<BulkImportResponse, AppError> {
        let role = self
            .role_in(auth.user_id, project)
            .await?
            .ok_or(AppError::NotFound(ErrorCode::NotFoundDocument))?;
        if !Permission::DocumentIngest.access_for(role).can_write() {
            return Err(AppError::Forbidden("document:write がありません".into()));
        }

        let manifest_key = self.manifest_key(&req.manifest_url)?;
        let path = self.data_dir().join(&manifest_key);
        let content = tokio::fs::read(&path)
            .await
            .map_err(|_| AppError::Validation("manifest を読み込めません".into()))?;
        let estimated_files = content
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.iter().all(|byte| byte.is_ascii_whitespace()))
            .count() as i64;
        let idempotency_key = format!(
            "bulk:{manifest_key}:{}:{:?}:{:?}",
            content.len(),
            req.mode,
            req.on_duplicate
        );
        let input = json!({
            "project_id": project,
            "manifest_key": manifest_key,
            "mode": req.mode,
            "on_duplicate": req.on_duplicate,
        });

        let mut tx = self.store().begin_scoped(project).await?;
        let enqueued = self
            .store()
            .enqueue_job(
                &mut tx,
                &store::NewJob {
                    project_id: project.0,
                    kind: "BULK_IMPORT".into(),
                    idempotency_key,
                    input,
                },
            )
            .await?;
        self.store()
            .append_audit(
                &mut tx,
                &store::AuditDraft {
                    project_id: Some(project.0),
                    actor_type: "USER".into(),
                    actor_id: auth.user_id.to_string(),
                    action: "document.bulk_import".into(),
                    target_type: Some("job".into()),
                    target_id: Some(enqueued.job.id.to_string()),
                    input_digest: None,
                    result: "QUEUED".into(),
                    detail: json!({
                        "manifest_key": manifest_key,
                        "estimated_files": estimated_files,
                        "mode": req.mode,
                        "on_duplicate": req.on_duplicate,
                    }),
                    trace_id: trace,
                },
            )
            .await?;
        store::commit(tx).await?;

        Ok(BulkImportResponse {
            job_id: JobId(enqueued.job.id),
            status: JobStatus::parse(&enqueued.job.status).unwrap_or(JobStatus::Queued),
            estimated_files,
            links: JobLinks {
                events: format!("/api/v1/jobs/{}/events", enqueued.job.id),
            },
        })
    }

    pub fn manifest_key(&self, manifest_url: &str) -> Result<String, AppError> {
        let raw = manifest_url.strip_prefix("file://").unwrap_or(manifest_url);
        let path = std::path::Path::new(raw);
        let data_dir = self
            .data_dir()
            .canonicalize()
            .unwrap_or_else(|_| self.data_dir().to_path_buf());
        let candidate = if path.is_absolute() {
            path.to_path_buf()
        } else {
            data_dir.join(path)
        };
        let canonical = candidate
            .canonicalize()
            .map_err(|_| AppError::Validation("manifest が見つかりません".into()))?;
        let relative = canonical.strip_prefix(&data_dir).map_err(|_| {
            AppError::Validation("manifest はデータディレクトリ内に置いてください".into())
        })?;
        let key = relative.to_string_lossy().replace('\\', "/");
        if key.is_empty() || key.contains("..") {
            return Err(AppError::Validation("manifest のパスが不正です".into()));
        }
        Ok(key)
    }

    pub async fn bulk_import_entry(
        &self,
        project: ProjectId,
        entry: BulkEntry,
        mode: BulkImportMode,
        on_duplicate: BulkDuplicate,
        parent_job: Uuid,
        line: usize,
    ) -> Result<BulkEntryOutcome, AppError> {
        if entry.title.trim().is_empty() {
            return Err(AppError::Validation("title は必須です".into()));
        }
        let mime_type = mime_for_filename(&entry.filename)?;
        if entry.bytes.is_empty() {
            return Err(AppError::Validation("空のファイルは登録できません".into()));
        }
        if entry.bytes.len() as u64 > MAX_UPLOAD_BYTES {
            return Err(AppError::Validation(
                "ファイルサイズが上限を超えています".into(),
            ));
        }
        let content_hash = engine_rag::sha256(&entry.bytes);

        let mut tx = self.store().begin_scoped(project).await?;
        if let Some((document_id, revision_id)) = self
            .store()
            .find_revision_by_hash(&mut tx, &content_hash)
            .await?
        {
            if on_duplicate == BulkDuplicate::Fail {
                return Err(AppError::Conflict(ErrorCode::ConflictAlreadyImported));
            }
            if mode != BulkImportMode::Reindex {
                store::commit(tx).await?;
                return Ok(BulkEntryOutcome::SkippedDuplicate);
            }
            let revision = self
                .store()
                .find_revision(&mut tx, revision_id)
                .await?
                .ok_or(AppError::NotFound(ErrorCode::NotFoundDocument))?;
            let (parser, version) = parser_for_mime(&revision.mime_type);
            let parse_run_id = self
                .store()
                .insert_parse_run(&mut tx, revision_id, parser, version)
                .await?;
            let input = json!({
                "project_id": project,
                "document_id": document_id,
                "revision_id": revision_id,
                "parse_run_id": parse_run_id,
                "object_key": revision.object_key,
                "mime_type": revision.mime_type,
                "file_size": revision.file_size,
            });
            self.store()
                .enqueue_job(
                    &mut tx,
                    &store::NewJob {
                        project_id: project.0,
                        kind: "DOCUMENT_PARSE".into(),
                        idempotency_key: format!("bulk:{parent_job}:{line}:parse"),
                        input,
                    },
                )
                .await?;
            store::commit(tx).await?;
            return Ok(BulkEntryOutcome::Reindexed {
                document_id,
                revision_id,
            });
        }

        let object_key = self.new_object_key(project, &entry.filename);
        self.put_object(&object_key, &entry.bytes).await?;
        let document_id = DocumentId::new();
        let revision_id = RevisionId::new();
        let now = Utc::now();
        self.store()
            .insert_document(
                &mut tx,
                &store::NewDocument {
                    id: document_id.0,
                    project_id: project.0,
                    doc_type: entry.doc_type.as_str().to_string(),
                    title: engine_rag::normalize_nfc(entry.title.trim()),
                    business_domain: entry.business_domain,
                    acl_groups: entry.acl_groups,
                },
            )
            .await?;
        self.store()
            .insert_revision(
                &mut tx,
                &store::NewRevision {
                    id: revision_id.0,
                    document_id: document_id.0,
                    source_revision: entry.source_revision,
                    content_hash: content_hash.clone(),
                    object_key: object_key.clone(),
                    file_size: entry.bytes.len() as i64,
                    mime_type: mime_type.clone(),
                    acquired_at: now,
                },
            )
            .await?;
        self.store()
            .set_current_revision(&mut tx, document_id.0, revision_id.0)
            .await?;

        let (parser, version) = parser_for_mime(&mime_type);
        let parse_run_id = self
            .store()
            .insert_parse_run(&mut tx, revision_id.0, parser, version)
            .await?;
        let input = json!({
            "project_id": project,
            "document_id": document_id,
            "revision_id": revision_id,
            "parse_run_id": parse_run_id,
            "object_key": object_key,
            "mime_type": mime_type,
            "file_size": entry.bytes.len(),
        });
        let child = self
            .store()
            .enqueue_job(
                &mut tx,
                &store::NewJob {
                    project_id: project.0,
                    kind: "DOCUMENT_PARSE".into(),
                    idempotency_key: format!("bulk:{parent_job}:{line}:parse"),
                    input,
                },
            )
            .await?;
        self.store()
            .append_audit(
                &mut tx,
                &store::AuditDraft {
                    project_id: Some(project.0),
                    actor_type: "SYSTEM".into(),
                    actor_id: "systivra-worker".into(),
                    action: "document.bulk_import.registered".into(),
                    target_type: Some("document".into()),
                    target_id: Some(document_id.to_string()),
                    input_digest: Some(content_hash),
                    result: "QUEUED".into(),
                    detail: json!({
                        "filename": entry.filename,
                        "line": line,
                        "job_id": child.job.id,
                    }),
                    trace_id: None,
                },
            )
            .await?;
        store::commit(tx).await?;

        Ok(BulkEntryOutcome::Queued {
            document_id: document_id.0,
            revision_id: revision_id.0,
        })
    }
}

#[derive(Debug, Clone)]
pub struct BulkEntry {
    pub doc_type: DocType,
    pub title: String,
    pub business_domain: Option<String>,
    pub acl_groups: Vec<String>,
    pub source_revision: Option<String>,
    pub filename: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BulkEntryOutcome {
    Queued {
        document_id: Uuid,
        revision_id: Uuid,
    },
    Reindexed {
        document_id: Uuid,
        revision_id: Uuid,
    },
    SkippedDuplicate,
}

fn mime_for_filename(filename: &str) -> Result<String, AppError> {
    let lower = filename.to_ascii_lowercase();
    let mime = if lower.ends_with(".pdf") {
        PDF_MIME
    } else if lower.ends_with(".xlsx") {
        XLSX_MIME
    } else if lower.ends_with(".xlsm") {
        XLSM_MIME
    } else if lower.ends_with(".docx") {
        DOCX_MIME
    } else if lower.ends_with(".pptx") {
        PPTX_MIME
    } else {
        return Err(AppError::Validation(
            "PDF / XLSX / DOCX / PPTX のみ取り込めます".into(),
        ));
    };
    Ok(mime.to_string())
}

fn parser_for_mime(mime: &str) -> (&'static str, &'static str) {
    if mime.eq_ignore_ascii_case(XLSX_MIME) || mime.eq_ignore_ascii_case(XLSM_MIME) {
        ("xlsx-native", "0.1.0")
    } else if mime.eq_ignore_ascii_case(DOCX_MIME) {
        ("docx-native", "0.1.0")
    } else if mime.eq_ignore_ascii_case(PPTX_MIME) {
        ("pptx-native", "0.1.0")
    } else {
        ("in-process-pdf", "0.1.0")
    }
}

pub(crate) fn parse_doc_type(value: &str) -> DocType {
    DocType::ALL
        .into_iter()
        .find(|d| d.as_str() == value)
        .unwrap_or(DocType::Other)
}

fn encode_cursor(created_at: DateTime<Utc>, id: Uuid) -> Result<String, AppError> {
    let raw = json!({"t": created_at.to_rfc3339(), "id": id.to_string()}).to_string();
    Ok(URL_SAFE_NO_PAD.encode(raw))
}

fn decode_cursor(cursor: &str) -> Result<(Option<DateTime<Utc>>, Option<Uuid>), AppError> {
    let raw = URL_SAFE_NO_PAD
        .decode(cursor)
        .map_err(|_| AppError::Validation("cursor が不正です".into()))?;
    let value: Value = serde_json::from_slice(&raw)
        .map_err(|_| AppError::Validation("cursor が不正です".into()))?;
    let time = value
        .get("t")
        .and_then(Value::as_str)
        .and_then(|v| DateTime::parse_from_rfc3339(v).ok())
        .map(|v| v.with_timezone(&Utc))
        .ok_or_else(|| AppError::Validation("cursor が不正です".into()))?;
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .and_then(|v| v.parse::<Uuid>().ok())
        .ok_or_else(|| AppError::Validation("cursor が不正です".into()))?;
    Ok((Some(time), Some(id)))
}
