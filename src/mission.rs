// tools/hermes-engine/src/mission.rs
//
// Mission lifecycle — state machine, event log, persistence.

use anyhow::Result;
use chrono::Utc;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

// ── Status state machine ─────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MissionStatus {
    Preflight,
    Active,
    Landing,
    Completed,
    Abandoned,
}

impl MissionStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Preflight => "preflight",
            Self::Active => "active",
            Self::Landing => "landing",
            Self::Completed => "completed",
            Self::Abandoned => "abandoned",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "preflight" => Some(Self::Preflight),
            "active" => Some(Self::Active),
            "landing" => Some(Self::Landing),
            "completed" => Some(Self::Completed),
            "abandoned" => Some(Self::Abandoned),
            _ => None,
        }
    }

    /// Returns `true` if `from → to` is a valid transition.
    pub fn can_transition(from: &Self, to: &Self) -> bool {
        matches!(
            (from, to),
            (Self::Preflight, Self::Active)
                | (Self::Preflight, Self::Abandoned)
                | (Self::Active, Self::Landing)
                | (Self::Active, Self::Abandoned)
                | (Self::Landing, Self::Completed)
                | (Self::Landing, Self::Active)
                | (Self::Landing, Self::Abandoned)
        )
    }
}

// ── Mission struct ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mission {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub description: Option<String>,
    pub status: String,
    pub tags: Option<String>,
    pub checklist: Option<String>,
    pub diff: Option<String>,
    pub commit_range: Option<String>,
    pub repo_id: Option<String>,
    pub agent_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

// ── Log entry ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissionLogEntry {
    pub id: i64,
    pub mission_id: String,
    pub event_type: String,
    pub data: Option<Value>,
    pub created_at: String,
}

// ── Store ────────────────────────────────────────────────────────────────

pub struct MissionStore<'a> {
    conn: &'a Connection,
    project_id: &'a str,
}

impl<'a> MissionStore<'a> {
    pub fn new(conn: &'a Connection, project_id: &'a str) -> Self {
        Self { conn, project_id }
    }

    pub fn create(
        &self,
        title: &str,
        description: Option<&str>,
        tags: Option<&str>,
    ) -> Result<Mission> {
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().to_rfc3339();
        let tags_val = tags.unwrap_or("[]");
        self.conn.execute(
            "INSERT INTO missions (id, project_id, title, description, status, tags, created_at, updated_at)
             VALUES (?1,?2,?3,?4,'preflight',?5,?6,?7)",
            params![id, self.project_id, title, description, tags_val, now, now],
        )?;
        self.append_log(&id, "phase_enter", &json!({"phase": "preflight"}))?;

        self.get(&id)
    }

    pub fn get(&self, mission_id: &str) -> Result<Mission> {
        self.conn
            .query_row(
                "SELECT id, project_id, title, description, status, tags,
                        checklist, diff, commit_range, repo_id, agent_id,
                        created_at, updated_at
                 FROM missions WHERE id = ?1",
                params![mission_id],
                |row| {
                    Ok(Mission {
                        id: row.get(0)?,
                        project_id: row.get(1)?,
                        title: row.get(2)?,
                        description: row.get(3)?,
                        status: row.get(4)?,
                        tags: row.get(5)?,
                        checklist: row.get(6)?,
                        diff: row.get(7)?,
                        commit_range: row.get(8)?,
                        repo_id: row.get(9)?,
                        agent_id: row.get(10)?,
                        created_at: row.get(11)?,
                        updated_at: row.get(12)?,
                    })
                },
            )
            .map_err(|e| anyhow::anyhow!("mission not found: {e}"))
    }

    /// Transition status with state-machine validation.
    pub fn update_status(&self, mission_id: &str, new_status: &str) -> Result<Mission> {
        let current = self.get(mission_id)?;
        let from = MissionStatus::parse(&current.status)
            .ok_or_else(|| anyhow::anyhow!("invalid current status: {}", current.status))?;
        let to = MissionStatus::parse(new_status)
            .ok_or_else(|| anyhow::anyhow!("invalid target status: {new_status}"))?;
        anyhow::ensure!(
            MissionStatus::can_transition(&from, &to),
            "invalid transition: {} → {new_status}",
            current.status
        );

        let now = Utc::now().to_rfc3339();
        self.conn.execute(
            "UPDATE missions SET status = ?1, updated_at = ?2 WHERE id = ?3",
            params![new_status, now, mission_id],
        )?;
        self.append_log(mission_id, "phase_enter", &json!({"phase": new_status}))?;

        self.get(mission_id)
    }

    /// Update optional metadata fields (title, description, tags, checklist, diff, commit_range).
    pub fn update_metadata(&self, mission_id: &str, args: &Value) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let sets: Vec<(&str, &str)> = [
            ("title", args["title"].as_str()),
            ("description", args["description"].as_str()),
            ("tags", args["tags"].as_str()),
            ("checklist", args["checklist"].as_str()),
            ("diff", args["diff"].as_str()),
            ("commit_range", args["commit_range"].as_str()),
        ]
        .into_iter()
        .filter_map(|(col, v)| v.map(|val| (col, val)))
        .collect();

        for (col, val) in &sets {
            let sql = format!("UPDATE missions SET {col} = ?1, updated_at = ?2 WHERE id = ?3");
            self.conn.execute(&sql, params![val, now, mission_id])?;
        }
        Ok(())
    }

    pub fn list(&self, status: Option<&str>, limit: usize) -> Result<Vec<Mission>> {
        let (sql, pv) = if let Some(st) = status {
            (
                "SELECT id, project_id, title, description, status, tags,
                        checklist, diff, commit_range, repo_id, agent_id,
                        created_at, updated_at
                 FROM missions WHERE status = ?1
                 ORDER BY updated_at DESC LIMIT ?2",
                vec![
                    rusqlite::types::Value::from(st.to_string()),
                    rusqlite::types::Value::from(limit as i64),
                ],
            )
        } else {
            (
                "SELECT id, project_id, title, description, status, tags,
                        checklist, diff, commit_range, repo_id, agent_id,
                        created_at, updated_at
                 FROM missions
                 ORDER BY updated_at DESC LIMIT ?1",
                vec![rusqlite::types::Value::from(limit as i64)],
            )
        };
        let mut stmt = self.conn.prepare(sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(pv), |row| {
            Ok(Mission {
                id: row.get(0)?,
                project_id: row.get(1)?,
                title: row.get(2)?,
                description: row.get(3)?,
                status: row.get(4)?,
                tags: row.get(5)?,
                checklist: row.get(6)?,
                diff: row.get(7)?,
                commit_range: row.get(8)?,
                repo_id: row.get(9)?,
                agent_id: row.get(10)?,
                created_at: row.get(11)?,
                updated_at: row.get(12)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn get_log(&self, mission_id: &str) -> Result<Vec<MissionLogEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, mission_id, event_type, data, created_at
             FROM mission_log WHERE mission_id = ?1 ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(params![mission_id], |row| {
            let data_str: Option<String> = row.get(3)?;
            let data = data_str.and_then(|s| serde_json::from_str(&s).ok());
            Ok(MissionLogEntry {
                id: row.get(0)?,
                mission_id: row.get(1)?,
                event_type: row.get(2)?,
                data,
                created_at: row.get(4)?,
            })
        })?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }

    pub fn append_log(&self, mission_id: &str, event_type: &str, data: &Value) -> Result<()> {
        self.conn.execute(
            "INSERT INTO mission_log (mission_id, event_type, data) VALUES (?1, ?2, ?3)",
            params![mission_id, event_type, serde_json::to_string(data)?],
        )?;
        Ok(())
    }
}
