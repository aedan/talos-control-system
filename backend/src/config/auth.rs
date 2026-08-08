use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupRoleMapping {
    pub group_dn_pattern: String,
    pub role: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LdapConfig {
    pub url: String,
    pub user_search_base: String,
    pub user_search_filter: String,
    pub group_role_mappings: Vec<GroupRoleMapping>,
    pub default_role: String,
    pub use_tls: bool,
}

impl Default for LdapConfig {
    fn default() -> Self {
        Self {
            url: "ldap://localhost:389".to_string(),
            user_search_base: "dc=example,dc=com".to_string(),
            user_search_filter: "sAMAccountName={0}".to_string(),
            group_role_mappings: vec![],
            default_role: "reader".to_string(),
            use_tls: false,
        }
    }
}
