use anyhow::Result;
use serde::{Deserialize, Serialize};
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
            let mut stmt =
                conn.prepare("SELECT id, kind, name, file_path, data FROM graph_entities")?;
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

        let mut pinned_concepts = Vec::new();
        let mut normal_concepts = Vec::new();
        let mut pinned_memories = Vec::new();
        let mut normal_memories = Vec::new();

        for entity in all_entities {
            // Filter noise kinds. Agent/Call are structural graph-plumbing
            // entities (agent registrations, raw call-graph edges) — never
            // knowledge concepts a seed summary should surface.
            if entity.kind == "ToolCall"
                || entity.kind == "ReasoningLog"
                || entity.kind == "TestRun"
                || entity.kind == "Event"
                || entity.kind == "Session"
                || entity.kind == "QueryTrace"
                || entity.kind == "Agent"
                || entity.kind == "Call"
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

            // Skip entities with no renderable text — an empty summary
            // produces a bare "- name: " line that wastes budget for zero
            // information.
            let has_text = ["summary", "content", "body"].iter().any(|field| {
                entity
                    .data
                    .get(*field)
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| !s.trim().is_empty())
            });
            if !has_text {
                continue;
            }

            let is_pinned = entity
                .data
                .get("pinned")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            if entity.kind == EntityType::Memory.as_str() {
                if is_pinned {
                    pinned_memories.push(entity);
                } else {
                    normal_memories.push(entity);
                }
            } else {
                if is_pinned {
                    pinned_concepts.push(entity);
                } else {
                    normal_concepts.push(entity);
                }
            }
        }

        // Sort normal memories by updated_at / created_at descending
        normal_memories.sort_by(|a, b| {
            let ta = a
                .data
                .get("updated_at")
                .or_else(|| a.data.get("created_at"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let tb = b
                .data
                .get("updated_at")
                .or_else(|| b.data.get("created_at"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            tb.cmp(ta)
        });

        // Group normal concepts by (kind, scope)
        let mut groups: std::collections::HashMap<(String, String), Vec<GraphEntity>> =
            std::collections::HashMap::new();
        for c in normal_concepts {
            let scope = c
                .data
                .get("scope")
                .and_then(|v| v.as_str())
                .unwrap_or("global")
                .to_string();
            groups.entry((c.kind.clone(), scope)).or_default().push(c);
        }

        let mut sorted_keys: Vec<_> = groups.keys().cloned().collect();
        sorted_keys.sort();

        let mut instructions = String::new();
        let mut current_tokens = 0;

        // Render Pinned Concepts first
        if !pinned_concepts.is_empty() {
            instructions.push_str("### Pinned Concepts\n");
            current_tokens += "### Pinned Concepts\n".len() / 4;
            for entity in pinned_concepts {
                let summary = entity
                    .data
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .or_else(|| entity.data.get("content").and_then(|v| v.as_str()))
                    .or_else(|| entity.data.get("body").and_then(|v| v.as_str()))
                    .unwrap_or("");
                let short_summary = if summary.len() > 80 {
                    let end = summary
                        .char_indices()
                        .map(|(i, c)| i + c.len_utf8())
                        .take_while(|&i| i <= 80)
                        .last()
                        .unwrap_or(0);
                    format!("{}...", &summary[..end])
                } else {
                    summary.to_string()
                };
                let line = format!("- [PINNED] {}: {}\n", entity.name, short_summary);
                instructions.push_str(&line);
                current_tokens += line.len() / 4;
            }
        }

        // Render Pinned Memories next
        if !pinned_memories.is_empty() {
            instructions.push_str("### Pinned Memories\n");
            current_tokens += "### Pinned Memories\n".len() / 4;
            for m in pinned_memories {
                let content = m.data.get("content").and_then(|v| v.as_str()).unwrap_or("");
                let short_content = if content.len() > 80 {
                    let end = content
                        .char_indices()
                        .map(|(i, c)| i + c.len_utf8())
                        .take_while(|&i| i <= 80)
                        .last()
                        .unwrap_or(0);
                    format!("{}...", &content[..end])
                } else {
                    content.to_string()
                };
                let line = format!("- [PINNED MEMORY] {}: {}\n", m.name, short_content);
                instructions.push_str(&line);
                current_tokens += line.len() / 4;
            }
        }

        // Reserve a slice of the remaining budget for Recent Memories so
        // a large concept dump can't starve it out entirely (concepts are
        // rendered first and would otherwise always win the whole budget).
        let remaining_for_split = token_budget.saturating_sub(current_tokens);
        let memory_reserve = if normal_memories.is_empty() {
            0
        } else {
            (remaining_for_split / 3).clamp(60.min(remaining_for_split), remaining_for_split)
        };
        let concept_budget = token_budget.saturating_sub(memory_reserve);

        // Render Normal Concepts
        for key in &sorted_keys {
            let group_entities = &groups[key];
            let group_header = format!("### {} (Scope: {})\n", key.0, key.1);
            let header_tokens = group_header.len() / 4;
            if current_tokens + header_tokens > concept_budget {
                instructions.push_str("... (more concepts truncated)\n");
                current_tokens += 30;
                break;
            }
            instructions.push_str(&group_header);
            current_tokens += header_tokens;

            let mut truncated_count = 0;
            for entity in group_entities {
                let summary = entity
                    .data
                    .get("summary")
                    .and_then(|v| v.as_str())
                    .or_else(|| entity.data.get("content").and_then(|v| v.as_str()))
                    .or_else(|| entity.data.get("body").and_then(|v| v.as_str()))
                    .unwrap_or("");
                let short_summary = if summary.len() > 80 {
                    let end = summary
                        .char_indices()
                        .map(|(i, c)| i + c.len_utf8())
                        .take_while(|&i| i <= 80)
                        .last()
                        .unwrap_or(0);
                    format!("{}...", &summary[..end])
                } else {
                    summary.to_string()
                };
                let line = format!("- {}: {}\n", entity.name, short_summary);
                let line_tokens = line.len() / 4;
                if current_tokens + line_tokens > concept_budget {
                    truncated_count = group_entities.len()
                        - (group_entities
                            .iter()
                            .position(|e| e.id == entity.id)
                            .unwrap());
                    break;
                }
                instructions.push_str(&line);
                current_tokens += line_tokens;
            }
            if truncated_count > 0 {
                instructions.push_str(&format!(
                    "... ({} more concepts truncated)\n",
                    truncated_count
                ));
                break;
            }
        }

        // Render Normal Memories
        if !normal_memories.is_empty() && current_tokens < token_budget {
            let memories_header = "### Recent Memories\n".to_string();
            let header_tokens = memories_header.len() / 4;
            if current_tokens + header_tokens <= token_budget {
                instructions.push_str(&memories_header);
                current_tokens += header_tokens;

                let max_memories = normal_memories.len().min(10);
                let mut truncated_count = 0;
                for i in 0..max_memories {
                    let m = &normal_memories[i];
                    let content = m.data.get("content").and_then(|v| v.as_str()).unwrap_or("");
                    let short_content = if content.len() > 80 {
                        let end = content
                            .char_indices()
                            .map(|(i, c)| i + c.len_utf8())
                            .take_while(|&i| i <= 80)
                            .last()
                            .unwrap_or(0);
                        format!("{}...", &content[..end])
                    } else {
                        content.to_string()
                    };
                    let line = format!("- {}: {}\n", m.name, short_content);
                    let line_tokens = line.len() / 4;
                    if current_tokens + line_tokens > token_budget {
                        truncated_count = normal_memories.len() - i;
                        break;
                    }
                    instructions.push_str(&line);
                    current_tokens += line_tokens;
                }
                let total_truncated = truncated_count + normal_memories.len().saturating_sub(10);
                if total_truncated > 0 {
                    instructions.push_str(&format!(
                        "... ({} more memories truncated)\n",
                        total_truncated
                    ));
                }
            }
        }

        Ok(SeedMemory {
            instructions,
            token_estimate: current_tokens,
        })
    }
}
