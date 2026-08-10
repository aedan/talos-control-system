use crate::db::models::auth::{User, UserRole};

#[derive(Debug, Clone, PartialEq)]
pub enum Resource {
    Cluster,
    Machine,
    MachineSet,
    Config,
    Branding,
    User,
    System,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    Read,
    Create,
    Update,
    Delete,
    Admin,
}

#[derive(Debug, Clone)]
pub struct Permission {
    pub resource: Resource,
    pub action: Action,
}

impl Permission {
    pub fn new(resource: Resource, action: Action) -> Self {
        Self { resource, action }
    }
}

/// Rank roles for comparison (higher = more privilege).
pub fn role_rank(role: &str) -> u8 {
    match role {
        "admin" => 3,
        "operator" => 2,
        "reader" | "viewer" => 1,
        _ => 0,
    }
}

pub fn role_at_least(have: &str, need: &str) -> bool {
    role_rank(have) >= role_rank(need)
}

pub fn check_permission(user: &User, resource: &Resource, action: &Action) -> bool {
    check_permission_by_role(&user.role, resource, action)
}

pub fn get_permissions(role: &UserRole) -> Vec<Permission> {
    match role {
        UserRole::Admin => vec![
            Permission::new(Resource::Cluster, Action::Read),
            Permission::new(Resource::Cluster, Action::Create),
            Permission::new(Resource::Cluster, Action::Update),
            Permission::new(Resource::Cluster, Action::Delete),
            Permission::new(Resource::Cluster, Action::Admin),
            Permission::new(Resource::Machine, Action::Read),
            Permission::new(Resource::Machine, Action::Create),
            Permission::new(Resource::Machine, Action::Update),
            Permission::new(Resource::Machine, Action::Delete),
            Permission::new(Resource::MachineSet, Action::Read),
            Permission::new(Resource::MachineSet, Action::Create),
            Permission::new(Resource::MachineSet, Action::Update),
            Permission::new(Resource::MachineSet, Action::Delete),
            Permission::new(Resource::Config, Action::Read),
            Permission::new(Resource::Config, Action::Update),
            Permission::new(Resource::Branding, Action::Read),
            Permission::new(Resource::Branding, Action::Update),
            Permission::new(Resource::User, Action::Read),
            Permission::new(Resource::User, Action::Create),
            Permission::new(Resource::User, Action::Update),
            Permission::new(Resource::User, Action::Delete),
            Permission::new(Resource::System, Action::Admin),
        ],
        UserRole::Operator => vec![
            Permission::new(Resource::Cluster, Action::Read),
            Permission::new(Resource::Cluster, Action::Create),
            Permission::new(Resource::Cluster, Action::Update),
            Permission::new(Resource::Machine, Action::Read),
            Permission::new(Resource::Machine, Action::Create),
            Permission::new(Resource::Machine, Action::Update),
            Permission::new(Resource::MachineSet, Action::Read),
            Permission::new(Resource::MachineSet, Action::Create),
            Permission::new(Resource::MachineSet, Action::Update),
            Permission::new(Resource::Config, Action::Read),
            Permission::new(Resource::Config, Action::Update),
        ],
        UserRole::Reader => vec![
            Permission::new(Resource::Cluster, Action::Read),
            Permission::new(Resource::Machine, Action::Read),
            Permission::new(Resource::MachineSet, Action::Read),
            Permission::new(Resource::Config, Action::Read),
            Permission::new(Resource::Branding, Action::Read),
        ],
    }
}

pub fn check_permission_by_role(role: &str, resource: &Resource, action: &Action) -> bool {
    match role {
        "admin" => true,
        "operator" => matches!(
            (resource, action),
            (Resource::Cluster, Action::Read)
                | (Resource::Cluster, Action::Create)
                | (Resource::Cluster, Action::Update)
                | (Resource::Machine, Action::Read)
                | (Resource::Machine, Action::Create)
                | (Resource::Machine, Action::Update)
                | (Resource::MachineSet, Action::Read)
                | (Resource::MachineSet, Action::Create)
                | (Resource::MachineSet, Action::Update)
                | (Resource::Config, Action::Read)
                | (Resource::Config, Action::Update)
        ),
        "reader" | "viewer" | _ => matches!(
            (resource, action),
            (Resource::Cluster, Action::Read)
                | (Resource::Machine, Action::Read)
                | (Resource::MachineSet, Action::Read)
                | (Resource::Config, Action::Read)
                | (Resource::Branding, Action::Read)
        ),
    }
}

/// Check whether an effective cluster role may perform `action` on cluster resources.
pub fn check_cluster_permission(effective_role: &str, action: &Action) -> bool {
    check_permission_by_role(effective_role, &Resource::Cluster, action)
        || check_permission_by_role(effective_role, &Resource::Machine, action)
        || check_permission_by_role(effective_role, &Resource::Config, action)
}

/// Minimum membership role required for a given HTTP-style action on a cluster.
pub fn min_role_for_action(action: &Action) -> &'static str {
    match action {
        Action::Read => "reader",
        Action::Create | Action::Update => "operator",
        Action::Delete | Action::Admin => "admin",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranks_and_thresholds() {
        assert!(role_at_least("admin", "operator"));
        assert!(role_at_least("operator", "reader"));
        assert!(!role_at_least("reader", "operator"));
        assert_eq!(min_role_for_action(&Action::Read), "reader");
        assert_eq!(min_role_for_action(&Action::Update), "operator");
        assert_eq!(min_role_for_action(&Action::Delete), "admin");
    }

    #[test]
    fn operator_cannot_delete_cluster_globally() {
        assert!(!check_permission_by_role(
            "operator",
            &Resource::Cluster,
            &Action::Delete
        ));
        assert!(check_permission_by_role(
            "operator",
            &Resource::Cluster,
            &Action::Update
        ));
    }
}
