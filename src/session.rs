use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::warn;

use crate::util::{config_dir, uuid};

#[derive(Serialize, Deserialize)]
struct LockFile {
    pid: u32,
    started_at: u64,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct JournalEntry {
    pub id: String,
    pub op: String,
    pub site: String,
    pub subject: String,
    pub status: String, // "pending" | "done" | "failed"
    pub ts: u64,
}

pub struct Session {
    pub pid: u32,
    pub started_at: u64,
    lock_path: PathBuf,
    journal_path: PathBuf,
}

impl Session {
    pub fn acquire() -> Result<Self> {
        let dir = config_dir();
        std::fs::create_dir_all(&dir)?;
        let lock_path = dir.join("session.lock");
        let journal_path = dir.join("journal.jsonl");

        // Check for existing lock
        if let Ok(raw) = std::fs::read_to_string(&lock_path) {
            if let Ok(lock) = serde_json::from_str::<LockFile>(&raw) {
                // Check if that PID is still alive
                let alive = std::process::Command::new("kill")
                    .args(["-0", &lock.pid.to_string()])
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false);
                if alive {
                    warn!("Orphan MCP server (PID {}) detected — sending SIGTERM", lock.pid);
                    let _ = std::process::Command::new("kill")
                        .args(["-TERM", &lock.pid.to_string()])
                        .output();
                    std::thread::sleep(std::time::Duration::from_secs(2));
                }
            }
        }

        let pid = std::process::id();
        let started_at = now_secs();
        let lock = LockFile { pid, started_at };
        std::fs::write(&lock_path, serde_json::to_string(&lock)?)?;

        Ok(Self { pid, started_at, lock_path, journal_path })
    }

    /// Append a pending entry to the journal. Returns the entry id.
    pub fn record(&self, op: &str, site: &str, subject: &str) -> String {
        let entry = JournalEntry {
            id: uuid(),
            op: op.to_string(),
            site: site.to_string(),
            subject: subject.to_string(),
            status: "pending".to_string(),
            ts: now_secs(),
        };
        let id = entry.id.clone();
        if let Ok(line) = serde_json::to_string(&entry) {
            if let Err(e) = append_line(&self.journal_path, &line) {
                warn!("Journal write failed: {e}");
            }
        }
        id
    }

    /// Mark a journal entry as done.
    pub fn complete(&self, id: &str) {
        if let Err(e) = rewrite_status(&self.journal_path, id, "done") {
            warn!("Journal complete failed: {e}");
        }
    }

    pub fn pending_ops(&self) -> Vec<JournalEntry> {
        read_journal(&self.journal_path)
            .into_iter()
            .filter(|e| e.status == "pending")
            .collect()
    }

    pub fn recent_ops(&self, n: usize) -> Vec<JournalEntry> {
        let all = read_journal(&self.journal_path);
        let done: Vec<_> = all.into_iter().filter(|e| e.status == "done").collect();
        done.into_iter().rev().take(n).collect()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.lock_path);
    }
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn append_line(path: &PathBuf, line: &str) -> Result<()> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
    writeln!(f, "{line}")?;
    Ok(())
}

fn rewrite_status(path: &PathBuf, id: &str, new_status: &str) -> Result<()> {
    let content = std::fs::read_to_string(path).unwrap_or_default();
    let updated = content.lines().map(|line| {
        if line.contains(id) {
            line.replace("\"status\":\"pending\"", &format!("\"status\":\"{new_status}\""))
        } else {
            line.to_string()
        }
    }).collect::<Vec<_>>().join("\n") + "\n";
    std::fs::write(path, updated)?;
    Ok(())
}

fn read_journal(path: &PathBuf) -> Vec<JournalEntry> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}
