# Organizations and Teams

`djangors-contrib-orgs` provides multi-user `Organization`, `Membership`, `Team`, and `TeamMembership` models along with role-based permissions and request extractors.

## Setup and Permissions

```rust,illustrative
use djangors::orgs::{create_organization, OrgPermissions, ROLE_ADMIN, ROLE_MEMBER};

# async fn example(db: &djangors::db::Database, owner_id: i64) -> Result<(), djangors::orgs::OrgError> {
let (org, _membership) = create_organization(db, "Acme", "acme", owner_id, None).await?;

let perms = OrgPermissions::new()
    .role(ROLE_ADMIN, &["members.invite", "teams.manage"])
    .role(ROLE_MEMBER, &["projects.read"]);
# let _ = (org, perms);
# Ok(())
# }
```

## Resolving the Active Organization

`CurrentOrg` resolves the organization slug in this order:

1. URL path parameter `{org_slug}`
2. `X-Organization` HTTP header
3. `_current_org_slug` session key

Pair it with `IsOrgMember`, `HasOrgRole`, and `HasOrgPermission` to guard routes. At least one `"owner"` must always remain in an organization (`remove_member` and `update_member_role` return `OrgError::LastOwner`).

## Relationship to Tenancy

An `Organization` can optionally reference a `tenant_id` (from `djangors-contrib-tenancy`) when each organization maps to an isolated tenant scope. Use `djangors-contrib-tenancy` for row/schema isolation and `djangors-contrib-orgs` for roles, teams, and invitations.
