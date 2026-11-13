#![deny(missing_docs)]
//! Organizations, teams, role-based permissions, and membership management for Djangors.

use chrono::{DateTime, Utc};
use djangors_auth::{accept_invitation, invite, Auth, AuthError, Invitation, User};
use djangors_core::extract::FromRequest;
use djangors_core::path_params::PathParams;
use djangors_core::{DjangorsError, Request, StatusCode};
use djangors_db::Database;
use djangors_macros::Model;
use djangors_mail::MailBackend;
use djangors_orm::{ForeignKey, Model as _};
use djangors_sessions::Session;
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use thiserror::Error;

/// Session key used to store the active organization slug.
pub const SESSION_ORG_SLUG_KEY: &str = "_current_org_slug";

/// Built-in organization owner role.
pub const ROLE_OWNER: &str = "owner";
/// Built-in organization admin role.
pub const ROLE_ADMIN: &str = "admin";
/// Built-in organization member role.
pub const ROLE_MEMBER: &str = "member";
/// Built-in organization viewer role.
pub const ROLE_VIEWER: &str = "viewer";

/// Errors produced by organization and team operations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum OrgError {
    /// Database / ORM error.
    #[error("database error: {0}")]
    Database(#[from] djangors_orm::OrmError),
    /// Authentication / invitation error.
    #[error("auth error: {0}")]
    Auth(#[from] AuthError),
    /// Operation would leave an organization without an owner.
    #[error("organization must retain at least one owner")]
    LastOwner,
    /// Organization or membership not found.
    #[error("{0}")]
    NotFound(String),
}

impl From<OrgError> for DjangorsError {
    fn from(err: OrgError) -> Self {
        match err {
            OrgError::LastOwner => {
                DjangorsError::api(StatusCode::BAD_REQUEST, "last_owner", err.to_string())
            }
            OrgError::NotFound(msg) => DjangorsError::api(StatusCode::NOT_FOUND, "not_found", msg),
            OrgError::Auth(e) => e.into(),
            OrgError::Database(e) => DjangorsError::Internal(e.to_string()),
        }
    }
}

/// Organization model.
#[derive(Model, Debug, Clone)]
#[djangors(app = "djangors_contrib_orgs", table_name = "orgs_organization")]
pub struct Organization {
    /// Primary key identifier.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// Human-readable organization name.
    #[djangors(max_length = 200)]
    pub name: String,
    /// Unique URL slug.
    #[djangors(max_length = 100, unique)]
    pub slug: String,
    /// Creation timestamp (UTC).
    pub created_at: DateTime<Utc>,
    /// Optional linked tenant ID (for multi-tenant apps).
    pub tenant_id: Option<i64>,
}

/// Organization membership linking a [`User`] to an [`Organization`] with a role.
#[derive(Model, Debug, Clone)]
#[djangors(app = "djangors_contrib_orgs", table_name = "orgs_membership")]
pub struct Membership {
    /// Primary key identifier.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// Organization foreign key.
    pub organization: ForeignKey<Organization>,
    /// User foreign key.
    pub user: ForeignKey<User>,
    /// Role name (e.g. `"owner"`, `"admin"`, `"member"`, `"viewer"`).
    #[djangors(max_length = 64)]
    pub role: String,
    /// Timestamp when the user joined (UTC).
    pub joined_at: DateTime<Utc>,
}

/// Team within an [`Organization`].
#[derive(Model, Debug, Clone)]
#[djangors(app = "djangors_contrib_orgs", table_name = "orgs_team")]
pub struct Team {
    /// Primary key identifier.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// Owning organization.
    pub organization: ForeignKey<Organization>,
    /// Team name.
    #[djangors(max_length = 200)]
    pub name: String,
    /// Team slug (unique within the organization).
    #[djangors(max_length = 100)]
    pub slug: String,
    /// Creation timestamp (UTC).
    pub created_at: DateTime<Utc>,
}

/// Membership linking a [`User`] to a [`Team`].
#[derive(Model, Debug, Clone)]
#[djangors(app = "djangors_contrib_orgs", table_name = "orgs_team_membership")]
pub struct TeamMembership {
    /// Primary key identifier.
    #[djangors(primary_key, auto)]
    pub id: i64,
    /// Team foreign key.
    pub team: ForeignKey<Team>,
    /// User foreign key.
    pub user: ForeignKey<User>,
    /// Timestamp when the user joined the team (UTC).
    pub joined_at: DateTime<Utc>,
}

/// Ensures the organization, membership, team, and team-membership tables exist.
pub async fn create_orgs_tables(db: &Database) -> Result<(), OrgError> {
    let mut conn = db.conn();
    let dialect = conn.dialect();
    let pk = dialect.auto_pk_type();
    let bigint = dialect.bigint_type();
    let ts = dialect.timestamp_type();

    let stmts = [
        format!(
            "CREATE TABLE IF NOT EXISTS orgs_organization (
                id {pk},
                name VARCHAR(200) NOT NULL,
                slug VARCHAR(100) NOT NULL UNIQUE,
                created_at {ts} NOT NULL,
                tenant_id {bigint}
            )"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS orgs_membership (
                id {pk},
                organization {bigint} NOT NULL REFERENCES orgs_organization(id) ON DELETE CASCADE,
                \"user\" {bigint} NOT NULL REFERENCES auth_user(id) ON DELETE CASCADE,
                role VARCHAR(64) NOT NULL,
                joined_at {ts} NOT NULL,
                UNIQUE (organization, \"user\")
            )"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS orgs_team (
                id {pk},
                organization {bigint} NOT NULL REFERENCES orgs_organization(id) ON DELETE CASCADE,
                name VARCHAR(200) NOT NULL,
                slug VARCHAR(100) NOT NULL,
                created_at {ts} NOT NULL,
                UNIQUE (organization, slug)
            )"
        ),
        format!(
            "CREATE TABLE IF NOT EXISTS orgs_team_membership (
                id {pk},
                team {bigint} NOT NULL REFERENCES orgs_team(id) ON DELETE CASCADE,
                \"user\" {bigint} NOT NULL REFERENCES auth_user(id) ON DELETE CASCADE,
                joined_at {ts} NOT NULL,
                UNIQUE (team, \"user\")
            )"
        ),
    ];

    for sql in stmts {
        conn.execute(&sql, &[])
            .await
            .map_err(djangors_orm::OrmError::Query)?;
    }
    Ok(())
}

/// Code-configured mapping from role names to permission strings.
#[derive(Debug, Clone, Default)]
pub struct OrgPermissions {
    roles: HashMap<String, HashSet<String>>,
}

impl OrgPermissions {
    /// Creates an empty [`OrgPermissions`] registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Grants `perms` to `role`.
    pub fn role(mut self, role: &str, perms: &[&str]) -> Self {
        let entry = self.roles.entry(role.to_string()).or_default();
        for p in perms {
            entry.insert((*p).to_string());
        }
        self
    }

    /// Returns `true` if `role` has `perm` (or if `role` is `"owner"`).
    pub fn has_permission(&self, role: &str, perm: &str) -> bool {
        if role == ROLE_OWNER {
            return true;
        }
        self.roles
            .get(role)
            .is_some_and(|set| set.contains(perm) || set.contains("*"))
    }
}

/// Extractor resolving the active [`Organization`] from:
/// 1. URL path parameter `org_slug`
/// 2. `X-Organization` request header
/// 3. `_current_org_slug` session key
#[derive(Debug, Clone)]
pub struct CurrentOrg(pub Organization);

#[async_trait::async_trait]
impl FromRequest for CurrentOrg {
    async fn from_request(req: &Request) -> Result<Self, DjangorsError> {
        let slug = req
            .ext::<PathParams>()
            .and_then(|p| p.get("org_slug"))
            .map(str::to_string)
            .or_else(|| {
                req.header("x-organization")
                    .and_then(|v| v.to_str().ok())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            })
            .or_else(|| {
                req.ext::<Session>()
                    .and_then(|s| s.get::<String>(SESSION_ORG_SLUG_KEY))
            })
            .ok_or_else(|| DjangorsError::BadRequest("missing organization slug".to_string()))?;

        let db = req
            .state::<Database>()
            .ok_or_else(|| DjangorsError::Internal("database state absent".to_string()))?;

        let org = Organization::objects()
            .filter(djangors_orm::q!(slug = slug.as_str()))
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .get(db)
            .await
            .map_err(|_| {
                DjangorsError::api(
                    StatusCode::NOT_FOUND,
                    "organization_not_found",
                    format!("organization '{slug}' not found"),
                )
            })?;

        Ok(CurrentOrg(org))
    }
}

/// Extractor ensuring the authenticated user is a member of [`CurrentOrg`].
#[derive(Debug, Clone)]
pub struct IsOrgMember {
    /// Authenticated user.
    pub user: User,
    /// Active organization.
    pub org: Organization,
    /// User's membership record.
    pub membership: Membership,
}

#[async_trait::async_trait]
impl FromRequest for IsOrgMember {
    async fn from_request(req: &Request) -> Result<Self, DjangorsError> {
        let Auth(user) = Auth::<User>::from_request(req).await?;
        let CurrentOrg(org) = CurrentOrg::from_request(req).await?;
        let db = req
            .state::<Database>()
            .ok_or_else(|| DjangorsError::Internal("database state absent".to_string()))?;

        let membership = Membership::objects()
            .filter(djangors_orm::q!(organization = org.id, user = user.id))
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .all(db)
            .await
            .map_err(|e| DjangorsError::Internal(e.to_string()))?
            .into_iter()
            .next()
            .ok_or_else(|| {
                DjangorsError::Forbidden("user is not a member of this organization".to_string())
            })?;

        Ok(IsOrgMember {
            user,
            org,
            membership,
        })
    }
}

/// Checks that `user_id` holds `required_role` in `org_id`.
pub async fn require_org_role(
    db: &Database,
    org_id: i64,
    user_id: i64,
    required_role: &str,
) -> Result<Membership, DjangorsError> {
    let membership = Membership::objects()
        .filter(djangors_orm::q!(organization = org_id, user = user_id))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .all(db)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .into_iter()
        .next()
        .ok_or_else(|| DjangorsError::Forbidden("not an organization member".to_string()))?;

    if membership.role != required_role && membership.role != ROLE_OWNER {
        return Err(DjangorsError::Forbidden(format!(
            "requires organization role '{required_role}'"
        )));
    }
    Ok(membership)
}

/// Checks that `user_id`'s role in `org_id` grants `perm` according to `perms`.
pub async fn require_org_permission(
    db: &Database,
    perms: &OrgPermissions,
    org_id: i64,
    user_id: i64,
    perm: &str,
) -> Result<Membership, DjangorsError> {
    let membership = Membership::objects()
        .filter(djangors_orm::q!(organization = org_id, user = user_id))
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .all(db)
        .await
        .map_err(|e| DjangorsError::Internal(e.to_string()))?
        .into_iter()
        .next()
        .ok_or_else(|| DjangorsError::Forbidden("not an organization member".to_string()))?;

    if !perms.has_permission(&membership.role, perm) {
        return Err(DjangorsError::Forbidden(format!(
            "missing organization permission '{perm}'"
        )));
    }
    Ok(membership)
}

/// Permission helper descriptor for role checks (`HasOrgRole("admin")`).
#[derive(Debug, Clone)]
pub struct HasOrgRole(pub &'static str);

impl HasOrgRole {
    /// Verifies the request's authenticated user has this role in [`CurrentOrg`].
    pub async fn check(&self, req: &Request) -> Result<IsOrgMember, DjangorsError> {
        let member = IsOrgMember::from_request(req).await?;
        if member.membership.role != self.0 && member.membership.role != ROLE_OWNER {
            return Err(DjangorsError::Forbidden(format!(
                "requires organization role '{}'",
                self.0
            )));
        }
        Ok(member)
    }
}

/// Permission helper descriptor for permission checks (`HasOrgPermission("members.invite")`).
#[derive(Debug, Clone)]
pub struct HasOrgPermission(pub &'static str);

impl HasOrgPermission {
    /// Verifies the request's authenticated user has this permission via [`OrgPermissions`] in state.
    pub async fn check(&self, req: &Request) -> Result<IsOrgMember, DjangorsError> {
        let member = IsOrgMember::from_request(req).await?;
        let perms = req
            .state::<OrgPermissions>()
            .ok_or_else(|| DjangorsError::Internal("OrgPermissions state absent".to_string()))?;
        if !perms.has_permission(&member.membership.role, self.0) {
            return Err(DjangorsError::Forbidden(format!(
                "missing organization permission '{}'",
                self.0
            )));
        }
        Ok(member)
    }
}

/// Creates a new [`Organization`] and adds `owner_user_id` with role `"owner"`.
pub async fn create_organization(
    db: &Database,
    name: &str,
    slug: &str,
    owner_user_id: i64,
    tenant_id: Option<i64>,
) -> Result<(Organization, Membership), OrgError> {
    create_orgs_tables(db).await?;
    let org = Organization {
        id: 0,
        name: name.to_string(),
        slug: slug.to_string(),
        created_at: Utc::now(),
        tenant_id,
    };
    let org = org.save(db).await?;

    let membership = Membership {
        id: 0,
        organization: ForeignKey::new(org.id),
        user: ForeignKey::new(owner_user_id),
        role: ROLE_OWNER.to_string(),
        joined_at: Utc::now(),
    };
    let membership = membership.save(db).await?;

    Ok((org, membership))
}

async fn count_owners(db: &Database, org_id: i64) -> Result<usize, OrgError> {
    let owners = Membership::objects()
        .filter(djangors_orm::q!(organization = org_id, role = ROLE_OWNER))?
        .all(db)
        .await?;
    Ok(owners.len())
}

/// Removes `user_id` from `org_id`, refusing if it would leave the organization with no owners.
pub async fn remove_member(db: &Database, org_id: i64, user_id: i64) -> Result<(), OrgError> {
    let membership = Membership::objects()
        .filter(djangors_orm::q!(organization = org_id, user = user_id))?
        .all(db)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| OrgError::NotFound("membership not found".to_string()))?;

    if membership.role == ROLE_OWNER && count_owners(db, org_id).await? <= 1 {
        return Err(OrgError::LastOwner);
    }

    membership.delete(db).await?;
    Ok(())
}

/// Updates `user_id`'s role in `org_id`, refusing to demote the last remaining owner.
pub async fn update_member_role(
    db: &Database,
    org_id: i64,
    user_id: i64,
    new_role: &str,
) -> Result<Membership, OrgError> {
    let mut membership = Membership::objects()
        .filter(djangors_orm::q!(organization = org_id, user = user_id))?
        .all(db)
        .await?
        .into_iter()
        .next()
        .ok_or_else(|| OrgError::NotFound("membership not found".to_string()))?;

    if membership.role == ROLE_OWNER
        && new_role != ROLE_OWNER
        && count_owners(db, org_id).await? <= 1
    {
        return Err(OrgError::LastOwner);
    }

    membership.role = new_role.to_string();
    membership.update(db).await?;
    Ok(membership)
}

/// Sends an organization invitation using `djangors-auth::invite`.
#[allow(clippy::too_many_arguments)]
pub async fn invite_to_org(
    db: &Database,
    mailer: &dyn MailBackend,
    inviter_id: i64,
    org_id: i64,
    email: &str,
    role: &str,
    base_url: &str,
    ttl: Duration,
) -> Result<(Invitation, String), OrgError> {
    let res = invite(
        db,
        mailer,
        inviter_id,
        email,
        Some(org_id),
        role,
        base_url,
        ttl,
    )
    .await?;
    Ok(res)
}

/// Accepts an organization invitation and creates the [`Membership`] if `organization_id` is set.
pub async fn accept_org_invitation(
    db: &Database,
    token: &str,
    password: Option<&str>,
    current_user: Option<&User>,
) -> Result<(User, Option<Membership>), OrgError> {
    create_orgs_tables(db).await?;
    let (user, inv) = accept_invitation(db, token, password, current_user).await?;
    let membership = if let Some(org_id) = inv.organization_id {
        let m = Membership {
            id: 0,
            organization: ForeignKey::new(org_id),
            user: ForeignKey::new(user.id),
            role: inv.role,
            joined_at: Utc::now(),
        };
        let m = m.save(db).await?;
        Some(m)
    } else {
        None
    };
    Ok((user, membership))
}

/// Registers `Organization`, `Membership`, `Team`, and `TeamMembership` with `AdminSite`.
#[cfg(feature = "admin")]
pub fn register_admin(site: &djangors_admin::AdminSite) {
    site.register::<Organization>();
    site.register::<Membership>();
    site.register::<Team>();
    site.register::<TeamMembership>();
}

#[cfg(test)]
mod tests;
