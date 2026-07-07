use std::sync::Arc;
use crate::config::Config;
use crate::store::AgentStore;

pub struct Runtime {
    pub config: Config,
    pub agent: Arc<AgentStore>,
}
