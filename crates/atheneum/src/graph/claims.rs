use anyhow::Result;
use chrono::Utc;

use super::AtheneumGraph;
use crate::db::claims::{
    audit_claims_for_project, get_claims_for_entity, insert_grounded_claim, list_all_claims,
    list_claims_for_project, list_stale_entity_ids, update_claim_status, ClaimAuditReport,
    GroundedClaim,
};

impl AtheneumGraph {
    /// Pin or update a grounded claim referencing concrete source code or verification receipt.
    pub fn pin_grounded_claim(&self, claim: &GroundedClaim) -> Result<()> {
        self.with_raw_connection(|conn| insert_grounded_claim(conn, claim))
    }

    /// Retrieve all grounded claims attached to a specific graph entity.
    pub fn get_claims_for_entity(&self, entity_id: i64) -> Result<Vec<GroundedClaim>> {
        self.with_raw_connection(|conn| get_claims_for_entity(conn, entity_id))
    }

    /// List grounded claims, optionally filtered by project.
    pub fn list_claims(&self, project: Option<&str>) -> Result<Vec<GroundedClaim>> {
        self.with_raw_connection(|conn| match project {
            Some(p) => list_claims_for_project(conn, p),
            None => list_all_claims(conn),
        })
    }

    /// Update a claim's status ('verified', 'stale', 'invalid').
    pub fn update_claim_status(&self, claim_id: &str, status: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        self.with_raw_connection(|conn| update_claim_status(conn, claim_id, status, &now))
    }

    /// Retrieve unique entity IDs that have at least one non-verified (stale/invalid) claim.
    pub fn list_stale_entity_ids(&self, project: &str) -> Result<Vec<i64>> {
        self.with_raw_connection(|conn| list_stale_entity_ids(conn, project))
    }

    /// Audit all grounded claims for a project and compute summary statistics.
    pub fn audit_claims(&self, project: &str) -> Result<ClaimAuditReport> {
        self.with_raw_connection(|conn| audit_claims_for_project(conn, project))
    }

    /// Verify all claims for a project against live files on disk under `repo_root`.
    /// If `fix` is true, updates the claim status in the database.
    pub fn verify_project_claims(
        &self,
        repo_root: &std::path::Path,
        project: &str,
        fix: bool,
    ) -> Result<ClaimAuditReport> {
        let claims = self.list_claims(Some(project))?;
        let now = Utc::now().to_rfc3339();

        for claim in &claims {
            let file_target = repo_root.join(&claim.file_path);
            let new_status = if !file_target.exists() {
                "invalid"
            } else {
                match crate::graph::hashing::compute_file_sha256(&file_target) {
                    Ok(live_hash) => {
                        if live_hash == claim.ast_hash {
                            "verified"
                        } else {
                            "stale"
                        }
                    }
                    Err(_) => "invalid",
                }
            };

            if fix && (new_status != claim.status || claim.status != "verified") {
                self.with_raw_connection(|conn| {
                    update_claim_status(conn, &claim.id, new_status, &now)
                })?;
            }
        }

        self.audit_claims(project)
    }
}
