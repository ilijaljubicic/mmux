//! Shared wire DTOs and schema mount points for mmux controller/node traffic.
//!
//! The canonical schema lives under `proto/mmux/wire/v1`. This crate mounts
//! the generated Buffa/ConnectRPC modules and keeps stable DTO names for the
//! controller/node actor boundary.

use serde::{Deserialize, Serialize};

#[path = "generated/buffa/mod.rs"]
pub mod proto;

#[path = "generated/connect/mod.rs"]
pub mod connect;

use proto::mmux::wire::v1 as wire;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeDescriptor {
    pub node_id: String,
    pub display_name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegisterNodeRequest {
    pub descriptor: NodeDescriptor,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegisterNodeResponse {
    pub accepted: bool,
    pub message: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PullCommandsRequest {
    pub node_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PullCommandsResponse {
    pub commands: Vec<NodeCommand>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeCommand {
    pub command_id: String,
    pub kind: NodeCommandKind,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum NodeCommandKind {
    Tmux {
        args: Vec<String>,
    },
    ReadFile {
        path: String,
        offset: Option<u64>,
        limit: usize,
    },
    WriteFile {
        path: String,
        content_base64: String,
        append: bool,
    },
    Shutdown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubmitCommandResultRequest {
    pub node_id: String,
    pub command_id: String,
    pub result: NodeCommandResult,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum NodeCommandResult {
    TmuxOutput(String),
    FileContent { content_base64: String },
    WriteComplete { bytes_written: usize },
    Error { message: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubmitCommandResultResponse {
    pub accepted: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HeartbeatRequest {
    pub node_id: String,
    pub status: NodeStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum NodeStatus {
    Ready,
    Busy,
    Draining,
    Unhealthy { message: String },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HeartbeatResponse {
    pub accepted: bool,
}

pub fn node_descriptor_to_proto(descriptor: NodeDescriptor) -> wire::NodeDescriptor {
    wire::NodeDescriptor {
        node_id: descriptor.node_id,
        display_name: descriptor.display_name,
        ..Default::default()
    }
}

pub fn node_descriptor_from_proto(descriptor: wire::NodeDescriptor) -> NodeDescriptor {
    NodeDescriptor {
        node_id: descriptor.node_id,
        display_name: descriptor.display_name,
    }
}

pub fn register_node_request_to_proto(request: RegisterNodeRequest) -> wire::RegisterNodeRequest {
    wire::RegisterNodeRequest {
        descriptor: buffa::MessageField::some(node_descriptor_to_proto(request.descriptor)),
        ..Default::default()
    }
}

pub fn register_node_request_from_proto(
    request: wire::RegisterNodeRequest,
) -> Result<RegisterNodeRequest, String> {
    let descriptor = request
        .descriptor
        .into_option()
        .ok_or("register request missing descriptor")?;
    Ok(RegisterNodeRequest {
        descriptor: node_descriptor_from_proto(descriptor),
    })
}

pub fn register_node_response_to_proto(
    response: RegisterNodeResponse,
) -> wire::RegisterNodeResponse {
    wire::RegisterNodeResponse {
        accepted: response.accepted,
        message: response.message,
        ..Default::default()
    }
}

pub fn pull_commands_request_to_proto(request: PullCommandsRequest) -> wire::PullCommandsRequest {
    wire::PullCommandsRequest {
        node_id: request.node_id,
        ..Default::default()
    }
}

pub fn pull_commands_request_from_proto(request: wire::PullCommandsRequest) -> PullCommandsRequest {
    PullCommandsRequest {
        node_id: request.node_id,
    }
}

pub fn pull_commands_response_to_proto(
    response: PullCommandsResponse,
) -> wire::PullCommandsResponse {
    wire::PullCommandsResponse {
        commands: response
            .commands
            .into_iter()
            .map(node_command_to_proto)
            .collect(),
        ..Default::default()
    }
}

pub fn pull_commands_response_from_proto(
    response: wire::PullCommandsResponse,
) -> Result<PullCommandsResponse, String> {
    let commands = response
        .commands
        .into_iter()
        .map(node_command_from_proto)
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PullCommandsResponse { commands })
}

pub fn node_command_to_proto(command: NodeCommand) -> wire::NodeCommand {
    let kind = match command.kind {
        NodeCommandKind::Tmux { args } => {
            wire::__buffa::oneof::node_command::Kind::Tmux(Box::new(wire::TmuxCommand {
                args,
                ..Default::default()
            }))
        }
        NodeCommandKind::ReadFile {
            path,
            offset,
            limit,
        } => wire::__buffa::oneof::node_command::Kind::ReadFile(Box::new(wire::ReadFileCommand {
            path,
            offset,
            limit: limit as u64,
            ..Default::default()
        })),
        NodeCommandKind::WriteFile {
            path,
            content_base64,
            append,
        } => {
            wire::__buffa::oneof::node_command::Kind::WriteFile(Box::new(wire::WriteFileCommand {
                path,
                content_base64,
                append,
                ..Default::default()
            }))
        }
        NodeCommandKind::Shutdown => {
            wire::__buffa::oneof::node_command::Kind::Shutdown(Box::new(wire::ShutdownCommand {
                ..Default::default()
            }))
        }
    };
    wire::NodeCommand {
        command_id: command.command_id,
        kind: Some(kind),
        ..Default::default()
    }
}

pub fn node_command_from_proto(command: wire::NodeCommand) -> Result<NodeCommand, String> {
    let kind = match command.kind.ok_or("node command missing kind")? {
        wire::__buffa::oneof::node_command::Kind::Tmux(command) => {
            NodeCommandKind::Tmux { args: command.args }
        }
        wire::__buffa::oneof::node_command::Kind::ReadFile(command) => NodeCommandKind::ReadFile {
            path: command.path,
            offset: command.offset,
            limit: command.limit as usize,
        },
        wire::__buffa::oneof::node_command::Kind::WriteFile(command) => {
            NodeCommandKind::WriteFile {
                path: command.path,
                content_base64: command.content_base64,
                append: command.append,
            }
        }
        wire::__buffa::oneof::node_command::Kind::Shutdown(_) => NodeCommandKind::Shutdown,
    };
    Ok(NodeCommand {
        command_id: command.command_id,
        kind,
    })
}

pub fn submit_command_result_request_to_proto(
    request: SubmitCommandResultRequest,
) -> wire::SubmitCommandResultRequest {
    wire::SubmitCommandResultRequest {
        node_id: request.node_id,
        command_id: request.command_id,
        result: buffa::MessageField::some(node_command_result_to_proto(request.result)),
        ..Default::default()
    }
}

pub fn submit_command_result_request_from_proto(
    request: wire::SubmitCommandResultRequest,
) -> Result<SubmitCommandResultRequest, String> {
    let result = request
        .result
        .into_option()
        .ok_or_else(|| "submit command result request missing result".to_string())
        .and_then(node_command_result_from_proto)?;
    Ok(SubmitCommandResultRequest {
        node_id: request.node_id,
        command_id: request.command_id,
        result,
    })
}

pub fn submit_command_result_response_to_proto(
    response: SubmitCommandResultResponse,
) -> wire::SubmitCommandResultResponse {
    wire::SubmitCommandResultResponse {
        accepted: response.accepted,
        ..Default::default()
    }
}

pub fn heartbeat_request_to_proto(request: HeartbeatRequest) -> wire::HeartbeatRequest {
    wire::HeartbeatRequest {
        node_id: request.node_id,
        status: buffa::MessageField::some(node_status_to_proto(request.status)),
        ..Default::default()
    }
}

pub fn heartbeat_request_from_proto(
    request: wire::HeartbeatRequest,
) -> Result<HeartbeatRequest, String> {
    let status = request
        .status
        .into_option()
        .map(node_status_from_proto)
        .transpose()?
        .unwrap_or(NodeStatus::Ready);
    Ok(HeartbeatRequest {
        node_id: request.node_id,
        status,
    })
}

pub fn heartbeat_response_to_proto(response: HeartbeatResponse) -> wire::HeartbeatResponse {
    wire::HeartbeatResponse {
        accepted: response.accepted,
        ..Default::default()
    }
}

pub fn node_command_result_to_proto(result: NodeCommandResult) -> wire::NodeCommandResult {
    let kind = match result {
        NodeCommandResult::TmuxOutput(output) => {
            wire::__buffa::oneof::node_command_result::Kind::TmuxOutput(Box::new(
                wire::TmuxOutput {
                    output,
                    ..Default::default()
                },
            ))
        }
        NodeCommandResult::FileContent { content_base64 } => {
            wire::__buffa::oneof::node_command_result::Kind::FileContent(Box::new(
                wire::FileContent {
                    content_base64,
                    ..Default::default()
                },
            ))
        }
        NodeCommandResult::WriteComplete { bytes_written } => {
            wire::__buffa::oneof::node_command_result::Kind::WriteComplete(Box::new(
                wire::WriteComplete {
                    bytes_written: bytes_written as u64,
                    ..Default::default()
                },
            ))
        }
        NodeCommandResult::Error { message } => {
            wire::__buffa::oneof::node_command_result::Kind::Error(Box::new(wire::CommandError {
                message,
                ..Default::default()
            }))
        }
    };
    wire::NodeCommandResult {
        kind: Some(kind),
        ..Default::default()
    }
}

pub fn node_command_result_from_proto(
    result: wire::NodeCommandResult,
) -> Result<NodeCommandResult, String> {
    match result.kind.ok_or("node command result missing kind")? {
        wire::__buffa::oneof::node_command_result::Kind::TmuxOutput(result) => {
            Ok(NodeCommandResult::TmuxOutput(result.output))
        }
        wire::__buffa::oneof::node_command_result::Kind::FileContent(result) => {
            Ok(NodeCommandResult::FileContent {
                content_base64: result.content_base64,
            })
        }
        wire::__buffa::oneof::node_command_result::Kind::WriteComplete(result) => {
            Ok(NodeCommandResult::WriteComplete {
                bytes_written: result.bytes_written as usize,
            })
        }
        wire::__buffa::oneof::node_command_result::Kind::Error(result) => {
            Ok(NodeCommandResult::Error {
                message: result.message,
            })
        }
    }
}

pub fn node_status_to_proto(status: NodeStatus) -> wire::NodeStatus {
    let (kind, message) = match status {
        NodeStatus::Ready => (wire::node_status::Kind::KIND_READY, String::new()),
        NodeStatus::Busy => (wire::node_status::Kind::KIND_BUSY, String::new()),
        NodeStatus::Draining => (wire::node_status::Kind::KIND_DRAINING, String::new()),
        NodeStatus::Unhealthy { message } => (wire::node_status::Kind::KIND_UNHEALTHY, message),
    };
    wire::NodeStatus {
        kind: buffa::EnumValue::from(kind),
        message,
        ..Default::default()
    }
}

pub fn node_status_from_proto(status: wire::NodeStatus) -> Result<NodeStatus, String> {
    match status
        .kind
        .as_known()
        .unwrap_or(wire::node_status::Kind::KIND_UNSPECIFIED)
    {
        wire::node_status::Kind::KIND_READY => Ok(NodeStatus::Ready),
        wire::node_status::Kind::KIND_BUSY => Ok(NodeStatus::Busy),
        wire::node_status::Kind::KIND_DRAINING => Ok(NodeStatus::Draining),
        wire::node_status::Kind::KIND_UNHEALTHY => Ok(NodeStatus::Unhealthy {
            message: status.message,
        }),
        wire::node_status::Kind::KIND_UNSPECIFIED => Ok(NodeStatus::Ready),
    }
}
