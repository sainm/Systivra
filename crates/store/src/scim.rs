//! SCIM 2.0 と IdP グループ→ロール同期の SQL。
//!
//! `external_group` / `external_group_member` は認証・認可で参照するため
//! RLS の対象外。単一プロジェクト前提なので、ロールは最初の有効プロジェクトへ付与する。

use domain::{ProjectId, Role, UserId};
use sqlx::Row;
use uuid::Uuid;

use crate::pool::{Store, StoreError, Tx};

#[derive(Debug, Clone)]
pub struct ScimUserRow {
    pub id: UserId,
    pub login_id: String,
    pub display_name: String,
    pub email: Option<String>,
    pub is_active: bool,
}

#[derive(Debug, Clone)]
pub struct ScimGroupRow {
    pub id: Uuid,
    pub provider: String,
    pub external_id: String,
    pub display_name: String,
    pub role: Role,
    pub members: Vec<ScimUserRow>,
}

#[derive(Debug, Clone)]
pub struct MemberRow {
    pub user_id: UserId,
    pub login_id: String,
    pub display_name: String,
    pub email: Option<String>,
    pub is_active: bool,
    pub role: Role,
}

fn user_row(row: &sqlx::postgres::PgRow) -> ScimUserRow {
    ScimUserRow {
        id: UserId(row.get("id")),
        login_id: row.get("login_id"),
        display_name: row.get("display_name"),
        email: row.get("email"),
        is_active: row.get("is_active"),
    }
}

impl Store {
    pub async fn list_scim_users(&self, tx: &mut Tx<'_>) -> Result<Vec<ScimUserRow>, StoreError> {
        let rows = sqlx::query(
            "SELECT id, login_id, display_name, email, is_active
               FROM core.app_user ORDER BY login_id",
        )
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows.iter().map(user_row).collect())
    }

    pub async fn find_scim_user(
        &self,
        tx: &mut Tx<'_>,
        id: UserId,
    ) -> Result<Option<ScimUserRow>, StoreError> {
        let row = sqlx::query(
            "SELECT id, login_id, display_name, email, is_active
               FROM core.app_user WHERE id = $1",
        )
        .bind(id.0)
        .fetch_optional(&mut **tx)
        .await?;
        Ok(row.as_ref().map(user_row))
    }

    pub async fn create_scim_user(
        &self,
        tx: &mut Tx<'_>,
        login_id: &str,
        display_name: &str,
        email: Option<&str>,
        active: bool,
    ) -> Result<ScimUserRow, StoreError> {
        let row = sqlx::query(
            "INSERT INTO core.app_user (login_id, display_name, email, is_active)
             VALUES ($1, $2, $3, $4)
             RETURNING id, login_id, display_name, email, is_active",
        )
        .bind(login_id)
        .bind(display_name)
        .bind(email)
        .bind(active)
        .fetch_one(&mut **tx)
        .await?;
        Ok(user_row(&row))
    }

    pub async fn set_user_active(
        &self,
        tx: &mut Tx<'_>,
        id: UserId,
        active: bool,
    ) -> Result<bool, StoreError> {
        let result = sqlx::query("UPDATE core.app_user SET is_active = $2 WHERE id = $1")
            .bind(id.0)
            .bind(active)
            .execute(&mut **tx)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn list_scim_groups(&self, tx: &mut Tx<'_>) -> Result<Vec<ScimGroupRow>, StoreError> {
        let groups = sqlx::query(
            "SELECT id, provider, external_id, display_name, role
               FROM core.external_group ORDER BY display_name",
        )
        .fetch_all(&mut **tx)
        .await?;
        let mut result = Vec::with_capacity(groups.len());
        for group in groups {
            let id: Uuid = group.get("id");
            let members = sqlx::query(
                "SELECT u.id, u.login_id, u.display_name, u.email, u.is_active
                   FROM core.external_group_member m
                   JOIN core.app_user u ON u.id = m.user_id
                  WHERE m.group_id = $1
                  ORDER BY u.login_id",
            )
            .bind(id)
            .fetch_all(&mut **tx)
            .await?;
            let Some(role) = Role::parse(group.get::<String, _>("role").as_str()) else {
                continue;
            };
            result.push(ScimGroupRow {
                id,
                provider: group.get("provider"),
                external_id: group.get("external_id"),
                display_name: group.get("display_name"),
                role,
                members: members.iter().map(user_row).collect(),
            });
        }
        Ok(result)
    }

    pub async fn upsert_scim_group(
        &self,
        tx: &mut Tx<'_>,
        provider: &str,
        external_id: &str,
        display_name: &str,
        role: Role,
        members: &[UserId],
    ) -> Result<Uuid, StoreError> {
        let id: Uuid = sqlx::query_scalar(
            "INSERT INTO core.external_group (provider, external_id, display_name, role)
             VALUES ($1, $2, $3, $4)
             ON CONFLICT (provider, external_id)
             DO UPDATE SET display_name = EXCLUDED.display_name,
                           role = EXCLUDED.role,
                           updated_at = now()
             RETURNING id",
        )
        .bind(provider)
        .bind(external_id)
        .bind(display_name)
        .bind(role.as_str())
        .fetch_one(&mut **tx)
        .await?;

        sqlx::query("DELETE FROM core.external_group_member WHERE group_id = $1")
            .bind(id)
            .execute(&mut **tx)
            .await?;
        for member in members {
            sqlx::query(
                "INSERT INTO core.external_group_member (group_id, user_id)
                 VALUES ($1, $2) ON CONFLICT DO NOTHING",
            )
            .bind(id)
            .bind(member.0)
            .execute(&mut **tx)
            .await?;
        }
        Ok(id)
    }

    pub async fn delete_scim_group(&self, tx: &mut Tx<'_>, id: Uuid) -> Result<bool, StoreError> {
        let result = sqlx::query("DELETE FROM core.external_group WHERE id = $1")
            .bind(id)
            .execute(&mut **tx)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// IdP のグループ名に対応するロール。`provider='*'` は全 IdP 共通。
    pub async fn group_roles(
        &self,
        tx: &mut Tx<'_>,
        provider: &str,
        groups: &[String],
    ) -> Result<Vec<Role>, StoreError> {
        let rows = sqlx::query(
            "SELECT role FROM core.external_group
              WHERE display_name = ANY($2)
                AND (provider = $1 OR provider = '*')",
        )
        .bind(provider)
        .bind(groups)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| Role::parse(row.get::<String, _>("role").as_str()))
            .collect())
    }

    pub async fn first_project_id(&self, tx: &mut Tx<'_>) -> Result<Option<ProjectId>, StoreError> {
        let id: Option<Uuid> = sqlx::query_scalar("SELECT core.first_project()")
            .fetch_optional(&mut **tx)
            .await?;
        Ok(id.map(ProjectId))
    }

    pub async fn upsert_member_role(
        &self,
        tx: &mut Tx<'_>,
        project: ProjectId,
        user: UserId,
        role: Role,
    ) -> Result<(), StoreError> {
        sqlx::query("SELECT set_config('app.project_id', $1, true)")
            .bind(project.0.to_string())
            .execute(&mut **tx)
            .await?;
        sqlx::query(
            "INSERT INTO core.project_member (project_id, user_id, role)
             VALUES ($1, $2, $3)
             ON CONFLICT (project_id, user_id) DO UPDATE SET role = EXCLUDED.role",
        )
        .bind(project.0)
        .bind(user.0)
        .bind(role.as_str())
        .execute(&mut **tx)
        .await?;
        Ok(())
    }

    pub async fn list_members(
        &self,
        tx: &mut Tx<'_>,
        project: ProjectId,
    ) -> Result<Vec<MemberRow>, StoreError> {
        sqlx::query("SELECT set_config('app.project_id', $1, true)")
            .bind(project.0.to_string())
            .execute(&mut **tx)
            .await?;
        let rows = sqlx::query(
            "SELECT u.id, u.login_id, u.display_name, u.email, u.is_active, m.role
               FROM core.project_member m
               JOIN core.app_user u ON u.id = m.user_id
              WHERE m.project_id = $1
              ORDER BY u.login_id",
        )
        .bind(project.0)
        .fetch_all(&mut **tx)
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let role = Role::parse(row.get::<String, _>("role").as_str())?;
                Some(MemberRow {
                    user_id: UserId(row.get("id")),
                    login_id: row.get("login_id"),
                    display_name: row.get("display_name"),
                    email: row.get("email"),
                    is_active: row.get("is_active"),
                    role,
                })
            })
            .collect())
    }
}
