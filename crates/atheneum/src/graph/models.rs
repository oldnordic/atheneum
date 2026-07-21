use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub name: String,
    pub loaded: bool,
    pub size_bytes: Option<u64>,
    pub provider: String,
}

impl crate::AtheneumGraph {
    /// Discovers available models on local Ollama or llama.cpp servers.
    pub fn discover_available_models(&self) -> Result<Vec<ModelInfo>> {
        let mut models = Vec::new();

        // 1. Attempt to query Ollama at the default URL or environment override
        let ollama_url =
            std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "http://127.0.0.1:11434".to_string());
        if let Ok(resp) = ureq::get(&format!("{}/api/ps", ollama_url))
            .timeout(Duration::from_secs(1))
            .call()
        {
            #[derive(Deserialize)]
            struct OllamaPsModel {
                name: String,
                size: u64,
            }
            #[derive(Deserialize)]
            struct OllamaPsResponse {
                models: Vec<OllamaPsModel>,
            }
            if let Ok(ps_res) = resp.into_json::<OllamaPsResponse>() {
                for m in ps_res.models {
                    models.push(ModelInfo {
                        name: m.name,
                        loaded: true,
                        size_bytes: Some(m.size),
                        provider: "ollama".to_string(),
                    });
                }
            }
        }

        // 2. Attempt to query llama.cpp at default URL
        let llamacpp_url =
            std::env::var("LLAMACPP_HOST").unwrap_or_else(|_| "http://127.0.0.1:8080".to_string());
        if let Ok(resp) = ureq::get(&format!("{}/health", llamacpp_url))
            .timeout(Duration::from_secs(1))
            .call()
        {
            #[derive(Deserialize)]
            struct LlamaCppHealth {
                status: String,
            }
            if let Ok(health) = resp.into_json::<LlamaCppHealth>() {
                if health.status == "ok" {
                    if let Ok(props_resp) = ureq::get(&format!("{}/props", llamacpp_url)).call() {
                        #[derive(Deserialize)]
                        struct LlamaCppProps {
                            model_path: String,
                        }
                        if let Ok(props) = props_resp.into_json::<LlamaCppProps>() {
                            let filename = std::path::Path::new(&props.model_path)
                                .file_name()
                                .and_then(|f| f.to_str())
                                .unwrap_or(&props.model_path)
                                .to_string();
                            models.push(ModelInfo {
                                name: filename,
                                loaded: true,
                                size_bytes: None,
                                provider: "llamacpp".to_string(),
                            });
                        }
                    }
                }
            }
        }

        Ok(models)
    }

    pub fn apply_swap_guard(
        &self,
        preferred_model: &str,
        mode: crate::config::SwapGuardMode,
    ) -> Result<String, crate::graph::types::AtheneumError> {
        let loaded = self.discover_available_models().unwrap_or_default();
        let is_loaded = loaded
            .iter()
            .any(|m| m.name == preferred_model || m.name.starts_with(preferred_model));

        if is_loaded {
            return Ok(preferred_model.to_string());
        }

        match mode {
            crate::config::SwapGuardMode::Strict => {
                Err(crate::graph::types::AtheneumError::ModelSwapBlocked {
                    model: preferred_model.to_string(),
                })
            }
            crate::config::SwapGuardMode::Adapt => {
                if let Some(first_loaded) = loaded.first() {
                    Ok(first_loaded.name.clone())
                } else {
                    Ok(preferred_model.to_string())
                }
            }
            crate::config::SwapGuardMode::Fallback => {
                Err(crate::graph::types::AtheneumError::ModelSwapBlocked {
                    model: preferred_model.to_string(),
                })
            }
        }
    }

    pub fn pin_entity(&self, id: i64) -> Result<()> {
        let mut entity = self.get_entity(id)?;
        if let Some(obj) = entity.data.as_object_mut() {
            obj.insert("pinned".to_string(), serde_json::Value::Bool(true));
        }
        self.update_entity_data(id, &entity.data)?;
        Ok(())
    }

    pub fn unpin_entity(&self, id: i64) -> Result<()> {
        let mut entity = self.get_entity(id)?;
        if let Some(obj) = entity.data.as_object_mut() {
            obj.insert("pinned".to_string(), serde_json::Value::Bool(false));
        }
        self.update_entity_data(id, &entity.data)?;
        Ok(())
    }
}
