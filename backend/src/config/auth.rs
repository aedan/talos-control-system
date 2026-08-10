use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupRoleMapping {
    pub group_dn_pattern: String,
    pub role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LdapConfig {
    /// LDAP URL, e.g. `ldaps://ad.example.com:636` or `ldap://dc.example.com:389`.
    pub url: String,
    /// Optional service account used to search for the user DN before bind.
    /// Required for most Active Directory deployments.
    #[serde(default)]
    pub bind_dn: String,
    #[serde(default)]
    pub bind_password: String,
    pub user_search_base: String,
    /// Filter with `{0}` replaced by the login username (email local-part or full email).
    /// Example: `(sAMAccountName={0})` or `(uid={0})`
    pub user_search_filter: String,
    #[serde(default)]
    pub group_role_mappings: Vec<GroupRoleMapping>,
    #[serde(default = "default_ldap_role")]
    pub default_role: String,
    /// Prefer TLS (StartTLS) when using `ldap://` URLs. `ldaps://` is always TLS.
    #[serde(default)]
    pub use_tls: bool,
}

fn default_ldap_role() -> String {
    "reader".to_string()
}

impl Default for LdapConfig {
    fn default() -> Self {
        Self {
            url: "ldap://localhost:389".to_string(),
            bind_dn: String::new(),
            bind_password: String::new(),
            user_search_base: "dc=example,dc=com".to_string(),
            user_search_filter: "(sAMAccountName={0})".to_string(),
            group_role_mappings: vec![],
            default_role: default_ldap_role(),
            use_tls: false,
        }
    }
}

fn default_scopes() -> Vec<String> {
    vec![
        "openid".to_string(),
        "profile".to_string(),
        "email".to_string(),
    ]
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OidcConfig {
    pub enabled: bool,
    pub issuer_url: String,
    pub client_id: String,
    pub client_secret: String,
    pub redirect_url: String,
    #[serde(default = "default_scopes")]
    pub scopes: Vec<String>,
    /// Optional default role for newly provisioned OIDC users (default: reader).
    #[serde(default = "default_oidc_role")]
    pub default_role: String,
}

fn default_oidc_role() -> String {
    "reader".to_string()
}

impl Default for OidcConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            issuer_url: String::new(),
            client_id: String::new(),
            client_secret: String::new(),
            redirect_url: String::new(),
            scopes: default_scopes(),
            default_role: default_oidc_role(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SamlGroupMapping {
    pub group_pattern: String,
    pub role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SamlConfig {
    pub enabled: bool,
    /// Optional metadata URL to discover SSO location + signing cert.
    pub idp_metadata_url: String,
    /// Direct IdP SSO URL (HTTP-Redirect).
    pub idp_sso_url: String,
    /// Optional PEM of IdP signing certificate.
    pub idp_cert_pem: Option<String>,
    pub sp_entity_id: String,
    pub acs_url: String,
    pub attribute_email: String,
    pub attribute_name: String,
    pub attribute_groups: String,
    pub default_role: String,
    pub group_role_mappings: Vec<SamlGroupMapping>,
}

impl Default for SamlGroupMapping {
    fn default() -> Self {
        Self {
            group_pattern: String::new(),
            role: "reader".to_string(),
        }
    }
}

impl Default for SamlConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            idp_metadata_url: String::new(),
            idp_sso_url: String::new(),
            idp_cert_pem: None,
            sp_entity_id: "https://tcs.local/saml/sp".to_string(),
            acs_url: "https://tcs.local/api/auth/saml/acs".to_string(),
            attribute_email: "email".to_string(),
            attribute_name: "displayName".to_string(),
            attribute_groups: "groups".to_string(),
            default_role: "reader".to_string(),
            group_role_mappings: vec![],
        }
    }
}
