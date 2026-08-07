use crate::AppState;

pub struct ClusterServiceImpl {
    pub state: AppState,
}

impl ClusterServiceImpl {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

pub struct MachineServiceImpl {
    pub state: AppState,
}

impl MachineServiceImpl {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}

pub struct ResourceServiceImpl {
    pub state: AppState,
}

impl ResourceServiceImpl {
    pub fn new(state: AppState) -> Self {
        Self { state }
    }
}
