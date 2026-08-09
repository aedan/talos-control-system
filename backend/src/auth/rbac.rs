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

pub fn check_permission(user: &User, resource: &Resource, action: &Action) -> bool {
    match user.role.as_str() {
        "admin" => true,
        "operator" => {
            matches!(
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
            )
        },
        "reader" | _ => {
            matches!(
                (resource, action),
                (Resource::Cluster, Action::Read)
                | (Resource::Machine, Action::Read)
                | (Resource::MachineSet, Action::Read)
                | (Resource::Config, Action::Read)
                | (Resource::Branding, Action::Read)
            )
        },
    }
}

pub fn get_permissions(role: &UserRole) -> Vec<Permission> {
    match role {
        UserRole::Admin => {
            vec![
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
            ]
        },
        UserRole::Operator => {
            vec![
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
            ]
        },
        UserRole::Reader => {
            vec![
                Permission::new(Resource::Cluster, Action::Read),
                Permission::new(Resource::Machine, Action::Read),
                Permission::new(Resource::MachineSet, Action::Read),
                Permission::new(Resource::Config, Action::Read),
                Permission::new(Resource::Branding, Action::Read),
            ]
        },
    }
}

pub fn check_cluster_permission(user: &User, _cluster_id: &str, action: &Action) -> bool {
    check_permission(user, &Resource::Cluster, action)
}

pub fn check_permission_by_role(role: &str, resource: &Resource, action: &Action) -> bool {
    match role {
        "admin" => true,
        "operator" => {
            matches!(
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
            )
        },
        "reader" | _ => {
            matches!(
                (resource, action),
                (Resource::Cluster, Action::Read)
                | (Resource::Machine, Action::Read)
                | (Resource::MachineSet, Action::Read)
                | (Resource::Config, Action::Read)
                | (Resource::Branding, Action::Read)
                | (Resource::User, Action::Read)
            )
        },
    }
}
