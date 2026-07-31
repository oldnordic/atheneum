//! Envoy HTTP passthrough — the event tool's adapter. Pattern copied from
//! envoy-mcp's HttpBackend (separate repo, not a workspace dependency —
//! same rationale as subprocess.rs for code_query).

use anyhow::{anyhow, Result};
use serde_json::Value;
use std::time::Duration;

const ENVOY_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy)]
pub enum EnvoyVerb {
    Send,
    Claim,
    Heartbeat,
    CreateDependency,
}

impl EnvoyVerb {
    pub fn path(self) -> &'static str {
        match self {
            EnvoyVerb::Send => "/messages/send",
            EnvoyVerb::Claim => "/handoffs/claim",
            EnvoyVerb::Heartbeat => "/agents/heartbeat",
            EnvoyVerb::CreateDependency => "/dependencies",
        }
    }
}

impl std::str::FromStr for EnvoyVerb {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "send" => Ok(EnvoyVerb::Send),
            "claim" => Ok(EnvoyVerb::Claim),
            "heartbeat" => Ok(EnvoyVerb::Heartbeat),
            "create_dependency" => Ok(EnvoyVerb::CreateDependency),
            _ => Err(()),
        }
    }
}

pub struct EnvoyClient {
    client: reqwest::Client,
    base_url: String,
}

impl EnvoyClient {
    pub fn new(base_url: String) -> Self {
        Self {
            client: reqwest::Client::new(),
            base_url: base_url.trim_end_matches('/').to_string(),
        }
    }

    pub async fn call(&self, verb: EnvoyVerb, payload: Value) -> Result<Value> {
        let url = format!("{}{}", self.base_url, verb.path());
        let send = self.client.post(&url).json(&payload).send();
        let resp = tokio::time::timeout(ENVOY_TIMEOUT, send)
            .await
            .map_err(|_| anyhow!("envoy call to {url} timed out after {ENVOY_TIMEOUT:?}"))??;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow!("HTTP {status} from {url}: {text}"));
        }
        Ok(resp.json().await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn call_returns_err_when_envoy_unreachable() {
        // Port 9 is the "discard" service — connection refused/unreachable
        // in any real environment, standing in for envoy being down.
        let client = EnvoyClient::new("http://127.0.0.1:9".to_string());
        let result = client
            .call(
                EnvoyVerb::Heartbeat,
                serde_json::json!({"agent_id": "test"}),
            )
            .await;
        assert!(
            result.is_err(),
            "expected connection failure, got {result:?}"
        );
    }

    #[test]
    fn verb_maps_to_expected_path() {
        assert_eq!(EnvoyVerb::Send.path(), "/messages/send");
        assert_eq!(EnvoyVerb::Claim.path(), "/handoffs/claim");
        assert_eq!(EnvoyVerb::Heartbeat.path(), "/agents/heartbeat");
        assert_eq!(EnvoyVerb::CreateDependency.path(), "/dependencies");
    }
}
