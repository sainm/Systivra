use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use contracts::{
    DocumentChunkListResponse, DocumentChunkQuery, DocumentChunkSummary, ErrorCode, TreeNode,
    TreeQuery, TreeResponse,
};
use domain::{DocumentId, Permission};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::auth::Authenticated;
use crate::error::AppError;
use crate::App;

impl App {
    pub async fn document_tree(
        &self,
        auth: &Authenticated,
        document_id: DocumentId,
        query: TreeQuery,
    ) -> Result<TreeResponse, AppError> {
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
        let revision_id = match query.revision_id {
            Some(revision) => revision.0,
            None => {
                self.store()
                    .find_current_revision(&mut tx, document_id.0)
                    .await?
                    .ok_or(AppError::NotFound(ErrorCode::NotFoundDocument))?
                    .id
            }
        };
        let parent = match query.parent.as_deref() {
            None | Some("") | Some("ROOT") => None,
            Some(raw) => Some(
                raw.parse::<Uuid>()
                    .map_err(|_| AppError::Validation("parent が不正です".into()))?,
            ),
        };
        let after = query
            .cursor
            .as_deref()
            .map(decode_cursor)
            .transpose()?
            .map(|value| {
                (
                    value.get("o").and_then(Value::as_i64).unwrap_or(-1) as i32,
                    value
                        .get("i")
                        .and_then(Value::as_str)
                        .and_then(|raw| raw.parse::<Uuid>().ok())
                        .unwrap_or_else(Uuid::nil),
                )
            });
        let limit = query.limit.unwrap_or(100).clamp(1, 200);
        let mut rows = self
            .store()
            .tree_nodes(&mut tx, revision_id, parent, after, limit + 1)
            .await?;
        let truncated = rows.len() as i64 > limit;
        if truncated {
            rows.truncate(limit as usize);
        }
        let next_cursor = if truncated {
            rows.last()
                .map(|row| encode_cursor(&json!({"o": row.ordinal, "i": row.id})))
        } else {
            None
        };
        let nodes = rows
            .into_iter()
            .map(|row| TreeNode {
                id: row.id,
                kind: row.node_type.clone(),
                name: node_name(&row.node_type, &row.locator, row.text.as_deref()),
                locator: row.locator,
                child_count: row.child_count,
            })
            .collect();
        store::commit(tx).await?;
        Ok(TreeResponse {
            nodes,
            next_cursor,
            truncated,
        })
    }

    pub async fn document_chunks(
        &self,
        auth: &Authenticated,
        document_id: DocumentId,
        query: DocumentChunkQuery,
    ) -> Result<DocumentChunkListResponse, AppError> {
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
        let revision_id = match query.revision_id {
            Some(revision) => revision.0,
            None => {
                self.store()
                    .find_current_revision(&mut tx, document_id.0)
                    .await?
                    .ok_or(AppError::NotFound(ErrorCode::NotFoundDocument))?
                    .id
            }
        };
        let after = query
            .cursor
            .as_deref()
            .map(decode_cursor)
            .transpose()?
            .and_then(|value| value.get("o").and_then(Value::as_i64))
            .map(|value| value as i32);
        let limit = query.limit.unwrap_or(50).clamp(1, 200);
        let mut rows = self
            .store()
            .document_chunks(
                &mut tx,
                project.0,
                document_id.0,
                revision_id,
                after,
                limit + 1,
            )
            .await?;
        let has_more = rows.len() as i64 > limit;
        if has_more {
            rows.truncate(limit as usize);
        }
        let next_cursor = if has_more {
            rows.last()
                .map(|row| encode_cursor(&json!({"o": row.ordinal})))
        } else {
            None
        };
        let items = rows
            .into_iter()
            .map(|row| DocumentChunkSummary {
                id: row.id,
                chunk_type: row.chunk_type,
                ordinal: row.ordinal,
                text: row.text,
                locator: row.locator,
                token_count: row.token_count,
            })
            .collect();
        store::commit(tx).await?;
        Ok(DocumentChunkListResponse {
            items,
            next_cursor,
            has_more,
        })
    }
}

fn encode_cursor(value: &Value) -> String {
    URL_SAFE_NO_PAD.encode(value.to_string())
}

fn decode_cursor(raw: &str) -> Result<Value, AppError> {
    let bytes = URL_SAFE_NO_PAD
        .decode(raw)
        .map_err(|_| AppError::Validation("cursor が不正です".into()))?;
    serde_json::from_slice(&bytes).map_err(|_| AppError::Validation("cursor が不正です".into()))
}

fn node_name(kind: &str, locator: &Value, text: Option<&str>) -> Option<String> {
    if let Some(text) = text.filter(|value| !value.trim().is_empty()) {
        return Some(text.to_string());
    }
    let string = |key: &str| {
        locator
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
    };
    match kind {
        "SHEET" => string("sheet"),
        "REGION" => match (string("sheet"), string("range")) {
            (Some(sheet), Some(range)) => Some(format!("{sheet} / {range}")),
            _ => string("range"),
        },
        "SLIDE" => string("title").or_else(|| {
            locator
                .get("slide")
                .and_then(Value::as_i64)
                .map(|slide| format!("Slide {slide}"))
        }),
        "TABLE" => string("table").map(|table| format!("表 {table}")),
        "FIGURE" => string("anchor_cell")
            .or_else(|| string("name"))
            .or_else(|| Some("図".into())),
        _ => locator
            .get("section_path")
            .and_then(Value::as_array)
            .and_then(|path| path.iter().filter_map(Value::as_str).next_back())
            .map(str::to_string),
    }
}
