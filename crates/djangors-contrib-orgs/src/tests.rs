use super::*;
use djangors_auth::hash_password;
use djangors_core::AppState;

async fn setup_db() -> Database {
    let config = djangors_db::DatabaseConfig::new("sqlite::memory:")
        .max_connections(1)
        .min_connections(1);
    let db = Database::connect(&config).await.unwrap();
    let auto_pk = db.dialect().auto_pk_type();
    let ts_type = db.dialect().timestamp_type();
    let create_user_sql = format!(
        "CREATE TABLE auth_user (
            id {auto_pk},
            username VARCHAR(150) NOT NULL UNIQUE,
            email VARCHAR(254) NOT NULL,
            password TEXT NOT NULL,
            is_active BOOLEAN NOT NULL,
            is_staff BOOLEAN NOT NULL,
            is_superuser BOOLEAN NOT NULL,
            date_joined {ts_type} NOT NULL,
            last_login {ts_type}
        )"
    );
    db.conn().execute(&create_user_sql, &[]).await.unwrap();
    create_orgs_tables(&db).await.unwrap();
    db
}

#[tokio::test]
async fn test_org_creation_permissions_and_last_owner_guard() {
    let db = setup_db().await;
    let owner = User {
        id: 0,
        username: "owner".to_string(),
        email: "owner@example.com".to_string(),
        password: hash_password("StrongPass!987").unwrap(),
        is_active: true,
        is_staff: false,
        is_superuser: false,
        date_joined: Utc::now(),
        last_login: None,
    };
    let owner = owner.save(&db).await.unwrap();

    let (org, membership) = create_organization(&db, "Acme Corp", "acme", owner.id, None)
        .await
        .unwrap();
    assert_eq!(membership.role, ROLE_OWNER);

    // Last owner cannot be removed or demoted
    assert!(matches!(
        remove_member(&db, org.id, owner.id).await,
        Err(OrgError::LastOwner)
    ));
    assert!(matches!(
        update_member_role(&db, org.id, owner.id, ROLE_ADMIN).await,
        Err(OrgError::LastOwner)
    ));

    // Role permissions check
    let perms = OrgPermissions::new()
        .role(ROLE_ADMIN, &["members.invite", "billing.view"])
        .role(ROLE_MEMBER, &["projects.read"]);
    assert!(perms.has_permission(ROLE_OWNER, "anything"));
    assert!(perms.has_permission(ROLE_ADMIN, "members.invite"));
    assert!(!perms.has_permission(ROLE_MEMBER, "members.invite"));

    // Invite a new user into the organization
    let mailer = djangors_mail::InMemoryBackend::default();
    let (_inv, token) = invite_to_org(
        &db,
        &mailer,
        owner.id,
        org.id,
        "member@example.com",
        ROLE_ADMIN,
        "https://example.com",
        Duration::from_secs(3600),
    )
    .await
    .unwrap();

    let (new_user, org_member) = accept_org_invitation(&db, &token, Some("StrongPass!987"), None)
        .await
        .unwrap();
    let org_member = org_member.unwrap();
    assert_eq!(org_member.role, ROLE_ADMIN);

    let m = require_org_permission(&db, &perms, org.id, new_user.id, "members.invite")
        .await
        .unwrap();
    assert_eq!(m.user.id, new_user.id);

    // CurrentOrg extractor via X-Organization header
    let state = AppState::new().insert(db.clone());
    let mut headers = hyper::HeaderMap::new();
    headers.insert("x-organization", "acme".parse().unwrap());
    let req = Request::new(
        hyper::Method::GET,
        "/orgs".parse().unwrap(),
        headers,
        Default::default(),
    )
    .with_state(state);
    let CurrentOrg(resolved) = CurrentOrg::from_request(&req).await.unwrap();
    assert_eq!(resolved.id, org.id);
}
