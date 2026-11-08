//! SCIM 2.0 と IdP グループ→ロール同期のユースケース。

use domain::{ProjectId, Role, UserId};
use uuid::Uuid;

use crate::auth::AuthError;
use crate::App;

fn rank(role: Role) -> u8 {
    match role {
        Role::Viewer => 0,
        Role::Tester => 1,
        Role::Developer => 2,
        Role::Architect => 3,
        Role::Owner => 4,
    }
}

impl App {
    pub async fn list_scim_users(&self) -> Result<Vec<store::ScimUserRow>, AuthError> {
        let mut tx = self.store().begin_unscoped().await?;
        let users = self.store().list_scim_users(&mut tx).await?;
        store::commit(tx).await?;
        Ok(users)
    }

    pub async fn find_scim_user(
        &self,
        id: UserId,
    ) -> Result<Option<store::ScimUserRow>, AuthError> {
        let mut tx = self.store().begin_unscoped().await?;
        let user = self.store().find_scim_user(&mut tx, id).await?;
        store::commit(tx).await?;
        Ok(user)
    }

    pub async fn create_scim_user(
        &self,
        login_id: &str,
        display_name: &str,
        email: Option<&str>,
        active: bool,
    ) -> Result<store::ScimUserRow, AuthError> {
        let mut tx = self.store().begin_unscoped().await?;
        let user = self
            .store()
            .create_scim_user(&mut tx, login_id, display_name, email, active)
            .await?;
        store::commit(tx).await?;
        Ok(user)
    }

    pub async fn set_user_active(&self, id: UserId, active: bool) -> Result<bool, AuthError> {
        let mut tx = self.store().begin_unscoped().await?;
        let changed = self.store().set_user_active(&mut tx, id, active).await?;
        store::commit(tx).await?;
        Ok(changed)
    }

    pub async fn list_scim_groups(&self) -> Result<Vec<store::ScimGroupRow>, AuthError> {
        let mut tx = self.store().begin_unscoped().await?;
        let groups = self.store().list_scim_groups(&mut tx).await?;
        store::commit(tx).await?;
        Ok(groups)
    }

    pub async fn upsert_scim_group(
        &self,
        provider: &str,
        external_id: &str,
        display_name: &str,
        role: Role,
        members: &[UserId],
    ) -> Result<Uuid, AuthError> {
        let mut tx = self.store().begin_unscoped().await?;
        let id = self
            .store()
            .upsert_scim_group(&mut tx, provider, external_id, display_name, role, members)
            .await?;
        if let Some(project) = self.store().first_project_id(&mut tx).await? {
            for member in members {
                self.store()
                    .upsert_member_role(&mut tx, project, *member, role)
                    .await?;
            }
        }
        store::commit(tx).await?;
        Ok(id)
    }

    pub async fn delete_scim_group(&self, id: Uuid) -> Result<bool, AuthError> {
        let mut tx = self.store().begin_unscoped().await?;
        let deleted = self.store().delete_scim_group(&mut tx, id).await?;
        store::commit(tx).await?;
        Ok(deleted)
    }

    pub async fn list_members(
        &self,
        project: ProjectId,
    ) -> Result<Vec<store::MemberRow>, AuthError> {
        let mut tx = self.store().begin_unscoped().await?;
        let members = self.store().list_members(&mut tx, project).await?;
        store::commit(tx).await?;
        Ok(members)
    }

    pub async fn set_member_role(
        &self,
        project: ProjectId,
        user: UserId,
        role: Role,
    ) -> Result<(), AuthError> {
        let mut tx = self.store().begin_unscoped().await?;
        self.store()
            .upsert_member_role(&mut tx, project, user, role)
            .await?;
        store::commit(tx).await?;
        Ok(())
    }

    /// IdP の groups claim を `external_group` に照合し、最も強いロールを付与する。
    /// 対応するグループが無ければ何もしない（既存ロールを剥がさない）。
    pub async fn sync_group_roles(
        &self,
        user: UserId,
        provider: &str,
        groups: &[String],
    ) -> Result<Option<Role>, AuthError> {
        if groups.is_empty() {
            return Ok(None);
        }
        let mut tx = self.store().begin_unscoped().await?;
        let roles = self.store().group_roles(&mut tx, provider, groups).await?;
        let Some(best) = roles.into_iter().max_by_key(|role| rank(*role)) else {
            store::commit(tx).await?;
            return Ok(None);
        };
        let Some(project) = self.store().first_project_id(&mut tx).await? else {
            store::commit(tx).await?;
            return Ok(None);
        };
        self.store()
            .upsert_member_role(&mut tx, project, user, best)
            .await?;
        store::commit(tx).await?;
        Ok(Some(best))
    }
}
