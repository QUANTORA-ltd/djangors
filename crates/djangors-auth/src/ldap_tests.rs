use super::*;
use crate::{has_perm, GroupPermission, Permission};
use djangors_db::config::DatabaseConfig;
use std::collections::HashMap;

async fn setup_ldap_test_db() -> Database {
    let config = DatabaseConfig::new("sqlite::memory:")
        .max_connections(1)
        .min_connections(1);
    let db = Database::connect(&config).await.unwrap();
    let d = db.dialect();
    let pk = d.auto_pk_type();
    let ts = d.timestamp_type();
    let b = d.bool_type();

    db.conn()
        .execute(
            &format!(
                "CREATE TABLE auth_user (
                    id {pk},
                    username VARCHAR(150) NOT NULL UNIQUE,
                    email VARCHAR(254) NOT NULL,
                    password TEXT NOT NULL,
                    is_active {b} NOT NULL,
                    is_staff {b} NOT NULL,
                    is_superuser {b} NOT NULL,
                    date_joined {ts} NOT NULL,
                    last_login {ts}
                )"
            ),
            &[],
        )
        .await
        .unwrap();

    db.conn()
        .execute(
            &format!(
                "CREATE TABLE auth_group (
                    id {pk},
                    name VARCHAR(150) NOT NULL UNIQUE
                )"
            ),
            &[],
        )
        .await
        .unwrap();

    db.conn()
        .execute(
            &format!(
                "CREATE TABLE auth_permission (
                    id {pk},
                    codename VARCHAR(255) NOT NULL UNIQUE,
                    name VARCHAR(255) NOT NULL
                )"
            ),
            &[],
        )
        .await
        .unwrap();

    db.conn()
        .execute(
            &format!(
                "CREATE TABLE auth_user_groups (
                    id {pk},
                    \"user\" INTEGER NOT NULL,
                    \"group\" INTEGER NOT NULL
                )"
            ),
            &[],
        )
        .await
        .unwrap();

    db.conn()
        .execute(
            &format!(
                "CREATE TABLE auth_group_permissions (
                    id {pk},
                    \"group\" INTEGER NOT NULL,
                    permission INTEGER NOT NULL
                )"
            ),
            &[],
        )
        .await
        .unwrap();

    db.conn()
        .execute(
            &format!(
                "CREATE TABLE auth_user_permissions (
                    id {pk},
                    \"user\" INTEGER NOT NULL,
                    permission INTEGER NOT NULL
                )"
            ),
            &[],
        )
        .await
        .unwrap();

    db
}

fn sample_config() -> LdapConfig {
    LdapConfig::new(
        "ldaps://ldap.example.com:636",
        "cn=readonly,dc=example,dc=com",
        "super-secret-bind-pw",
        "ou=People,dc=example,dc=com",
    )
}

#[test]
fn escape_filter_value_blocks_rfc4515_injections() {
    assert_eq!(escape_filter_value("alice"), "alice");
    assert_eq!(
        escape_filter_value("admin)(|(objectClass=*))"),
        "admin\\29\\28|\\28objectClass=\\2a\\29\\29"
    );
    assert_eq!(
        escape_filter_value("*)(uid=*))(|(uid=*"),
        "\\2a\\29\\28uid=\\2a\\29\\29\\28|\\28uid=\\2a"
    );
    assert_eq!(escape_filter_value("a\\b\0c\n"), "a\\5cb\\00c\\0a");

    let rendered = render_user_filter("(&(objectClass=person)(uid={username}))", "bob)(cn=*");
    assert_eq!(rendered, "(&(objectClass=person)(uid=bob\\29\\28cn=\\2a))");
}

#[test]
fn escape_dn_value_follows_rfc4514() {
    assert_eq!(escape_dn_value("alice"), "alice");
    assert_eq!(escape_dn_value(" leading"), "\\ leading");
    assert_eq!(escape_dn_value("trailing "), "trailing\\ ");
    assert_eq!(escape_dn_value("#hash"), "\\#hash");
    assert_eq!(
        escape_dn_value("Doe, John+Team\\1<tag>;\"quoted\">"),
        "Doe\\, John\\+Team\\\\1\\<tag\\>\\;\\\"quoted\\\"\\>"
    );
}

#[test]
fn ldap_config_debug_redacts_bind_password_and_requires_tls() {
    let cfg = sample_config();
    let dbg = format!("{cfg:?}");
    assert!(!dbg.contains("super-secret-bind-pw"));
    assert!(dbg.contains("[REDACTED]"));
    assert!(cfg.validate().is_ok());

    let insecure = LdapConfig::new(
        "ldap://ldap.example.com:389",
        "cn=ro,dc=example,dc=com",
        "pw",
        "dc=example,dc=com",
    );
    assert!(insecure.validate().is_err());

    let starttls = insecure.with_starttls(true);
    assert!(starttls.validate().is_ok());
}

#[tokio::test]
async fn empty_password_or_username_is_rejected_without_network() {
    let db = setup_ldap_test_db().await;
    // Unreachable host verifies rejection occurs before any network I/O.
    let backend = LdapBackend::new(sample_config());

    assert!(backend
        .authenticate(&db, "alice", "")
        .await
        .unwrap()
        .is_none());
    assert!(backend.authenticate(&db, "", "pw").await.unwrap().is_none());
    assert!(backend
        .authenticate(&db, "   ", "pw")
        .await
        .unwrap()
        .is_none());
}

#[test]
fn extract_profile_maps_ad_attributes_case_insensitively() {
    let mut attrs = HashMap::new();
    attrs.insert("sAMAccountName".to_string(), vec!["jdoe".to_string()]);
    attrs.insert("MAIL".to_string(), vec!["jdoe@example.com".to_string()]);
    attrs.insert("givenName".to_string(), vec!["Jane".to_string()]);
    attrs.insert("sn".to_string(), vec!["Doe".to_string()]);
    attrs.insert(
        "memberOf".to_string(),
        vec![
            "CN=Staff,OU=Groups,DC=example,DC=com".to_string(),
            "CN=Admins,OU=Groups,DC=example,DC=com".to_string(),
        ],
    );

    let entry = SearchEntry {
        dn: "CN=Jane Doe,OU=Users,DC=example,DC=com".to_string(),
        attrs,
        bin_attrs: HashMap::new(),
    };

    let map = LdapAttributeMap {
        username_attr: "samaccountname".to_string(),
        ..Default::default()
    };

    let profile = extract_ldap_profile(&entry, "jdoe@example.com", &map);
    assert_eq!(profile.username, "jdoe");
    assert_eq!(profile.email.as_deref(), Some("jdoe@example.com"));
    assert_eq!(profile.first_name.as_deref(), Some("Jane"));
    assert_eq!(profile.last_name.as_deref(), Some("Doe"));
    assert_eq!(profile.groups.len(), 2);
}

#[tokio::test]
async fn jit_creation_and_group_flag_mapping_work_against_db() {
    let db = setup_ldap_test_db().await;
    let perm = Permission {
        id: 0,
        codename: "app.view_report".to_string(),
        name: "Can view report".to_string(),
    }
    .save(&db)
    .await
    .unwrap();
    let local_group = Group {
        id: 0,
        name: "editors".to_string(),
    }
    .save(&db)
    .await
    .unwrap();
    GroupPermission {
        id: 0,
        group: ForeignKey::new(local_group.id),
        permission: ForeignKey::new(perm.id),
    }
    .save(&db)
    .await
    .unwrap();

    let mut cfg = sample_config();
    cfg.group_mapping.staff_groups = vec!["Staff".to_string()];
    cfg.group_mapping.superuser_groups =
        vec!["CN=Domain Admins,OU=Groups,DC=example,DC=com".to_string()];
    cfg.group_mapping
        .group_map
        .insert("Content Editors".to_string(), "editors".to_string());

    let profile = LdapUserProfile {
        dn: "uid=alice,ou=People,dc=example,dc=com".to_string(),
        username: "alice".to_string(),
        email: Some("alice@example.com".to_string()),
        first_name: Some("Alice".to_string()),
        last_name: Some("Smith".to_string()),
        groups: vec![
            "CN=Staff,OU=Groups,DC=example,DC=com".to_string(),
            "CN=Content Editors,OU=Groups,DC=example,DC=com".to_string(),
        ],
    };

    let created = sync_ldap_user(&db, &profile, &cfg)
        .await
        .unwrap()
        .expect("JIT user created");
    assert_eq!(created.username, "alice");
    assert_eq!(created.email, "alice@example.com");
    assert_eq!(created.password, "!");
    assert!(created.is_staff);
    assert!(!created.is_superuser);
    assert!(has_perm(&db, created.id, "app.view_report").await.unwrap());

    // Simulate group removal and promotion on a subsequent login.
    let updated_profile = LdapUserProfile {
        groups: vec!["CN=Domain Admins,OU=Groups,DC=example,DC=com".to_string()],
        email: Some("alice.new@example.com".to_string()),
        ..profile
    };
    let updated = sync_ldap_user(&db, &updated_profile, &cfg)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.id, created.id);
    assert_eq!(updated.email, "alice.new@example.com");
    assert!(!updated.is_staff);
    assert!(updated.is_superuser);
    assert!(!has_perm(&db, updated.id, "app.view_report").await.unwrap());
}

#[tokio::test]
async fn create_users_jit_false_and_inactive_users_are_rejected() {
    let db = setup_ldap_test_db().await;
    let mut cfg = sample_config();
    cfg.create_users_jit = false;

    let profile = LdapUserProfile {
        dn: "uid=ghost,ou=People,dc=example,dc=com".to_string(),
        username: "ghost".to_string(),
        email: Some("ghost@example.com".to_string()),
        first_name: None,
        last_name: None,
        groups: vec![],
    };

    assert!(sync_ldap_user(&db, &profile, &cfg).await.unwrap().is_none());

    // An existing inactive user must also be rejected.
    User {
        id: 0,
        username: "ghost".to_string(),
        email: "ghost@example.com".to_string(),
        password: "!".to_string(),
        is_active: false,
        is_staff: false,
        is_superuser: false,
        date_joined: chrono::Utc::now(),
        last_login: None,
    }
    .save(&db)
    .await
    .unwrap();

    assert!(sync_ldap_user(&db, &profile, &cfg).await.unwrap().is_none());
}

#[tokio::test]
async fn live_ldap_authentication_when_env_configured() {
    let Ok(url) = std::env::var("LDAP_URL") else {
        return;
    };
    let bind_dn = std::env::var("LDAP_BIND_DN").unwrap_or_default();
    let bind_password = std::env::var("LDAP_BIND_PASSWORD").unwrap_or_default();
    let base_dn = std::env::var("LDAP_BASE_DN").expect("LDAP_BASE_DN must be set with LDAP_URL");
    let username = std::env::var("LDAP_TEST_USER").expect("LDAP_TEST_USER must be set");
    let password = std::env::var("LDAP_TEST_PASSWORD").expect("LDAP_TEST_PASSWORD must be set");

    let db = setup_ldap_test_db().await;
    let mut cfg = LdapConfig::new(url, bind_dn, bind_password, base_dn);
    if let Ok(filter) = std::env::var("LDAP_USER_FILTER") {
        cfg.user_filter = filter;
    }
    if std::env::var("LDAP_STARTTLS").as_deref() == Ok("1") {
        cfg.starttls = true;
    }
    if std::env::var("LDAP_ALLOW_INSECURE_TLS").as_deref() == Ok("1") {
        cfg.allow_insecure_tls = true;
        cfg.require_tls = false;
    }

    let backend = LdapBackend::new(cfg);
    let user = backend
        .authenticate(&db, &username, &password)
        .await
        .unwrap()
        .expect("live LDAP bind should succeed");
    assert_eq!(user.username, username);
    assert!(backend
        .authenticate(&db, &username, "definitely-wrong-password")
        .await
        .unwrap()
        .is_none());
}
