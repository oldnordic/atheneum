use anyhow::Result;
use chrono::Utc;
use serde_json::{json, Value};
use sqlitegraph::GraphEntity;

use super::{AppliedKanbanUpdate, AtheneumGraph, BlockerType, RequirementStatus, TaskDetail};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum KanbanStatus {
    Todo,
    InProgress,
    Done,
    Blocked,
}

impl KanbanStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            KanbanStatus::Todo => "TODO",
            KanbanStatus::InProgress => "IN_PROGRESS",
            KanbanStatus::Done => "DONE",
            KanbanStatus::Blocked => "BLOCKED",
        }
    }

    pub(super) fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_uppercase().as_str() {
            "TODO" => Some(KanbanStatus::Todo),
            "IN_PROGRESS" | "IN-PROGRESS" | "INPROGRESS" => Some(KanbanStatus::InProgress),
            "DONE" => Some(KanbanStatus::Done),
            "BLOCKED" => Some(KanbanStatus::Blocked),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct KanbanUpdate {
    pub task_title: String,
    pub new_status: KanbanStatus,
}

impl AtheneumGraph {
    pub fn create_task(
        &self,
        title: &str,
        description: Option<&str>,
        project_id: Option<&str>,
    ) -> Result<i64> {
        let created_at = Utc::now().to_rfc3339();

        let title_s = title.to_string();
        let description_s = description.map(|s| s.to_string());
        let project_s = project_id.map(|s| s.to_string());
        let created_at_s = created_at.clone();
        let sql_id = self.with_raw_connection(|conn| {
            conn.execute(
                "INSERT INTO tasks
                    (title, description, status, project_id, metadata, created_at)
                 VALUES (?1, ?2, 'TODO', ?3, '{}', ?4)",
                rusqlite::params![title_s, description_s, project_s, created_at_s],
            )?;
            Ok(conn.last_insert_rowid())
        })?;

        let mut data = json!({
            "sql_id": sql_id,
            "title": title,
            "description": description,
            "status": KanbanStatus::Todo.as_str(),
            "created_at": created_at,
        });
        if let (Some(pid), Some(obj)) = (project_id, data.as_object_mut()) {
            obj.insert("project_id".to_string(), Value::String(pid.to_string()));
        }
        let entity = GraphEntity {
            id: 0,
            kind: "Task".to_string(),
            name: title.to_string(),
            file_path: None,
            data,
        };
        self.inner
            .insert_entity(&entity)
            .map_err(|e| anyhow::anyhow!("Failed to insert Task: {}", e))
    }

    pub fn update_task_status(&self, task_id: i64, status: KanbanStatus) -> Result<()> {
        let entity = self.get_entity(task_id)?;
        let sql_id = entity.data.get("sql_id").and_then(|v| v.as_i64());

        let now = Utc::now().to_rfc3339();

        if let Some(sql_id) = sql_id {
            let status_str = status.as_str().to_string();
            let ts = now.clone();
            self.with_raw_connection(|conn| {
                conn.execute(
                    "UPDATE tasks SET status = ?1, status_updated_at = ?2 WHERE id = ?3",
                    rusqlite::params![status_str, ts, sql_id],
                )?;
                Ok(())
            })?;
        }

        let mut data = entity.data;
        if let Some(obj) = data.as_object_mut() {
            obj.insert(
                "status".to_string(),
                Value::String(status.as_str().to_string()),
            );
            obj.insert("status_updated_at".to_string(), Value::String(now));
        }
        self.update_entity_data(task_id, &data)
    }

    pub fn find_task_by_title(&self, title: &str, project_id: Option<&str>) -> Result<Option<i64>> {
        let tasks = self.entities_by_kind("Task")?;
        Ok(tasks
            .into_iter()
            .find(|t| {
                let title_matches = t.data.get("title").and_then(|v| v.as_str()) == Some(title);
                let project_matches = match project_id {
                    None => true,
                    Some(pid) => t.data.get("project_id").and_then(|v| v.as_str()) == Some(pid),
                };
                title_matches && project_matches
            })
            .map(|t| t.id))
    }

    pub fn list_tasks_by_status(
        &self,
        status: KanbanStatus,
        project_id: Option<&str>,
    ) -> Result<Vec<GraphEntity>> {
        let want = status.as_str();
        Ok(self
            .entities_by_kind("Task")?
            .into_iter()
            .filter(|t| t.data.get("status").and_then(|v| v.as_str()) == Some(want))
            .filter(|t| match project_id {
                None => true,
                Some(pid) => t.data.get("project_id").and_then(|v| v.as_str()) == Some(pid),
            })
            .collect())
    }

    pub fn add_requirement(
        &self,
        task_id: i64,
        statement: &str,
        verification_method: Option<&str>,
    ) -> Result<i64> {
        let task_entity = self.get_entity(task_id)?;
        let task_sql_id = task_entity
            .data
            .get("sql_id")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Task graph_entity {} has no sql_id pointer; migration may not have run",
                    task_id
                )
            })?;

        let created_at = Utc::now().to_rfc3339();
        let statement_s = statement.to_string();
        let verification_s = verification_method.map(|s| s.to_string());
        let created_at_s = created_at.clone();
        let sql_id = self.with_raw_connection(|conn| {
            conn.execute(
                "INSERT INTO requirements
                    (task_id, statement, status, verification_method, created_at)
                 VALUES (?1, ?2, 'UNMET', ?3, ?4)",
                rusqlite::params![task_sql_id, statement_s, verification_s, created_at_s],
            )?;
            Ok(conn.last_insert_rowid())
        })?;

        let data = json!({
            "sql_id": sql_id,
            "task_id": task_id,
            "statement": statement,
            "status": RequirementStatus::Unmet.as_str(),
            "verification_method": verification_method,
            "created_at": created_at,
        });
        let entity = GraphEntity {
            id: 0,
            kind: "Requirement".to_string(),
            name: statement.to_string(),
            file_path: None,
            data,
        };
        self.inner
            .insert_entity(&entity)
            .map_err(|e| anyhow::anyhow!("Failed to insert Requirement: {}", e))
    }

    pub fn mark_requirement_met(&self, req_id: i64) -> Result<()> {
        let entity = self.get_entity(req_id)?;
        let sql_id = entity.data.get("sql_id").and_then(|v| v.as_i64());
        let now = Utc::now().to_rfc3339();

        if let Some(sql_id) = sql_id {
            let ts = now.clone();
            self.with_raw_connection(|conn| {
                conn.execute(
                    "UPDATE requirements SET status = 'MET', met_at = ?1 WHERE id = ?2",
                    rusqlite::params![ts, sql_id],
                )?;
                Ok(())
            })?;
        }

        let mut data = entity.data;
        if let Some(obj) = data.as_object_mut() {
            obj.insert(
                "status".to_string(),
                Value::String(RequirementStatus::Met.as_str().to_string()),
            );
            obj.insert("met_at".to_string(), Value::String(now));
        }
        self.update_entity_data(req_id, &data)
    }

    pub fn add_blocker(
        &self,
        task_id: i64,
        description: &str,
        blocker_type: BlockerType,
    ) -> Result<i64> {
        let task_entity = self.get_entity(task_id)?;
        let task_sql_id = task_entity
            .data
            .get("sql_id")
            .and_then(|v| v.as_i64())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "Task graph_entity {} has no sql_id pointer; migration may not have run",
                    task_id
                )
            })?;

        let created_at = Utc::now().to_rfc3339();
        let description_s = description.to_string();
        let blocker_kind = blocker_type.as_str().to_string();
        let created_at_s = created_at.clone();
        let sql_id = self.with_raw_connection(|conn| {
            conn.execute(
                "INSERT INTO blockers
                    (task_id, description, blocker_type, resolved_at, created_at)
                 VALUES (?1, ?2, ?3, NULL, ?4)",
                rusqlite::params![task_sql_id, description_s, blocker_kind, created_at_s],
            )?;
            Ok(conn.last_insert_rowid())
        })?;

        let data = json!({
            "sql_id": sql_id,
            "task_id": task_id,
            "description": description,
            "blocker_type": blocker_type.as_str(),
            "resolved_at": Value::Null,
            "created_at": created_at,
        });
        let entity = GraphEntity {
            id: 0,
            kind: "Blocker".to_string(),
            name: description.to_string(),
            file_path: None,
            data,
        };
        self.inner
            .insert_entity(&entity)
            .map_err(|e| anyhow::anyhow!("Failed to insert Blocker: {}", e))
    }

    pub fn resolve_blocker(&self, blocker_id: i64) -> Result<()> {
        let entity = self.get_entity(blocker_id)?;
        let sql_id = entity.data.get("sql_id").and_then(|v| v.as_i64());
        let now = Utc::now().to_rfc3339();

        if let Some(sql_id) = sql_id {
            let ts = now.clone();
            self.with_raw_connection(|conn| {
                conn.execute(
                    "UPDATE blockers SET resolved_at = ?1 WHERE id = ?2",
                    rusqlite::params![ts, sql_id],
                )?;
                Ok(())
            })?;
        }

        let mut data = entity.data;
        if let Some(obj) = data.as_object_mut() {
            obj.insert("resolved_at".to_string(), Value::String(now));
        }
        self.update_entity_data(blocker_id, &data)
    }

    pub fn get_task_with_details(&self, task_id: i64) -> Result<TaskDetail> {
        let task = self.get_entity(task_id)?;
        let requirements = self
            .entities_by_kind("Requirement")?
            .into_iter()
            .filter(|r| r.data.get("task_id").and_then(|v| v.as_i64()) == Some(task_id))
            .collect();
        let blockers = self
            .entities_by_kind("Blocker")?
            .into_iter()
            .filter(|b| b.data.get("task_id").and_then(|v| v.as_i64()) == Some(task_id))
            .collect();
        Ok(TaskDetail {
            task,
            requirements,
            blockers,
        })
    }

    pub fn apply_kanban_updates_from_journal(
        &self,
        journal_section_id: i64,
    ) -> Result<Vec<AppliedKanbanUpdate>> {
        let section = self.get_entity(journal_section_id)?;
        let project_id = section.data.get("project_id").and_then(|v| v.as_str());
        let updates = section
            .data
            .get("kanban_updates")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut applied = Vec::new();
        for update in updates {
            let Some(title) = update.get("task_title").and_then(|v| v.as_str()) else {
                continue;
            };
            let Some(status_str) = update.get("new_status").and_then(|v| v.as_str()) else {
                continue;
            };
            let Some(new_status) = KanbanStatus::parse(status_str) else {
                continue;
            };
            let Some(task_id) = self.find_task_by_title(title, project_id)? else {
                continue;
            };

            let task = self.get_entity(task_id)?;
            let previous_status = task
                .data
                .get("status")
                .and_then(|v| v.as_str())
                .and_then(KanbanStatus::parse)
                .unwrap_or(KanbanStatus::Todo);

            self.update_task_status(task_id, new_status)?;
            applied.push(AppliedKanbanUpdate {
                task_id,
                task_title: title.to_string(),
                previous_status,
                new_status,
            });
        }
        Ok(applied)
    }
}
