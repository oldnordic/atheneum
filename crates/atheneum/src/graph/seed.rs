use anyhow::Result;
use serde::{Serialize, Deserialize};
use sqlitegraph::GraphEntity;

use super::{AtheneumGraph, EntityType};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeedMemory {
    pub instructions: String,
    pub token_estimate: usize,
}

impl AtheneumGraph {
    pub fn seed_memory(&self, project: Option<&str>, token_budget: usize) -> Result<SeedMemory> {
        let all_entities = self.with_raw_connection(|conn| {
            let mut stmt = conn.prepare("SELECT id, kind, name, file_path, data FROM graph_entities")?;
            let rows = stmt.query_map([], |row| {
                Ok(GraphEntity {
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    name: row.get(2)?,
                    file_path: row.get(3)?,
                    data: serde_json::from_str(row.get_ref(4)?.as_str()?)
                        .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?,
                })
            })?;
            let mut out = Vec::new();
            for r in rows {
                out.push(r?);
            }
            Ok(out)
        })?;

        let mut concepts: Vec<GraphEntity> = Vec::new();
        let mut memories: Vec<GraphEntity> = Vec::new();

        for entity in all_entities {
            // Filter noise kinds
            if entity.kind == "ToolCall"
                || entity.kind == "ReasoningLog"
                || entity.kind == "TestRun"
                || entity.kind == "Event"
                || entity.kind == "Session"
                || entity.kind == "QueryTrace"
            {
                continue;
            }
            // Filter superseded rows
            if entity.data.get("superseded_at").is_some() {
                continue;
            }
            // Filter project
            if let Some(proj) = project {
                let ent_proj = entity.data.get("project_id").and_then(|v| v.as_str());
                if ent_proj != Some(proj) {
                    continue;
                }
            }

            if entity.kind == EntityType::Memory.as_str() {
                memories.push(entity);
            } else {
                concepts.push(entity);
            }
        }

        // Sort memories by updated_at / created_at descending
        memories.sort_by(|a, b| {
            let ta = a.data.get("updated_at").or_else(|| a.data.get("created_at")).and_then(|v| v.as_str()).unwrap_or("");
            let tb = b.data.get("updated_at").or_else(|| b.data.get("created_at")).and_then(|v| v.as_str()).unwrap_or("");
            tb.cmp(ta)
        });

        // Group concepts by (kind, scope)
        let mut groups: std::collections::HashMap<(String, String), Vec<GraphEntity>> = std::collections::HashMap::new();
        for c in concepts {
            let scope = c.data.get("scope").and_then(|v| v.as_str()).unwrap_or("global").to_string();
            groups.entry((c.kind.clone(), scope)).or_default().push(c);
        }

        let mut sorted_keys: Vec<_> = groups.keys().cloned().collect();
        sorted_keys.sort();

        let mut instructions = String::new();
        let mut current_tokens = 0;

        for key in &sorted_keys {
            let group_entities = &groups[key];
            let group_header = format!("### {} (Scope: {})\n", key.0, key.1);
            let header_tokens = group_header.len() / 4;
            if current_tokens + header_tokens > token_budget {
                instructions.push_str("... (more concepts truncated)\n");
                current_tokens += 30;
                break;
            }
            instructions.push_str(&group_header);
            current_tokens += header_tokens;

            let mut truncated_count = 0;
            for entity in group_entities {
                let summary = entity.data.get("summary").and_then(|v| v.as_str())
                    .or_else(|| entity.data.get("content").and_then(|v| v.as_str()))
                    .or_else(|| entity.data.get("body").and_then(|v| v.as_str()))
                    .unwrap_or("");
                let short_summary = if summary.len() > 80 {
                    format!("{}...", &summary[..80])
                } else {
                    summary.to_string()
                };
                let line = format!("- {}: {}\n", entity.name, short_summary);
                let line_tokens = line.len() / 4;
                if current_tokens + line_tokens > token_budget {
                    truncated_count = group_entities.len() - (group_entities.iter().position(|e| e.id == entity.id).unwrap());
                    break;
                }
                instructions.push_str(&line);
                current_tokens += line_tokens;
            }
            if truncated_count > 0 {
                instructions.push_str(&format!("... ({} more concepts truncated)\n", truncated_count));
                break;
            }
        }

        if !memories.is_empty() && current_tokens < token_budget {
            let memories_header = "### Recent Memories\n".to_string();
            let header_tokens = memories_header.len() / 4;
            if current_tokens + header_tokens <= token_budget {
                instructions.push_str(&memories_header);
                current_tokens += header_tokens;

                let max_memories = memories.len().min(10);
                let mut truncated_count = 0;
                for i in 0..max_memories {
                    let m = &memories[i];
                    let content = m.data.get("content").and_then(|v| v.as_str()).unwrap_or("");
                    let short_content = if content.len() > 80 {
                        format!("{}...", &content[..80])
                    } else {
                        content.to_string()
                    };
                    let line = format!("- {}: {}\n", m.name, short_content);
                    let line_tokens = line.len() / 4;
                    if current_tokens + line_tokens > token_budget {
                        truncated_count = memories.len() - i;
                        break;
                    }
                    instructions.push_str(&line);
                    current_tokens += line_tokens;
                }
                let total_truncated = truncated_count + memories.len().saturating_sub(10);
                if total_truncated > 0 {
                    instructions.push_str(&format!("... ({} more memories truncated)\n", total_truncated));
                }
            }
        }

        Ok(SeedMemory {
            instructions,
            token_estimate: current_tokens,
        })
    }
}
