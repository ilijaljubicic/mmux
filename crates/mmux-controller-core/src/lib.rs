//! Runtime-neutral controller state for mmux.
//!
//! This crate owns the node registry and command queue semantics shared by
//! native and future Worker-based controller runtimes. It intentionally avoids
//! local runtime dependencies such as axum, tokio networking, ractor, tmux, and
//! filesystem access.

mod auth;
pub mod orchestration;

pub use auth::{
    NodeWireAuthContext, NodeWireAuthMethod, NodeWireAuthMode, NodeWireAuthPolicy,
    NodeWireIdentity, NodeWireIdentitySource,
};

use std::collections::{HashMap, VecDeque};

use mmux_wire::{NodeCommand, NodeCommandKind, NodeDescriptor, NodeStatus};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeSummary {
    pub node_id: String,
    pub display_name: String,
    pub status: String,
    pub last_seen_ms_ago: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegisteredNode {
    pub descriptor: NodeDescriptor,
    pub status: NodeStatus,
    pub last_seen_ms: u64,
}

#[derive(Clone, Debug)]
pub struct NodeRegistry {
    nodes: HashMap<String, RegisteredNode>,
    queues: HashMap<String, VecDeque<NodeCommand>>,
    next_command_id: u64,
    local_enabled: bool,
}

impl NodeRegistry {
    pub fn new(local_enabled: bool) -> Self {
        Self {
            nodes: HashMap::new(),
            queues: HashMap::new(),
            next_command_id: 1,
            local_enabled,
        }
    }

    pub fn register(&mut self, descriptor: NodeDescriptor, now_ms: u64) -> Result<String, String> {
        if descriptor.node_id.trim().is_empty() {
            return Err("node_id must not be empty".into());
        }
        if descriptor.node_id == "local" {
            return Err("'local' is reserved for the built-in node".into());
        }
        let node_id = descriptor.node_id.clone();
        self.nodes.insert(
            node_id.clone(),
            RegisteredNode {
                descriptor,
                status: NodeStatus::Ready,
                last_seen_ms: now_ms,
            },
        );
        self.queues.entry(node_id.clone()).or_default();
        Ok(format!("registered node '{}'", node_id))
    }

    pub fn heartbeat(
        &mut self,
        node_id: &str,
        status: NodeStatus,
        now_ms: u64,
    ) -> Result<(), String> {
        match self.nodes.get_mut(node_id) {
            Some(node) => {
                node.status = status;
                node.last_seen_ms = now_ms;
                Ok(())
            }
            None => Err(format!("node '{}' is not registered", node_id)),
        }
    }

    pub fn pull_commands(
        &mut self,
        node_id: &str,
        now_ms: u64,
    ) -> Result<Vec<NodeCommand>, String> {
        if !self.nodes.contains_key(node_id) {
            return Err(format!("node '{}' is not registered", node_id));
        }
        if let Some(node) = self.nodes.get_mut(node_id) {
            node.last_seen_ms = now_ms;
        }
        Ok(self
            .queues
            .entry(node_id.to_owned())
            .or_default()
            .drain(..)
            .collect())
    }

    pub fn note_result(&mut self, node_id: &str, now_ms: u64) {
        if let Some(node) = self.nodes.get_mut(node_id) {
            node.last_seen_ms = now_ms;
        }
    }

    pub fn dispatch(
        &mut self,
        node_id: &str,
        kind: NodeCommandKind,
    ) -> Result<NodeCommand, String> {
        if !self.nodes.contains_key(node_id) {
            return Err(format!("node '{}' is not registered", node_id));
        }
        let command_id = format!("cmd-{}", self.next_command_id);
        self.next_command_id += 1;
        let command = NodeCommand { command_id, kind };
        self.queues
            .entry(node_id.to_owned())
            .or_default()
            .push_back(command.clone());
        Ok(command)
    }

    pub fn list_nodes(&self, now_ms: u64) -> Vec<NodeSummary> {
        let mut nodes = Vec::new();
        if self.local_enabled {
            nodes.push(NodeSummary {
                node_id: "local".into(),
                display_name: "Local tmux node".into(),
                status: "ready".into(),
                last_seen_ms_ago: 0,
            });
        }
        nodes.extend(self.nodes.values().map(|node| self.summary(node, now_ms)));
        nodes
    }

    pub fn node_info(&self, node_id: &str, now_ms: u64) -> Result<NodeSummary, String> {
        if node_id == "local" {
            return Ok(NodeSummary {
                node_id: "local".into(),
                display_name: "Local tmux node".into(),
                status: if self.local_enabled {
                    "ready"
                } else {
                    "disabled"
                }
                .into(),
                last_seen_ms_ago: 0,
            });
        }
        self.nodes
            .get(node_id)
            .map(|node| self.summary(node, now_ms))
            .ok_or_else(|| format!("Node '{}' not found", node_id))
    }

    fn summary(&self, node: &RegisteredNode, now_ms: u64) -> NodeSummary {
        NodeSummary {
            node_id: node.descriptor.node_id.clone(),
            display_name: node.descriptor.display_name.clone(),
            status: format!("{:?}", node.status),
            last_seen_ms_ago: now_ms.saturating_sub(node.last_seen_ms),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descriptor(node_id: &str) -> NodeDescriptor {
        NodeDescriptor {
            node_id: node_id.into(),
            display_name: format!("node {node_id}"),
        }
    }

    #[test]
    fn registry_registers_and_lists_nodes() {
        let mut registry = NodeRegistry::new(true);

        registry.register(descriptor("n1"), 100).unwrap();
        let nodes = registry.list_nodes(250);

        assert!(nodes.iter().any(|node| node.node_id == "local"));
        let remote = nodes.iter().find(|node| node.node_id == "n1").unwrap();
        assert_eq!(remote.display_name, "node n1");
        assert_eq!(remote.status, "Ready");
        assert_eq!(remote.last_seen_ms_ago, 150);
    }

    #[test]
    fn registry_queues_commands_until_pull() {
        let mut registry = NodeRegistry::new(false);
        registry.register(descriptor("n1"), 0).unwrap();

        let command = registry
            .dispatch(
                "n1",
                NodeCommandKind::Tmux {
                    args: vec!["ls".into()],
                },
            )
            .unwrap();

        assert_eq!(command.command_id, "cmd-1");
        assert_eq!(registry.pull_commands("n1", 10).unwrap().len(), 1);
        assert!(registry.pull_commands("n1", 20).unwrap().is_empty());
    }

    #[test]
    fn registry_rejects_unknown_nodes() {
        let mut registry = NodeRegistry::new(false);

        assert!(registry.pull_commands("missing", 0).is_err());
        assert!(registry
            .dispatch("missing", NodeCommandKind::Shutdown)
            .is_err());
    }
}
