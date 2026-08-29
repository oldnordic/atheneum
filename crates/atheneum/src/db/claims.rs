use anyhow::Result;
use rusqlite::{params, Connection, Transaction};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GroundedClaim {
    pub id: String,
    pub entity_id: i64,
    pub project: String,
    pub file_path: String,
    pub symbol_name: Option<String>,
    pub ast_hash: String,
    pub receipt_hash: Option<String>,
    pub status: String,
    pub created_at: String,
    pub last_verified_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ClaimAuditReport {
    pub project: String,
    pub total_claims: usize,
    pub verified_claims: usize,
    pub stale_claims: usize,
    pub invalid_claims: usize,
    pub stale_entity_ids: Vec<i64>,
}

pub(crate) fn migrate_v14_grounded_claims(tx: &Transaction<'_>) -> Result<()> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS grounded_claims (
            id TEXT PRIMARY KEY NOT NULL,
            entity_id INTEGER NOT NULL,
            project TEXT NOT NULL,
            file_path TEXT NOT NULL,
            symbol_name TEXT,
            ast_hash TEXT NOT NULL,
            receipt_hash TEXT,
            status TEXT NOT NULL DEFAULT 'verified',
            created_at TEXT NOT NULL,
            last_verified_at TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_grounded_claims_project_status ON grounded_claims(project, status);
        CREATE INDEX IF NOT EXISTS idx_grounded_claims_entity ON grounded_claims(entity_id);",
    )?;
    Ok(())
}

pub(crate) fn insert_grounded_claim(conn: &Connection, claim: &GroundedClaim) -> Result<()> {
    conn.execute(
        "INSERT INTO grounded_claims (
            id, entity_id, project, file_path, symbol_name, ast_hash, receipt_hash, status, created_at, last_verified_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        ON CONFLICT(id) DO UPDATE SET
            entity_id = excluded.entity_id,
            project = excluded.project,
            file_path = excluded.file_path,
            symbol_name = excluded.symbol_name,
            ast_hash = excluded.ast_hash,
            receipt_hash = excluded.receipt_hash,
            status = excluded.status,
            last_verified_at = excluded.last_verified_at",
        params![
            claim.id,
            claim.entity_id,
            claim.project,
            claim.file_path,
            claim.symbol_name,
            claim.ast_hash,
            claim.receipt_hash,
            claim.status,
            claim.created_at,
            claim.last_verified_at,
        ],
    )?;
    Ok(())
}

pub(crate) fn get_claims_for_entity(
    conn: &Connection,
    entity_id: i64,
) -> Result<Vec<GroundedClaim>> {
    let mut stmt = conn.prepare(
        "SELECT id, entity_id, project, file_path, symbol_name, ast_hash, receipt_hash, status, created_at, last_verified_at
         FROM grounded_claims WHERE entity_id = ?1 ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map(params![entity_id], |row| {
        Ok(GroundedClaim {
            id: row.get(0)?,
            entity_id: row.get(1)?,
            project: row.get(2)?,
            file_path: row.get(3)?,
            symbol_name: row.get(4)?,
            ast_hash: row.get(5)?,
            receipt_hash: row.get(6)?,
            status: row.get(7)?,
            created_at: row.get(8)?,
            last_verified_at: row.get(9)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub(crate) fn list_claims_for_project(
    conn: &Connection,
    project: &str,
) -> Result<Vec<GroundedClaim>> {
    let mut stmt = conn.prepare(
        "SELECT id, entity_id, project, file_path, symbol_name, ast_hash, receipt_hash, status, created_at, last_verified_at
         FROM grounded_claims WHERE project = ?1 ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map(params![project], |row| {
        Ok(GroundedClaim {
            id: row.get(0)?,
            entity_id: row.get(1)?,
            project: row.get(2)?,
            file_path: row.get(3)?,
            symbol_name: row.get(4)?,
            ast_hash: row.get(5)?,
            receipt_hash: row.get(6)?,
            status: row.get(7)?,
            created_at: row.get(8)?,
            last_verified_at: row.get(9)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub(crate) fn list_all_claims(conn: &Connection) -> Result<Vec<GroundedClaim>> {
    let mut stmt = conn.prepare(
        "SELECT id, entity_id, project, file_path, symbol_name, ast_hash, receipt_hash, status, created_at, last_verified_at
         FROM grounded_claims ORDER BY created_at ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(GroundedClaim {
            id: row.get(0)?,
            entity_id: row.get(1)?,
            project: row.get(2)?,
            file_path: row.get(3)?,
            symbol_name: row.get(4)?,
            ast_hash: row.get(5)?,
            receipt_hash: row.get(6)?,
            status: row.get(7)?,
            created_at: row.get(8)?,
            last_verified_at: row.get(9)?,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub(crate) fn update_claim_status(
    conn: &Connection,
    claim_id: &str,
    status: &str,
    last_verified: &str,
) -> Result<()> {
    conn.execute(
        "UPDATE grounded_claims SET status = ?1, last_verified_at = ?2 WHERE id = ?3",
        params![status, last_verified, claim_id],
    )?;
    Ok(())
}

pub(crate) fn list_stale_entity_ids(conn: &Connection, project: &str) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT entity_id FROM grounded_claims WHERE project = ?1 AND status != 'verified' ORDER BY entity_id ASC",
    )?;
    let rows = stmt.query_map(params![project], |row| row.get::<_, i64>(0))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub(crate) fn audit_claims_for_project(
    conn: &Connection,
    project: &str,
) -> Result<ClaimAuditReport> {
    let claims = list_claims_for_project(conn, project)?;
    let mut verified_count = 0;
    let mut stale_count = 0;
    let mut invalid_count = 0;
    let mut stale_entities = std::collections::BTreeSet::new();

    for c in &claims {
        match c.status.as_str() {
            "verified" => verified_count += 1,
            "stale" => {
                stale_count += 1;
                stale_entities.insert(c.entity_id);
            }
            _ => {
                invalid_count += 1;
                stale_entities.insert(c.entity_id);
            }
        }
    }

    Ok(ClaimAuditReport {
        project: project.to_string(),
        total_claims: claims.len(),
        verified_claims: verified_count,
        stale_claims: stale_count,
        invalid_claims: invalid_count,
        stale_entity_ids: stale_entities.into_iter().collect(),
    })
}
