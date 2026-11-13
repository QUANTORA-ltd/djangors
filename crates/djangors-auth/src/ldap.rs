//! LDAP / Active Directory authentication backend (`LdapBackend`).
//!
//! Connects over TLS (`ldaps://` or `ldap://` with `StartTLS`) via `ldap3`
//! using `rustls` (`ring` provider; zero OpenSSL / `aws-lc-rs`). Empty passwords
//! are always rejected before contacting the directory server so anonymous binds
//! can never authenticate a user.

use crate::{AuthBackend, AuthError, Group, User, UserGroup};
use djangors_db::Database;
use djangors_orm::{q, ForeignKey, Model as _};
use ldap3::{LdapConnAsync, LdapConnSettings, Scope, SearchEntry};
use secrecy::{ExposeSecret, SecretString};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

/// Escapes a user-supplied value for safe interpolation into an LDAP search filter
/// according to RFC 4515 §3.
///
/// Escapes `\`, `*`, `(`, `)`, NUL (`\0`), and any ASCII control characters
/// (`0x00..=0x1f`, `0x7f`) using `\xx` hexadecimal escapes.
pub fn escape_filter_value(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'\\' => out.push_str("\\5c"),
            b'*' => out.push_str("\\2a"),
            b'(' => out.push_str("\\28"),
            b')' => out.push_str("\\29"),
            0x00 => out.push_str("\\00"),
            b if b < 0x20 || b == 0x7f => {
                out.push('\\');
                out.push_str(&format!("{b:02x}"));
            }
            _ => {
                // Non-ASCII bytes are valid UTF-8 sequences inside an UTF-8 `&str`
                // and can be safely pushed after reconstructing char by char; handle
                // via char iteration below for clarity.
            }
        }
    }
    // Re-implement cleanly per char so multi-byte UTF-8 characters stay intact.
    out.clear();
    for ch in input.chars() {
        match ch {
            '\\' => out.push_str("\\5c"),
            '*' => out.push_str("\\2a"),
            '(' => out.push_str("\\28"),
            ')' => out.push_str("\\29"),
            '\0' => out.push_str("\\00"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                let mut buf = [0u8; 4];
                for &b in c.encode_utf8(&mut buf).as_bytes() {
                    out.push('\\');
                    out.push_str(&format!("{b:02x}"));
                }
            }
            c => out.push(c),
        }
    }
    out
}

/// Escapes an attribute value for safe interpolation into an LDAP Distinguished
/// Name (DN) according to RFC 4514 §2.4.
///
/// Escapes leading `' '` or `'#'`, trailing `' '`, the reserved characters
/// `"`, `+`, `,`, `;`, `<`, `>`, `\`, and any ASCII control bytes (`0x00..=0x1f`, `0x7f`).
pub fn escape_dn_value(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut out = String::with_capacity(input.len());

    for (idx, &ch) in chars.iter().enumerate() {
        match ch {
            ' ' if idx == 0 || idx + 1 == len => out.push_str("\\ "),
            '#' if idx == 0 => out.push_str("\\#"),
            '\\' | '"' | '+' | ',' | ';' | '<' | '>' => {
                out.push('\\');
                out.push(ch);
            }
            '\0' => out.push_str("\\00"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                let mut buf = [0u8; 4];
                for &b in c.encode_utf8(&mut buf).as_bytes() {
                    out.push('\\');
                    out.push_str(&format!("{b:02x}"));
                }
            }
            c => out.push(c),
        }
    }
    out
}

/// Maps LDAP directory attributes onto Djangors user fields and profile metadata.
#[derive(Debug, Clone)]
pub struct LdapAttributeMap {
    /// Attribute holding the canonical username (e.g. `"uid"` or `"sAMAccountName"`).
    pub username_attr: String,
    /// Attribute holding the user's email address (default `"mail"`).
    pub email_attr: String,
    /// Attribute holding given name (default `"givenName"`).
    pub first_name_attr: String,
    /// Attribute holding surname (default `"sn"`).
    pub last_name_attr: String,
    /// Attribute holding group DNs or names (default `"memberOf"`).
    pub groups_attr: String,
}

impl Default for LdapAttributeMap {
    fn default() -> Self {
        Self {
            username_attr: "uid".to_string(),
            email_attr: "mail".to_string(),
            first_name_attr: "givenName".to_string(),
            last_name_attr: "sn".to_string(),
            groups_attr: "memberOf".to_string(),
        }
    }
}

/// Controls how LDAP group memberships map to `User::is_staff`,
/// `User::is_superuser`, and local Djangors [`Group`] memberships.
#[derive(Debug, Clone, Default)]
pub struct LdapGroupMapping {
    /// LDAP group DNs or CNs whose members receive `is_staff = true`.
    pub staff_groups: Vec<String>,
    /// LDAP group DNs or CNs whose members receive `is_superuser = true`.
    pub superuser_groups: Vec<String>,
    /// Explicit mapping from LDAP group DN or CN to Djangors [`Group::name`].
    pub group_map: HashMap<String, String>,
    /// When `true`, automatically sync every LDAP group's CN into `auth_group` / `auth_user_groups`.
    pub sync_all_groups: bool,
}

/// User attributes extracted from an LDAP search entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LdapUserProfile {
    /// Full distinguished name (`dn`) returned by the directory.
    pub dn: String,
    /// Mapped username (falls back to the login username if attribute is absent).
    pub username: String,
    /// Optional email address.
    pub email: Option<String>,
    /// Optional given name (`givenName` by default).
    pub first_name: Option<String>,
    /// Optional surname (`sn` by default).
    pub last_name: Option<String>,
    /// Raw values from the configured groups attribute (typically DNs).
    pub groups: Vec<String>,
}

/// Configuration for [`LdapBackend`].
///
/// Secrets such as `bind_password` are stored in [`SecretString`] and redacted
/// in `Debug` output.
#[derive(Clone)]
pub struct LdapConfig {
    /// Directory URL (e.g. `"ldaps://ldap.example.com:636"` or `"ldap://ldap.example.com:389"`).
    pub url: String,
    /// Distinguished Name of the service account used to search for users.
    pub bind_dn: String,
    /// Password for `bind_dn` (redacted in `Debug`).
    pub bind_password: SecretString,
    /// Search base DN (e.g. `"ou=People,dc=example,dc=com"`).
    pub base_dn: String,
    /// LDAP filter template containing `{username}` (escaped per RFC 4515).
    /// Defaults to `"(uid={username})"`.
    pub user_filter: String,
    /// Upgrade plain `ldap://` connection using the StartTLS extended operation.
    pub starttls: bool,
    /// Optional PEM CA certificate bundle path validated before connecting.
    /// Note: `ldap3` 0.12 with `tls-rustls-ring` uses the platform trust store
    /// (and `SSL_CERT_FILE` if configured by the environment).
    pub ca_file: Option<PathBuf>,
    /// Reject connections that neither use `ldaps://` nor enable `starttls`.
    /// Defaults to `true`.
    pub require_tls: bool,
    /// Skip TLS certificate verification (for development/self-signed servers only).
    /// Defaults to `false`.
    pub allow_insecure_tls: bool,
    /// TCP connection timeout (defaults to 10 seconds).
    pub conn_timeout: Duration,
    /// Attribute mapping rules.
    pub attributes: LdapAttributeMap,
    /// Staff / superuser / `Group` mapping rules.
    pub group_mapping: LdapGroupMapping,
    /// Create missing local `User` rows automatically on first LDAP login (defaults to `true`).
    pub create_users_jit: bool,
    /// Update email, flags, and mapped groups on existing users at login (defaults to `true`).
    pub update_user_on_login: bool,
}

impl fmt::Debug for LdapConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LdapConfig")
            .field("url", &self.url)
            .field("bind_dn", &self.bind_dn)
            .field("bind_password", &"[REDACTED]")
            .field("base_dn", &self.base_dn)
            .field("user_filter", &self.user_filter)
            .field("starttls", &self.starttls)
            .field("ca_file", &self.ca_file)
            .field("require_tls", &self.require_tls)
            .field("allow_insecure_tls", &self.allow_insecure_tls)
            .field("conn_timeout", &self.conn_timeout)
            .field("attributes", &self.attributes)
            .field("group_mapping", &self.group_mapping)
            .field("create_users_jit", &self.create_users_jit)
            .field("update_user_on_login", &self.update_user_on_login)
            .finish()
    }
}

impl LdapConfig {
    /// Creates a new [`LdapConfig`] with secure TLS defaults (`require_tls = true`,
    /// `allow_insecure_tls = false`, `create_users_jit = true`, `update_user_on_login = true`).
    pub fn new(
        url: impl Into<String>,
        bind_dn: impl Into<String>,
        bind_password: impl Into<String>,
        base_dn: impl Into<String>,
    ) -> Self {
        Self {
            url: url.into(),
            bind_dn: bind_dn.into(),
            bind_password: SecretString::from(bind_password.into()),
            base_dn: base_dn.into(),
            user_filter: "(uid={username})".to_string(),
            starttls: false,
            ca_file: None,
            require_tls: true,
            allow_insecure_tls: false,
            conn_timeout: Duration::from_secs(10),
            attributes: LdapAttributeMap::default(),
            group_mapping: LdapGroupMapping::default(),
            create_users_jit: true,
            update_user_on_login: true,
        }
    }

    /// Sets the LDAP search filter template (e.g. `"(sAMAccountName={username})"`).
    pub fn with_user_filter(mut self, filter: impl Into<String>) -> Self {
        self.user_filter = filter.into();
        self
    }

    /// Enables or disables StartTLS on `ldap://` connections.
    pub fn with_starttls(mut self, starttls: bool) -> Self {
        self.starttls = starttls;
        self
    }

    /// Sets an optional custom PEM CA certificate path.
    pub fn with_ca_file(mut self, ca_file: impl Into<PathBuf>) -> Self {
        self.ca_file = Some(ca_file.into());
        self
    }

    /// Validates configuration invariants before dialing the LDAP server.
    pub fn validate(&self) -> Result<(), AuthError> {
        let trimmed = self.url.trim();
        if trimmed.is_empty() {
            return Err(AuthError::Ldap("LDAP url must not be empty".to_string()));
        }
        let lower = trimmed.to_ascii_lowercase();
        let is_ldaps = lower.starts_with("ldaps://");
        let is_ldap = lower.starts_with("ldap://");
        if !is_ldaps && !is_ldap {
            return Err(AuthError::Ldap(
                "LDAP url must begin with ldaps:// or ldap://".to_string(),
            ));
        }
        if self.require_tls && is_ldap && !self.starttls {
            return Err(AuthError::Ldap(
                "plaintext ldap:// refused while require_tls = true and starttls = false"
                    .to_string(),
            ));
        }
        if let Some(ca_path) = &self.ca_file {
            let pem = std::fs::read_to_string(ca_path).map_err(|e| {
                AuthError::Ldap(format!(
                    "failed to read LDAP ca_file '{}': {e}",
                    ca_path.display()
                ))
            })?;
            if !pem.contains("-----BEGIN CERTIFICATE-----") {
                return Err(AuthError::Ldap(format!(
                    "LDAP ca_file '{}' does not contain a PEM certificate",
                    ca_path.display()
                )));
            }
        }
        Ok(())
    }
}

/// Renders the configured filter template, replacing `{username}` with the
/// RFC 4515-escaped value and `{dn_username}` with the RFC 4514-escaped value.
pub fn render_user_filter(template: &str, username: &str) -> String {
    let filter_safe = escape_filter_value(username);
    let dn_safe = escape_dn_value(username);
    template
        .replace("{username}", &filter_safe)
        .replace("{dn_username}", &dn_safe)
}

fn attr_values<'a>(attrs: &'a HashMap<String, Vec<String>>, key: &str) -> Option<&'a Vec<String>> {
    attrs.get(key).or_else(|| {
        attrs
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v)
    })
}

fn first_attr(attrs: &HashMap<String, Vec<String>>, key: &str) -> Option<String> {
    attr_values(attrs, key)
        .and_then(|vals| vals.first())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Extracts a [`LdapUserProfile`] from a [`SearchEntry`], falling back to
/// `login_username` if the mapped username attribute is absent.
pub fn extract_ldap_profile(
    entry: &SearchEntry,
    login_username: &str,
    map: &LdapAttributeMap,
) -> LdapUserProfile {
    let username = first_attr(&entry.attrs, &map.username_attr)
        .unwrap_or_else(|| login_username.trim().to_string());
    let email = first_attr(&entry.attrs, &map.email_attr);
    let first_name = first_attr(&entry.attrs, &map.first_name_attr);
    let last_name = first_attr(&entry.attrs, &map.last_name_attr);
    let groups = attr_values(&entry.attrs, &map.groups_attr)
        .cloned()
        .unwrap_or_default();
    LdapUserProfile {
        dn: entry.dn.clone(),
        username,
        email,
        first_name,
        last_name,
        groups,
    }
}

/// Extracts the leading CN attribute value from an LDAP group DN
/// (for example `"CN=Engineering,OU=Groups,DC=example,DC=com"` -> `"Engineering"`),
/// or returns the trimmed string as-is when it is not a DN.
pub fn extract_group_cn(dn_or_name: &str) -> String {
    let trimmed = dn_or_name.trim();
    // Split on the first unescaped comma.
    let chars = trimmed.char_indices().peekable();
    let mut first_rdn_end = trimmed.len();
    let mut escaped = false;
    for (idx, ch) in chars {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
        } else if ch == ',' {
            first_rdn_end = idx;
            break;
        }
    }
    let first_rdn = &trimmed[..first_rdn_end];
    if let Some((attr, val)) = first_rdn.split_once('=') {
        if attr.trim().eq_ignore_ascii_case("cn") {
            return val.trim().replace("\\,", ",");
        }
    }
    trimmed.to_string()
}

fn matches_group(entry_groups: &[String], targets: &[String]) -> bool {
    if targets.is_empty() {
        return false;
    }
    for group in entry_groups {
        let cn = extract_group_cn(group);
        for target in targets {
            if group.eq_ignore_ascii_case(target.trim()) || cn.eq_ignore_ascii_case(target.trim()) {
                return true;
            }
        }
    }
    false
}

fn resolve_target_group_names(entry_groups: &[String], mapping: &LdapGroupMapping) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for raw in entry_groups {
        let cn = extract_group_cn(raw);
        let mapped = mapping
            .group_map
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(raw.trim()) || k.eq_ignore_ascii_case(&cn))
            .map(|(_, v)| v.clone())
            .or({
                if mapping.sync_all_groups && !cn.is_empty() {
                    Some(cn)
                } else {
                    None
                }
            });
        if let Some(name) = mapped {
            if seen.insert(name.clone()) {
                out.push(name);
            }
        }
    }
    out
}

/// Synchronizes (and optionally just-in-time provisions) a local [`User`] and its
/// [`Group`] memberships from an authenticated [`LdapUserProfile`].
pub async fn sync_ldap_user(
    db: &Database,
    profile: &LdapUserProfile,
    config: &LdapConfig,
) -> Result<Option<User>, AuthError> {
    let mut existing = User::objects()
        .filter(q!(username = profile.username.as_str()))?
        .all(db)
        .await?;

    let target_is_staff = matches_group(&profile.groups, &config.group_mapping.staff_groups);
    let target_is_superuser =
        matches_group(&profile.groups, &config.group_mapping.superuser_groups);

    let mut user = match existing.pop() {
        Some(mut u) => {
            if !u.is_active {
                return Ok(None);
            }
            if config.update_user_on_login {
                let mut changed = false;
                if let Some(email) = &profile.email {
                    if &u.email != email {
                        u.email = email.clone();
                        changed = true;
                    }
                }
                if !config.group_mapping.staff_groups.is_empty() && u.is_staff != target_is_staff {
                    u.is_staff = target_is_staff;
                    changed = true;
                }
                if !config.group_mapping.superuser_groups.is_empty()
                    && u.is_superuser != target_is_superuser
                {
                    u.is_superuser = target_is_superuser;
                    changed = true;
                }
                if changed {
                    u.update(db).await?;
                }
            }
            u
        }
        None => {
            if !config.create_users_jit {
                return Ok(None);
            }
            let new_user = User {
                id: 0,
                username: profile.username.clone(),
                email: profile.email.clone().unwrap_or_default(),
                // Unusable password sentinel so ModelBackend never accepts password logins.
                password: "!".to_string(),
                is_active: true,
                is_staff: target_is_staff,
                is_superuser: target_is_superuser,
                date_joined: chrono::Utc::now(),
                last_login: None,
            };
            new_user.save(db).await?
        }
    };

    if config.update_user_on_login
        && (!config.group_mapping.group_map.is_empty() || config.group_mapping.sync_all_groups)
    {
        let desired_groups = resolve_target_group_names(&profile.groups, &config.group_mapping);
        sync_user_groups(db, &user, &desired_groups, &config.group_mapping).await?;
    }

    user.last_login = Some(chrono::Utc::now());
    user.update(db).await?;

    Ok(Some(user))
}

async fn sync_user_groups(
    db: &Database,
    user: &User,
    desired_names: &[String],
    mapping: &LdapGroupMapping,
) -> Result<(), AuthError> {
    // Collect names of all groups managed by this mapping so removed memberships
    // are dropped without touching unrelated local groups (unless sync_all_groups).
    let managed_names: HashSet<String> = mapping.group_map.values().cloned().collect();
    let desired_set: HashSet<&str> = desired_names.iter().map(String::as_str).collect();

    let existing_links = UserGroup::objects()
        .filter(q!(user = user.id))?
        .all(db)
        .await?;
    let mut existing_group_names = HashSet::new();

    for link in existing_links {
        if let Some(group) = Group::objects()
            .filter(q!(id = link.group.id))?
            .all(db)
            .await?
            .into_iter()
            .next()
        {
            if (mapping.sync_all_groups || managed_names.contains(&group.name))
                && !desired_set.contains(group.name.as_str())
            {
                link.delete(db).await?;
            } else {
                existing_group_names.insert(group.name);
            }
        }
    }

    for group_name in desired_names {
        if existing_group_names.contains(group_name) {
            continue;
        }
        let group = match Group::objects()
            .filter(q!(name = group_name.as_str()))?
            .all(db)
            .await?
            .into_iter()
            .next()
        {
            Some(g) => g,
            None => {
                Group {
                    id: 0,
                    name: group_name.clone(),
                }
                .save(db)
                .await?
            }
        };
        UserGroup {
            id: 0,
            user: ForeignKey::new(user.id),
            group: ForeignKey::new(group.id),
        }
        .save(db)
        .await?;
    }

    Ok(())
}

/// [`AuthBackend`] authenticating users against an LDAP / Active Directory server.
#[derive(Debug, Clone)]
pub struct LdapBackend {
    config: LdapConfig,
}

impl LdapBackend {
    /// Constructs a new [`LdapBackend`] from `config`.
    pub fn new(config: LdapConfig) -> Self {
        Self { config }
    }

    /// Returns a reference to the underlying [`LdapConfig`].
    pub fn config(&self) -> &LdapConfig {
        &self.config
    }
}

#[async_trait::async_trait]
impl AuthBackend for LdapBackend {
    type User = User;

    async fn authenticate(
        &self,
        db: &Database,
        username: &str,
        password: &str,
    ) -> Result<Option<User>, AuthError> {
        // Never allow empty passwords (anonymous bind) or blank usernames.
        if username.trim().is_empty() || password.is_empty() {
            return Ok(None);
        }

        self.config.validate()?;

        let settings = LdapConnSettings::new()
            .set_conn_timeout(self.config.conn_timeout)
            .set_starttls(self.config.starttls)
            .set_no_tls_verify(self.config.allow_insecure_tls);

        let (conn, mut ldap) = LdapConnAsync::with_settings(settings, &self.config.url)
            .await
            .map_err(|e| AuthError::Ldap(format!("LDAP connection failed: {e}")))?;
        ldap3::drive!(conn);

        // 1. Bind with service account to search for the user DN.
        if !self.config.bind_dn.trim().is_empty() {
            ldap.simple_bind(
                &self.config.bind_dn,
                self.config.bind_password.expose_secret(),
            )
            .await
            .map_err(|e| AuthError::Ldap(format!("service account bind error: {e}")))?
            .success()
            .map_err(|e| AuthError::Ldap(format!("service account bind rejected: {e}")))?;
        }

        let filter = render_user_filter(&self.config.user_filter, username);
        let attrs = vec![
            self.config.attributes.username_attr.as_str(),
            self.config.attributes.email_attr.as_str(),
            self.config.attributes.first_name_attr.as_str(),
            self.config.attributes.last_name_attr.as_str(),
            self.config.attributes.groups_attr.as_str(),
        ];

        let (mut entries, _res) = ldap
            .search(&self.config.base_dn, Scope::Subtree, &filter, attrs)
            .await
            .map_err(|e| AuthError::Ldap(format!("LDAP search error: {e}")))?
            .success()
            .map_err(|e| AuthError::Ldap(format!("LDAP search rejected: {e}")))?;

        // Require an unambiguous single matching user entry.
        if entries.len() != 1 {
            let _ = ldap.unbind().await;
            return Ok(None);
        }

        let search_entry = SearchEntry::construct(entries.remove(0));
        if search_entry.dn.trim().is_empty() {
            let _ = ldap.unbind().await;
            return Ok(None);
        }

        // 2. Verify credentials by binding directly as the user DN.
        let bind_res = ldap
            .simple_bind(&search_entry.dn, password)
            .await
            .map_err(|e| AuthError::Ldap(format!("user bind transport error: {e}")))?;
        let _ = ldap.unbind().await;

        if bind_res.rc != 0 {
            if bind_res.rc == 49 || bind_res.rc == 32 {
                return Ok(None);
            }
            return Err(AuthError::Ldap(format!(
                "LDAP user bind returned error code {}",
                bind_res.rc
            )));
        }

        let profile = extract_ldap_profile(&search_entry, username, &self.config.attributes);
        sync_ldap_user(db, &profile, &self.config).await
    }
}

#[cfg(test)]
#[path = "ldap_tests.rs"]
mod tests;
