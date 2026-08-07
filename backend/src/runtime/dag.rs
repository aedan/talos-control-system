use std::collections::{HashMap, HashSet, VecDeque};
use tracing::{info, debug};

#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq)]
pub enum ControllerId {
    Cluster,
    Machine,
    MachineSet,
    Config,
    Branding,
}

impl std::fmt::Display for ControllerId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ControllerId::Cluster => write!(f, "cluster"),
            ControllerId::Machine => write!(f, "machine"),
            ControllerId::MachineSet => write!(f, "machineset"),
            ControllerId::Config => write!(f, "config"),
            ControllerId::Branding => write!(f, "branding"),
        }
    }
}

pub struct ControllerNode {
    pub id: ControllerId,
    pub enabled: bool,
    pub dependencies: Vec<ControllerId>,
}

impl ControllerNode {
    pub fn new(id: ControllerId) -> Self {
        Self {
            id,
            enabled: true,
            dependencies: Vec::new(),
        }
    }
}

pub struct ControllerDAG {
    nodes: HashMap<ControllerId, ControllerNode>,
    edges: HashMap<ControllerId, Vec<ControllerId>>,
}

impl ControllerDAG {
    pub fn new() -> Self {
        let mut dag = Self {
            nodes: HashMap::new(),
            edges: HashMap::new(),
        };

        dag.nodes.insert(ControllerId::Cluster, ControllerNode::new(ControllerId::Cluster));
        dag.nodes.insert(ControllerId::Machine, ControllerNode::new(ControllerId::Machine));
        dag.nodes.insert(ControllerId::MachineSet, ControllerNode::new(ControllerId::MachineSet));
        dag.nodes.insert(ControllerId::Config, ControllerNode::new(ControllerId::Config));
        dag.nodes.insert(ControllerId::Branding, ControllerNode::new(ControllerId::Branding));

        dag.add_edge(ControllerId::MachineSet, ControllerId::Cluster);
        dag.add_edge(ControllerId::Machine, ControllerId::MachineSet);
        dag.add_edge(ControllerId::Config, ControllerId::Machine);

        dag
    }

    pub fn add_edge(&mut self, from: ControllerId, to: ControllerId) {
        self.edges.entry(from).or_default().push(to);
        self.nodes.entry(to).or_insert(ControllerNode::new(to))
            .dependencies.push(from);
    }

    pub fn schedule(&self, triggered_by: ControllerId) -> Vec<ControllerId> {
        let mut affected = HashSet::new();
        let mut queue = VecDeque::new();

        queue.push_back(triggered_by);
        affected.insert(triggered_by);

        while let Some(node) = queue.pop_front() {
            if let Some(dependents) = self.edges.get(&node) {
                for dependent in dependents {
                    if affected.insert(*dependent) {
                        if let Some(nd) = self.nodes.get(dependent) {
                            if nd.enabled {
                                queue.push_back(*dependent);
                            }
                        }
                    }
                }
            }
        }

        let mut result: Vec<ControllerId> = affected.into_iter().collect();
        result.sort_by_key(|id| self.get_depth(id));

        let triggered = triggered_by;
        debug!(triggered_by = %triggered, affected = ?result, "DAG scheduling");
        result
    }

    fn get_depth(&self, id: &ControllerId) -> usize {
        if let Some(node) = self.nodes.get(id) {
            node.dependencies.iter()
                .map(|dep| self.get_depth(dep) + 1)
                .max()
                .unwrap_or(0)
        } else {
            0
        }
    }

    pub fn set_enabled(&mut self, id: &ControllerId, enabled: bool) {
        if let Some(node) = self.nodes.get_mut(id) {
            node.enabled = enabled;
            info!(controller = %id, enabled, "Controller enabled state changed");
        }
    }

    pub fn get_all(&self) -> &HashMap<ControllerId, ControllerNode> {
        &self.nodes
    }
}