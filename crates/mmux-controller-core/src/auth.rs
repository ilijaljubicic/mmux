use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum NodeWireAuthMode {
    Token,
    Mtls,
    Unauthenticated,
}

impl NodeWireAuthMode {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "token" => Ok(Self::Token),
            "mtls" => Ok(Self::Mtls),
            "unauthenticated" => Ok(Self::Unauthenticated),
            other => Err(format!(
                "unsupported wire auth mode '{}'; expected token, mtls, or unauthenticated",
                other
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Token => "token",
            Self::Mtls => "mtls",
            Self::Unauthenticated => "unauthenticated",
        }
    }

    pub fn allows_token(self) -> bool {
        matches!(self, Self::Token)
    }

    pub fn allows_mtls(self) -> bool {
        matches!(self, Self::Mtls)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum NodeWireIdentitySource {
    Mtls,
    Runtime(String),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NodeWireIdentity {
    pub node_id: String,
    pub source: NodeWireIdentitySource,
}

impl NodeWireIdentity {
    pub fn mtls(node_id: impl Into<String>) -> Self {
        Self {
            node_id: node_id.into(),
            source: NodeWireIdentitySource::Mtls,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum NodeWireAuthMethod {
    BearerToken,
    Mtls,
    Unauthenticated,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NodeWireAuthContext {
    pub method: NodeWireAuthMethod,
    pub identity: Option<NodeWireIdentity>,
}

impl NodeWireAuthContext {
    pub fn bearer_token() -> Self {
        Self {
            method: NodeWireAuthMethod::BearerToken,
            identity: None,
        }
    }

    pub fn mtls(identity: NodeWireIdentity) -> Self {
        Self {
            method: NodeWireAuthMethod::Mtls,
            identity: Some(identity),
        }
    }

    pub fn unauthenticated() -> Self {
        Self {
            method: NodeWireAuthMethod::Unauthenticated,
            identity: None,
        }
    }

    pub fn require_node_id(&self, requested_node_id: &str) -> Result<(), String> {
        if let Some(identity) = &self.identity {
            if identity.node_id != requested_node_id {
                return Err(format!(
                    "authenticated node identity '{}' cannot act as '{}'",
                    identity.node_id, requested_node_id
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct NodeWireAuthPolicy {
    pub mode: NodeWireAuthMode,
}

impl NodeWireAuthPolicy {
    pub fn authenticate(
        &self,
        token_valid: bool,
        mtls_identity: Option<NodeWireIdentity>,
    ) -> Result<NodeWireAuthContext, String> {
        if self.mode.allows_mtls() {
            if let Some(identity) = mtls_identity {
                return Ok(NodeWireAuthContext::mtls(identity));
            }
        }

        if self.mode.allows_token() && token_valid {
            return Ok(NodeWireAuthContext::bearer_token());
        }

        if self.mode == NodeWireAuthMode::Unauthenticated {
            return Ok(NodeWireAuthContext::unauthenticated());
        }

        Err(match self.mode {
            NodeWireAuthMode::Token => "node wire RPC requires a valid bearer token".into(),
            NodeWireAuthMode::Mtls => "node wire RPC requires verified mTLS identity".into(),
            NodeWireAuthMode::Unauthenticated => unreachable!(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_policy_accepts_token() {
        let policy = NodeWireAuthPolicy {
            mode: NodeWireAuthMode::Token,
        };

        assert_eq!(
            policy.authenticate(true, None).unwrap().method,
            NodeWireAuthMethod::BearerToken
        );
        assert!(policy
            .authenticate(false, Some(NodeWireIdentity::mtls("n1")))
            .is_err());
        assert!(policy.authenticate(false, None).is_err());
    }

    #[test]
    fn auth_policy_accepts_mtls() {
        let policy = NodeWireAuthPolicy {
            mode: NodeWireAuthMode::Mtls,
        };

        let mtls = policy
            .authenticate(true, Some(NodeWireIdentity::mtls("n1")))
            .unwrap();
        assert_eq!(mtls.method, NodeWireAuthMethod::Mtls);
        assert_eq!(mtls.identity.unwrap().node_id, "n1");
        assert!(policy.authenticate(true, None).is_err());
        assert!(policy.authenticate(false, None).is_err());
    }

    #[test]
    fn auth_policy_accepts_explicit_unauthenticated_mode() {
        let policy = NodeWireAuthPolicy {
            mode: NodeWireAuthMode::Unauthenticated,
        };

        assert_eq!(
            policy.authenticate(false, None).unwrap().method,
            NodeWireAuthMethod::Unauthenticated
        );
    }

    #[test]
    fn auth_context_rejects_node_identity_mismatch() {
        let context = NodeWireAuthContext::mtls(NodeWireIdentity::mtls("n1"));

        assert!(context.require_node_id("n1").is_ok());
        assert!(context.require_node_id("n2").is_err());
    }
}
