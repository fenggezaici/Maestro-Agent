//! SQLite is confined to one OS thread. Async callers only exchange messages.
use std::{path::PathBuf, sync::Mutex, thread, time::Duration};

use rusqlite::OptionalExtension;
use rusqlite::{params, Connection};
use tokio::sync::{mpsc, oneshot};

use crate::domain::{
    AppResult, Mode, PlanCard, PlanDecision, PlanDraft, PlanStatus, ProjectView, RunMetrics,
    RunStatus, RunView, SessionDetail, SessionView,
};

/// Every pre-v4 session migrates into the seeded default project, which can
/// neither be archived nor deleted.
pub const DEFAULT_PROJECT_ID: i64 = 1;

type Operation = Box<dyn FnOnce(&mut Connection) + Send>;

enum Request {
    Execute(Operation),
    Shutdown(oneshot::Sender<AppResult<()>>),
}

pub struct Database {
    sender: mpsc::Sender<Request>,
    thread: Mutex<Option<thread::JoinHandle<()>>>,
}

impl Database {
    pub async fn open(path: PathBuf) -> AppResult<Self> {
        let (sender, mut receiver) = mpsc::channel::<Request>(64);
        let (ready_tx, ready_rx) = oneshot::channel();
        let worker = thread::Builder::new()
            .name("maestro-sqlite".into())
            .spawn(move || {
                let mut connection = match initialize(path) {
                    Ok(connection) => {
                        let _ = ready_tx.send(Ok(()));
                        connection
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                        return;
                    }
                };
                while let Some(request) = receiver.blocking_recv() {
                    match request {
                        Request::Execute(operation) => operation(&mut connection),
                        Request::Shutdown(reply) => {
                            let result = connection.close().map_err(|(_, error)| error.to_string());
                            let _ = reply.send(result);
                            return;
                        }
                    }
                }
            })
            .map_err(|error| error.to_string())?;

        let database = Self {
            sender,
            thread: Mutex::new(Some(worker)),
        };
        ready_rx
            .await
            .map_err(|_| "Database initialization stopped".to_string())??;
        Ok(database)
    }

    async fn call<T, F>(&self, operation: F) -> AppResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&mut Connection) -> AppResult<T> + Send + 'static,
    {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.sender
            .send(Request::Execute(Box::new(move |connection| {
                let _ = reply_tx.send(operation(connection));
            })))
            .await
            .map_err(|_| "Database worker is closed".to_string())?;
        reply_rx
            .await
            .map_err(|_| "Database worker stopped before replying".to_string())?
    }

    #[cfg(test)]
    pub async fn create_run(&self, prompt: String, mode: Mode) -> AppResult<i64> {
        let session = self.create_session().await?;
        self.create_run_in_session(session.id, prompt, mode).await
    }

    #[cfg(test)]
    pub async fn create_session(&self) -> AppResult<SessionView> {
        self.create_session_in_project(DEFAULT_PROJECT_ID).await
    }

    pub async fn create_session_in_project(&self, project_id: i64) -> AppResult<SessionView> {
        self.call(move |connection| {
            let changed = connection
                .execute(
                    "INSERT INTO sessions (title, project_id)
                     SELECT '新会话', ?1 WHERE EXISTS
                     (SELECT 1 FROM projects WHERE id = ?1 AND archived_at IS NULL)",
                    [project_id],
                )
                .map_err(db_error)?;
            if changed != 1 {
                return Err("Project is missing or archived".into());
            }
            read_session_view(connection, connection.last_insert_rowid())
        })
        .await
    }

    pub async fn set_session_archived(
        &self,
        session_id: i64,
        archived: bool,
    ) -> AppResult<SessionView> {
        self.call(move |connection| {
            let running: bool = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM runs WHERE session_id = ?1 AND status = 'running')",
                    [session_id],
                    |row| row.get(0),
                )
                .map_err(db_error)?;
            if archived && running {
                return Err("Cannot archive a session with an active run".into());
            }
            let changed = connection
                .execute(
                    "UPDATE sessions SET archived_at = CASE WHEN ?2 THEN unixepoch() ELSE NULL END
                     WHERE id = ?1",
                    params![session_id, archived],
                )
                .map_err(db_error)?;
            if changed != 1 {
                return Err("Session not found".into());
            }
            read_session_view(connection, session_id)
        })
        .await
    }

    pub async fn rename_session(
        &self,
        session_id: i64,
        title: String,
    ) -> AppResult<SessionView> {
        let title = title.trim().to_string();
        if title.is_empty() || title.chars().count() > 200 {
            return Err("Session title must contain 1–200 characters".into());
        }
        self.call(move |connection| {
            let changed = connection
                .execute(
                    "UPDATE sessions SET title = ?2, updated_at = unixepoch() WHERE id = ?1",
                    params![session_id, title],
                )
                .map_err(db_error)?;
            if changed != 1 {
                return Err("Session not found".into());
            }
            read_session_view(connection, session_id)
        })
        .await
    }

    pub async fn list_projects(&self) -> AppResult<Vec<ProjectView>> {
        self.call(|connection| {
            let mut statement = connection
                .prepare(
                    "SELECT id, name, path, (archived_at IS NOT NULL), created_at
                     FROM projects ORDER BY created_at, id",
                )
                .map_err(db_error)?;
            let rows = statement.query_map([], project_row).map_err(db_error)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(db_error)
        })
        .await
    }

    pub async fn create_project(
        &self,
        name: String,
        path: Option<String>,
    ) -> AppResult<ProjectView> {
        let name = name.trim().to_string();
        if name.is_empty() || name.chars().count() > 128 {
            return Err("Project name must contain 1–128 characters".into());
        }
        let path = path
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        if let Some(value) = &path {
            if value.chars().count() > 1024 {
                return Err("Project path is too long".into());
            }
            if !std::path::Path::new(value).is_dir() {
                return Err("Project path is not an existing directory".into());
            }
        }
        self.call(move |connection| {
            connection
                .execute(
                    "INSERT INTO projects (name, path) VALUES (?1, ?2)",
                    params![name, path],
                )
                .map_err(db_error)?;
            connection
                .query_row(
                    "SELECT id, name, path, (archived_at IS NOT NULL), created_at
                     FROM projects WHERE id = ?1",
                    [connection.last_insert_rowid()],
                    project_row,
                )
                .map_err(db_error)
        })
        .await
    }

    pub async fn set_project_archived(
        &self,
        project_id: i64,
        archived: bool,
    ) -> AppResult<ProjectView> {
        if project_id == DEFAULT_PROJECT_ID && archived {
            return Err("The default project cannot be archived".into());
        }
        self.call(move |connection| {
            let changed = connection
                .execute(
                    "UPDATE projects SET archived_at = CASE WHEN ?2 THEN unixepoch() ELSE NULL END
                     WHERE id = ?1",
                    params![project_id, archived],
                )
                .map_err(db_error)?;
            if changed != 1 {
                return Err("Project not found".into());
            }
            connection
                .query_row(
                    "SELECT id, name, path, (archived_at IS NOT NULL), created_at
                     FROM projects WHERE id = ?1",
                    [project_id],
                    project_row,
                )
                .map_err(db_error)
        })
        .await
    }

    pub async fn delete_session(&self, session_id: i64) -> AppResult<()> {
        self.call(move |connection| {
            let running: bool = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM runs WHERE status = 'running' AND session_id = ?1)",
                    [session_id],
                    |row| row.get(0),
                )
                .map_err(db_error)?;
            if running {
                return Err("Cannot delete a session with an active run".into());
            }
            let transaction = connection.transaction().map_err(db_error)?;
            transaction
                .execute(
                    "DELETE FROM audit WHERE run_id IN (SELECT id FROM runs WHERE session_id = ?1)",
                    [session_id],
                )
                .map_err(db_error)?;
            for statement in [
                "DELETE FROM run_metrics WHERE session_id = ?1",
                "DELETE FROM plan_cards WHERE session_id = ?1",
                "DELETE FROM runs WHERE session_id = ?1",
            ] {
                transaction.execute(statement, [session_id]).map_err(db_error)?;
            }
            let changed = transaction
                .execute("DELETE FROM sessions WHERE id = ?1", [session_id])
                .map_err(db_error)?;
            if changed != 1 {
                return Err("Session not found".into());
            }
            transaction.commit().map_err(db_error)
        })
        .await
    }

    pub async fn update_project_name(&self, project_id: i64, name: String) -> AppResult<()> {
        let name = name.trim().to_string();
        if name.is_empty() || name.len() > 128 {
            return Err("项目名称为空或过长".into());
        }
        self.call(move |connection| {
            let changed = connection
                .execute("UPDATE projects SET name = ?1 WHERE id = ?2", params![name, project_id])
                .map_err(db_error)?;
            if changed == 0 {
                return Err("项目不存在".into());
            }
            Ok(())
        })
        .await
    }

    pub async fn update_project_path(&self, project_id: i64, path: String) -> AppResult<()> {
        let path = path.trim().to_string();
        if path.is_empty() || path.len() > 512 {
            return Err("项目路径为空或过长".into());
        }
        self.call(move |connection| {
            let changed = connection
                .execute("UPDATE projects SET path = ?1 WHERE id = ?2", params![path, project_id])
                .map_err(db_error)?;
            if changed == 0 {
                return Err("项目不存在".into());
            }
            Ok(())
        })
        .await
    }

    pub async fn delete_project(&self, project_id: i64) -> AppResult<()> {
        if project_id == DEFAULT_PROJECT_ID {
            return Err("The default project cannot be deleted".into());
        }
        self.call(move |connection| {
            let running: bool = connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM runs WHERE status = 'running'
                     AND session_id IN (SELECT id FROM sessions WHERE project_id = ?1))",
                    [project_id],
                    |row| row.get(0),
                )
                .map_err(db_error)?;
            if running {
                return Err("Cannot delete a project with an active run".into());
            }
            let transaction = connection.transaction().map_err(db_error)?;
            transaction
                .execute(
                    "DELETE FROM audit WHERE run_id IN
                     (SELECT id FROM runs WHERE session_id IN
                     (SELECT id FROM sessions WHERE project_id = ?1))",
                    [project_id],
                )
                .map_err(db_error)?;
            for statement in [
                "DELETE FROM run_metrics WHERE session_id IN
                 (SELECT id FROM sessions WHERE project_id = ?1)",
                "DELETE FROM plan_cards WHERE session_id IN
                 (SELECT id FROM sessions WHERE project_id = ?1)",
                "DELETE FROM runs WHERE session_id IN
                 (SELECT id FROM sessions WHERE project_id = ?1)",
                "DELETE FROM sessions WHERE project_id = ?1",
            ] {
                transaction.execute(statement, [project_id]).map_err(db_error)?;
            }
            let changed = transaction
                .execute("DELETE FROM projects WHERE id = ?1", [project_id])
                .map_err(db_error)?;
            if changed != 1 {
                return Err("Project not found".into());
            }
            transaction.commit().map_err(db_error)
        })
        .await
    }

    pub async fn create_run_in_session(
        &self,
        session_id: i64,
        prompt: String,
        mode: Mode,
    ) -> AppResult<i64> {
        self.call(move |connection| {
            let transaction = connection.transaction().map_err(db_error)?;
            let changed = transaction.execute(
                "UPDATE sessions SET updated_at = unixepoch(), title = CASE
                 WHEN NOT EXISTS (SELECT 1 FROM runs WHERE session_id = ?1) THEN substr(?2, 1, 40)
                 ELSE title END WHERE id = ?1", params![session_id, prompt],
            ).map_err(db_error)?;
            if changed != 1 { return Err("Session not found".into()); }
            transaction
                .execute(
                    "INSERT INTO runs (prompt, mode, status, session_id) VALUES (?1, ?2, 'running', ?3)",
                    params![prompt, mode.as_str(), session_id],
                )
                .map_err(db_error)?;
            let id = transaction.last_insert_rowid();
            transaction
                .execute(
                    "INSERT INTO audit (run_id, action, detail) VALUES (?1, 'run_started', ?2)",
                    params![id, mode.as_str()],
                )
                .map_err(db_error)?;
            transaction.commit().map_err(db_error)?;
            Ok(id)
        })
        .await
    }

    pub async fn finish_run(&self, run: RunView) -> AppResult<()> {
        self.call(move |connection| {
            let transaction = connection.transaction().map_err(db_error)?;
            let changed = transaction
                .execute(
                    "UPDATE runs SET status = ?1, output = ?2, error = ?3,
                     finished_at = unixepoch() WHERE id = ?4 AND status = 'running'",
                    params![run.status.as_str(), run.output, run.error, run.id],
                )
                .map_err(db_error)?;
            if changed != 1 {
                return Err("Run is missing or already finished".into());
            }
            transaction.execute("UPDATE sessions SET updated_at = unixepoch() WHERE id = ?1", [run.session_id]).map_err(db_error)?;
            if matches!(run.status, RunStatus::Cancelled | RunStatus::Failed) {
                transaction.execute("UPDATE plan_cards SET status = 'invalidated' WHERE run_id = ?1 AND status = 'pending'", [run.id]).map_err(db_error)?;
            }
            transaction
                .execute(
                    "INSERT INTO audit (run_id, action, detail) VALUES (?1, 'run_finished', ?2)",
                    params![run.id, run.status.as_str()],
                )
                .map_err(db_error)?;
            transaction.commit().map_err(db_error)
        })
        .await
    }

    pub async fn record_mode(&self, mode: Mode) -> AppResult<()> {
        self.call(move |connection| {
            connection
                .execute(
                    "INSERT INTO audit (action, detail) VALUES ('mode_changed', ?1)",
                    [mode.as_str()],
                )
                .map_err(db_error)?;
            Ok(())
        })
        .await
    }

    pub async fn list_runs(&self) -> AppResult<Vec<RunView>> {
        self.call(|connection| read_runs(connection, None)).await
    }

    pub async fn list_sessions(&self) -> AppResult<Vec<SessionView>> {
        self.call(|connection| {
            let mut statement = connection
                .prepare(&format!(
                    "SELECT {SESSION_COLUMNS} FROM sessions s
                     ORDER BY s.updated_at DESC, s.id DESC"
                ))
                .map_err(db_error)?;
            let rows = statement.query_map([], session_row).map_err(db_error)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(db_error)
        }).await
    }

    pub async fn project_path_for_session(&self, session_id: i64) -> AppResult<Option<String>> {
        self.call(move |connection| {
            connection
                .query_row(
                    "SELECT p.path FROM sessions s JOIN projects p ON p.id = s.project_id WHERE s.id = ?1",
                    [session_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(db_error)
        })
        .await
    }

    pub async fn session_detail(&self, session_id: i64) -> AppResult<SessionDetail> {
        self.call(move |connection| {
            let session = read_session_view(connection, session_id)?;
            let runs = read_runs(connection, Some(session_id))?;
            let mut statement = connection.prepare(
                "SELECT id, session_id, run_id, status, draft FROM plan_cards WHERE session_id = ?1 ORDER BY id"
            ).map_err(db_error)?;
            let rows = statement.query_map([session_id], plan_row).map_err(db_error)?;
            let plans = rows.collect::<Result<Vec<_>, _>>().map_err(db_error)?;
            let metrics = read_metrics(connection, session_id)?;
            let mut tool_statement = connection.prepare(
                "SELECT run_id, action, detail FROM audit WHERE action IN ('tool_allowed','tool_blocked')
                 AND run_id IN (SELECT id FROM runs WHERE session_id = ?1) ORDER BY id"
            ).map_err(db_error)?;
            let tools = tool_statement
                .query_map([session_id], |row| {
                    let detail: String = row.get(2)?;
                    let (tool, reason) = detail.split_once(": ").unwrap_or((detail.as_str(), ""));
                    Ok(crate::domain::ToolCallView {
                        run_id: row.get(0)?,
                        tool: tool.to_string(),
                        detail: reason.to_string(),
                        allowed: row.get::<_, String>(1)? == "tool_allowed",
                    })
                })
                .map_err(db_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db_error)?;
            Ok(SessionDetail { session, runs, plans, metrics, tools })
        }).await
    }

    pub async fn create_plan(&self, run: RunView, draft: PlanDraft) -> AppResult<PlanCard> {
        draft.validate()?;
        let payload = serde_json::to_string(&draft).map_err(|error| error.to_string())?;
        self.call(move |connection| {
            let transaction = connection.transaction().map_err(db_error)?;
            let changed = transaction.execute(
                "INSERT INTO plan_cards (session_id, run_id, draft, status)
                 SELECT session_id, id, ?1, 'pending' FROM runs WHERE id = ?2 AND session_id = ?3 AND status = 'running'",
                params![payload, run.id, run.session_id],
            ).map_err(db_error)?;
            if changed != 1 { return Err("Plan requires a matching active run".into()); }
            let id = transaction.last_insert_rowid();
            transaction.execute("INSERT INTO audit (run_id, action, detail) VALUES (?1, 'plan_created', ?2)", params![run.id, id.to_string()]).map_err(db_error)?;
            transaction.commit().map_err(db_error)?;
            Ok(PlanCard { id, session_id: run.session_id, run_id: run.id, status: PlanStatus::Pending, draft })
        }).await
    }

    pub async fn decide_plan(
        &self,
        session_id: i64,
        plan_id: i64,
        decision: PlanDecision,
    ) -> AppResult<PlanCard> {
        self.call(move |connection| {
            let transaction = connection.transaction().map_err(db_error)?;
            let status = match decision { PlanDecision::Approve => PlanStatus::Approved, PlanDecision::Reject => PlanStatus::Rejected };
            let changed = transaction.execute(
                "UPDATE plan_cards SET status = ?1, decided_at = unixepoch()
                 WHERE id = ?2 AND session_id = ?3 AND status = 'pending'
                 AND EXISTS (SELECT 1 FROM runs WHERE id = plan_cards.run_id AND status = 'completed')",
                params![status.as_str(), plan_id, session_id],
            ).map_err(db_error)?;
            if changed != 1 { return Err("Plan is missing, already decided, or its run has not completed".into()); }
            let plan = transaction.query_row(
                "SELECT id, session_id, run_id, status, draft FROM plan_cards WHERE id = ?1", [plan_id], plan_row
            ).map_err(db_error)?;
            transaction.execute("INSERT INTO audit (run_id, action, detail) VALUES (?1, 'plan_decided', ?2)", params![plan.run_id, format!("{plan_id}:{}", status.as_str())]).map_err(db_error)?;
            transaction.commit().map_err(db_error)?;
            Ok(plan)
        }).await
    }

    pub async fn record_tool(
        &self,
        run_id: i64,
        tool: String,
        reason: String,
        allowed: bool,
    ) -> AppResult<()> {
        self.call(move |connection| {
            connection
                .execute(
                    "INSERT INTO audit (run_id, action, detail) VALUES (?1, ?2, ?3)",
                    params![
                        run_id,
                        if allowed {
                            "tool_allowed"
                        } else {
                            "tool_blocked"
                        },
                        format!("{tool}: {reason}")
                    ],
                )
                .map_err(db_error)?;
            Ok(())
        })
        .await
    }

    pub async fn save_metrics(&self, metrics: RunMetrics) -> AppResult<()> {
        self.call(move |connection| {
            let input = metrics.input_tokens.map(i64::try_from).transpose().map_err(|_| "Provider token usage exceeds database range")?;
            let output = metrics.output_tokens.map(i64::try_from).transpose().map_err(|_| "Provider token usage exceeds database range")?;
            let elapsed = i64::try_from(metrics.elapsed_ms).map_err(|_| "Invalid elapsed time")?;
            let first = metrics.first_token_ms.map(i64::try_from).transpose().map_err(|_| "Invalid first-token time")?;
            let cache_hit = metrics.cache_hit_tokens.map(i64::try_from).transpose().map_err(|_| "Provider cache usage exceeds database range")?;
            connection.execute("INSERT INTO run_metrics (run_id, session_id, provider, model, input_tokens, output_tokens, cache_hit_tokens, elapsed_ms, first_token_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(run_id) DO UPDATE SET input_tokens=excluded.input_tokens, output_tokens=excluded.output_tokens, cache_hit_tokens=excluded.cache_hit_tokens, elapsed_ms=excluded.elapsed_ms, first_token_ms=excluded.first_token_ms",
                params![metrics.run_id, metrics.session_id, metrics.provider, metrics.model, input, output, cache_hit, elapsed, first]).map_err(db_error)?;
            Ok(())
        }).await
    }

    pub async fn shutdown(&self) -> AppResult<()> {
        let worker = self
            .thread
            .lock()
            .map_err(|_| "Database thread lock poisoned")?
            .take();
        let Some(worker) = worker else { return Ok(()) };
        let (reply_tx, reply_rx) = oneshot::channel();
        let result = match self.sender.send(Request::Shutdown(reply_tx)).await {
            Ok(()) => reply_rx
                .await
                .unwrap_or_else(|_| Err("Database shutdown reply lost".into())),
            Err(_) => Err("Database worker already stopped".into()),
        };
        // Joining is not a SQLite operation; never block a Tokio worker on thread exit.
        tauri::async_runtime::spawn_blocking(move || worker.join())
            .await
            .map_err(|error| error.to_string())?
            .map_err(|_| "Database thread panicked".to_string())?;
        result
    }
}

fn db_error(error: rusqlite::Error) -> String {
    error.to_string()
}

fn read_metrics(connection: &Connection, session_id: i64) -> AppResult<Vec<RunMetrics>> {
    fn number(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<Option<u64>> {
        row.get::<_, Option<i64>>(index)?
            .map(|value| {
                u64::try_from(value)
                    .map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
            })
            .transpose()
    }
    let mut statement = connection.prepare("SELECT run_id, session_id, provider, model, input_tokens, output_tokens, cache_hit_tokens, elapsed_ms, first_token_ms FROM run_metrics WHERE session_id=?1 ORDER BY run_id").map_err(db_error)?;
    let rows = statement
        .query_map([session_id], |row| {
            Ok(RunMetrics {
                run_id: row.get(0)?,
                session_id: row.get(1)?,
                provider: row.get(2)?,
                model: row.get(3)?,
                input_tokens: number(row, 4)?,
                output_tokens: number(row, 5)?,
                cache_hit_tokens: number(row, 6)?,
                elapsed_ms: number(row, 7)?.unwrap_or(0),
                first_token_ms: number(row, 8)?,
            })
        })
        .map_err(db_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(db_error)
}

const SESSION_COLUMNS: &str = "s.id, s.title, s.updated_at,
     (SELECT COUNT(*) FROM runs WHERE session_id = s.id), s.project_id, (s.archived_at IS NOT NULL)";

fn session_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionView> {
    Ok(SessionView {
        id: row.get(0)?,
        title: row.get(1)?,
        updated_at: row.get(2)?,
        run_count: row.get(3)?,
        project_id: row.get(4)?,
        archived: row.get(5)?,
    })
}

fn project_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProjectView> {
    Ok(ProjectView {
        id: row.get(0)?,
        name: row.get(1)?,
        path: row.get(2)?,
        archived: row.get(3)?,
        created_at: row.get(4)?,
    })
}

fn read_session_view(connection: &Connection, id: i64) -> AppResult<SessionView> {
    connection
        .query_row(
            &format!("SELECT {SESSION_COLUMNS} FROM sessions s WHERE s.id = ?1"),
            [id],
            session_row,
        )
        .map_err(db_error)
}

fn read_runs(connection: &Connection, session_id: Option<i64>) -> AppResult<Vec<RunView>> {
    let query = if session_id.is_some() {
        "SELECT id, session_id, prompt, mode, status, output, error FROM runs WHERE session_id = ?1 ORDER BY id"
    } else {
        "SELECT id, session_id, prompt, mode, status, output, error FROM runs WHERE ?1 IS NULL ORDER BY id DESC LIMIT 50"
    };
    let mut statement = connection.prepare(query).map_err(db_error)?;
    let rows = statement
        .query_map([session_id], |row| {
            let mode: String = row.get(3)?;
            let status: String = row.get(4)?;
            Ok(RunView {
                id: row.get(0)?,
                session_id: row.get(1)?,
                prompt: row.get(2)?,
                mode: match mode.as_str() {
                    "plan" => Mode::Plan,
                    "code" => Mode::Code,
                    "confirm" => Mode::Confirm,
                    "auto" => Mode::Auto,
                    "execute" => Mode::Execute,
                    _ => {
                        return Err(rusqlite::Error::FromSqlConversionFailure(
                            3,
                            rusqlite::types::Type::Text,
                            Box::new(std::io::Error::other("Unknown stored mode")),
                        ))
                    }
                },
                status: RunStatus::from_db(&status),
                output: row.get(5)?,
                error: row.get(6)?,
            })
        })
        .map_err(db_error)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(db_error)
}

fn plan_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PlanCard> {
    let status: String = row.get(3)?;
    let payload: String = row.get(4)?;
    let draft = serde_json::from_str(&payload).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(PlanCard {
        id: row.get(0)?,
        session_id: row.get(1)?,
        run_id: row.get(2)?,
        status: match status.as_str() {
            "pending" => PlanStatus::Pending,
            "approved" => PlanStatus::Approved,
            "rejected" => PlanStatus::Rejected,
            _ => PlanStatus::Invalidated,
        },
        draft,
    })
}

// Called only inside maestro-sqlite, including directory creation and recovery.
fn initialize(path: PathBuf) -> AppResult<Connection> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let mut connection = Connection::open(path).map_err(db_error)?;
    connection
        .busy_timeout(Duration::from_secs(5))
        .map_err(db_error)?;
    let version: u32 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(db_error)?;
    if version > 5 {
        return Err(format!(
            "Database schema {version} is newer than supported version 5"
        ));
    }
    connection
        .execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             CREATE TABLE IF NOT EXISTS runs (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 prompt TEXT NOT NULL,
                 mode TEXT NOT NULL CHECK(mode IN ('plan', 'execute')),
                 status TEXT NOT NULL CHECK(status IN ('running', 'completed', 'cancelled', 'failed')),
                 output TEXT NOT NULL DEFAULT '',
                 error TEXT,
                 created_at INTEGER NOT NULL DEFAULT (unixepoch()),
                 finished_at INTEGER
             );
             CREATE TABLE IF NOT EXISTS audit (
                 id INTEGER PRIMARY KEY AUTOINCREMENT,
                 run_id INTEGER REFERENCES runs(id),
                 action TEXT NOT NULL,
                 detail TEXT NOT NULL,
                 created_at INTEGER NOT NULL DEFAULT (unixepoch())
             );
             ",
        )
        .map_err(db_error)?;
    // ALTER TABLE only allows a REFERENCES column with a non-NULL default while
    // foreign keys are disabled, so the v4 session.project_id migration needs this off.
    if version < 4 {
        connection
            .pragma_update(None, "foreign_keys", false)
            .map_err(db_error)?;
    }
    let transaction = connection.transaction().map_err(db_error)?;
    if version < 2 {
        transaction.execute_batch(
            "CREATE TABLE sessions (
                id INTEGER PRIMARY KEY AUTOINCREMENT, title TEXT NOT NULL,
                created_at INTEGER NOT NULL DEFAULT (unixepoch()), updated_at INTEGER NOT NULL DEFAULT (unixepoch())
             );
             ALTER TABLE runs ADD COLUMN session_id INTEGER REFERENCES sessions(id);
             INSERT INTO sessions (id, title, created_at, updated_at)
             SELECT id, substr(prompt, 1, 40), created_at, COALESCE(finished_at, created_at) FROM runs;
             UPDATE runs SET session_id = id;
             CREATE INDEX runs_session_id ON runs(session_id);
             CREATE TABLE plan_cards (
                id INTEGER PRIMARY KEY AUTOINCREMENT, session_id INTEGER NOT NULL REFERENCES sessions(id),
                run_id INTEGER NOT NULL UNIQUE REFERENCES runs(id), draft TEXT NOT NULL,
                status TEXT NOT NULL CHECK(status IN ('pending','approved','rejected','invalidated')),
                created_at INTEGER NOT NULL DEFAULT (unixepoch()), decided_at INTEGER
             );
             PRAGMA user_version = 2;"
        ).map_err(db_error)?;
    }
    if version < 3 {
        let sequence: i64 = transaction
            .query_row(
                "SELECT COALESCE((SELECT seq FROM sqlite_sequence WHERE name='runs'),0)",
                [],
                |row| row.get(0),
            )
            .map_err(db_error)?;
        transaction.execute_batch("CREATE TABLE runs_v3 (id INTEGER PRIMARY KEY AUTOINCREMENT, prompt TEXT NOT NULL, mode TEXT NOT NULL CHECK(mode IN ('plan','code','confirm','auto','execute')), status TEXT NOT NULL CHECK(status IN ('running','completed','cancelled','failed')), output TEXT NOT NULL DEFAULT '', error TEXT, created_at INTEGER NOT NULL DEFAULT (unixepoch()), finished_at INTEGER, session_id INTEGER REFERENCES sessions(id));
            INSERT INTO runs_v3 SELECT id,prompt,mode,status,output,error,created_at,finished_at,session_id FROM runs;
            DROP TABLE runs; ALTER TABLE runs_v3 RENAME TO runs; CREATE INDEX runs_session_id ON runs(session_id);
            CREATE TABLE run_metrics (run_id INTEGER PRIMARY KEY REFERENCES runs(id), session_id INTEGER NOT NULL REFERENCES sessions(id), provider TEXT NOT NULL, model TEXT NOT NULL, input_tokens INTEGER, output_tokens INTEGER, elapsed_ms INTEGER NOT NULL, first_token_ms INTEGER);
            PRAGMA user_version=3;").map_err(db_error)?;
        transaction.execute("INSERT INTO sqlite_sequence(name,seq) SELECT 'runs',?1 WHERE NOT EXISTS(SELECT 1 FROM sqlite_sequence WHERE name='runs')", [sequence]).map_err(db_error)?;
        transaction
            .execute(
                "UPDATE sqlite_sequence SET seq=MAX(seq,?1) WHERE name='runs'",
                [sequence],
            )
            .map_err(db_error)?;
        let mut check = transaction
            .prepare("PRAGMA foreign_key_check")
            .map_err(db_error)?;
        if check
            .query([])
            .map_err(db_error)?
            .next()
            .map_err(db_error)?
            .is_some()
        {
            return Err("Migration foreign-key integrity check failed".into());
        }
    }
    if version < 4 {
        transaction
            .execute_batch(
                "CREATE TABLE projects (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    name TEXT NOT NULL,
                    path TEXT,
                    archived_at INTEGER,
                    created_at INTEGER NOT NULL DEFAULT (unixepoch())
                 );
                 INSERT INTO projects (id, name) VALUES (1, 'Maestro-Agent');
                 ALTER TABLE sessions ADD COLUMN project_id INTEGER NOT NULL DEFAULT 1 REFERENCES projects(id);
                 ALTER TABLE sessions ADD COLUMN archived_at INTEGER;
                 CREATE INDEX sessions_project_id ON sessions(project_id);
                 PRAGMA user_version = 4;",
            )
            .map_err(db_error)?;
    }
    if version < 5 {
        transaction
            .execute_batch(
                "ALTER TABLE run_metrics ADD COLUMN cache_hit_tokens INTEGER;
                 PRAGMA user_version = 5;",
            )
            .map_err(db_error)?;
    }
    transaction.execute("UPDATE plan_cards SET status = 'invalidated' WHERE status = 'pending' AND run_id IN (SELECT id FROM runs WHERE status = 'running')", []).map_err(db_error)?;
    transaction
        .execute(
            "INSERT INTO audit (run_id, action, detail)
             SELECT id, 'run_interrupted', 'Application exited before completion'
             FROM runs WHERE status = 'running'",
            [],
        )
        .map_err(db_error)?;
    transaction
        .execute(
            "UPDATE runs SET status = 'failed', error = 'Application exited before completion',
             finished_at = unixepoch() WHERE status = 'running'",
            [],
        )
        .map_err(db_error)?;
    transaction.commit().map_err(db_error)?;
    connection
        .pragma_update(None, "foreign_keys", true)
        .map_err(db_error)?;
    Ok(connection)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn concurrent_requests_are_serialized_and_committed() {
        let database = Arc::new(Database::open(":memory:".into()).await.unwrap());
        let mut tasks = Vec::new();
        for index in 0..16 {
            let database = database.clone();
            tasks.push(tokio::spawn(async move {
                let prompt = format!("request {index}");
                let id = database
                    .create_run(prompt.clone(), Mode::Plan)
                    .await
                    .unwrap();
                database
                    .finish_run(RunView {
                        id,
                        session_id: database
                            .list_runs()
                            .await
                            .unwrap()
                            .into_iter()
                            .find(|run| run.id == id)
                            .unwrap()
                            .session_id,
                        prompt,
                        mode: Mode::Plan,
                        status: RunStatus::Completed,
                        output: "done".into(),
                        error: None,
                    })
                    .await
                    .unwrap();
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        let runs = database.list_runs().await.unwrap();
        assert_eq!(runs.len(), 16);
        assert!(runs
            .iter()
            .all(|run| run.status == RunStatus::Completed && run.output == "done"));
        database.shutdown().await.unwrap();
        assert!(database.list_runs().await.is_err());
    }

    #[tokio::test]
    async fn all_sql_executes_on_the_named_os_thread() {
        let database = Database::open(":memory:".into()).await.unwrap();
        let name = database
            .call(|_| Ok(thread::current().name().unwrap().to_string()))
            .await
            .unwrap();
        assert_eq!(name, "maestro-sqlite");
        database.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn restart_preserves_completed_runs_and_recovers_interrupted_runs_once() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested/maestro.sqlite3");
        let database = Database::open(path.clone()).await.unwrap();
        let completed_id = database
            .create_run("complete".into(), Mode::Plan)
            .await
            .unwrap();
        database
            .finish_run(RunView {
                id: completed_id,
                session_id: database.list_runs().await.unwrap()[0].session_id,
                prompt: "complete".into(),
                mode: Mode::Plan,
                status: RunStatus::Completed,
                output: "persisted".into(),
                error: None,
            })
            .await
            .unwrap();
        let interrupted_id = database
            .create_run("interrupted".into(), Mode::Execute)
            .await
            .unwrap();
        database.shutdown().await.unwrap();

        for _ in 0..2 {
            let reopened = Database::open(path.clone()).await.unwrap();
            let runs = reopened.list_runs().await.unwrap();
            assert_eq!(runs.len(), 2);
            assert_eq!(runs[0].id, interrupted_id);
            assert_eq!(runs[0].status, RunStatus::Failed);
            assert_eq!(runs[0].mode, Mode::Execute);
            assert!(runs[0]
                .error
                .as_ref()
                .unwrap()
                .contains("before completion"));
            assert_eq!(runs[1].id, completed_id);
            assert_eq!(runs[1].status, RunStatus::Completed);
            assert_eq!(runs[1].output, "persisted");
            let audit_count: i64 = reopened
                .call(|connection| {
                    connection
                        .query_row(
                            "SELECT COUNT(*) FROM audit WHERE action = 'run_interrupted'",
                            [],
                            |row| row.get(0),
                        )
                        .map_err(db_error)
                })
                .await
                .unwrap();
            assert_eq!(audit_count, 1);
            reopened.shutdown().await.unwrap();
        }
    }

    #[tokio::test]
    async fn repeated_completion_does_not_duplicate_audit_or_change_the_result() {
        let database = Database::open(":memory:".into()).await.unwrap();
        let id = database
            .create_run("complete".into(), Mode::Plan)
            .await
            .unwrap();
        let run = RunView {
            id,
            session_id: database.list_runs().await.unwrap()[0].session_id,
            prompt: "complete".into(),
            mode: Mode::Plan,
            status: RunStatus::Completed,
            output: "first result".into(),
            error: None,
        };
        database.finish_run(run.clone()).await.unwrap();
        assert!(database
            .finish_run(RunView {
                output: "incorrect overwrite".into(),
                ..run
            })
            .await
            .is_err());
        assert_eq!(
            database.list_runs().await.unwrap()[0].output,
            "first result"
        );
        let audit_count: i64 = database
            .call(|connection| {
                connection
                    .query_row(
                        "SELECT COUNT(*) FROM audit WHERE action = 'run_finished'",
                        [],
                        |row| row.get(0),
                    )
                    .map_err(db_error)
            })
            .await
            .unwrap();
        assert_eq!(audit_count, 1);
        database.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn newer_database_schema_is_rejected_without_downgrading_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("future.sqlite3");
        let database = Database::open(path.clone()).await.unwrap();
        database
            .call(|connection| {
                connection
                    .pragma_update(None, "user_version", 6)
                    .map_err(db_error)
            })
            .await
            .unwrap();
        database.shutdown().await.unwrap();
        let error = match Database::open(path.clone()).await {
            Ok(database) => {
                database.shutdown().await.unwrap();
                panic!("Newer schema accepted");
            }
            Err(error) => error,
        };
        assert!(error.contains("newer than supported"));
    }

    #[tokio::test]
    async fn plans_and_decisions_survive_restart_and_are_single_use() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("plans.sqlite3");
        let database = Database::open(path.clone()).await.unwrap();
        let id = database
            .create_run("plan".into(), Mode::Plan)
            .await
            .unwrap();
        let run = database.list_runs().await.unwrap().remove(0);
        let draft = PlanDraft {
            title: "Review".into(),
            summary: "Proposed only".into(),
            steps: vec!["review".into()],
            diffs: Vec::new(),
        };
        let card = database.create_plan(run.clone(), draft).await.unwrap();
        assert!(database
            .decide_plan(run.session_id, card.id, PlanDecision::Approve)
            .await
            .is_err());
        database
            .finish_run(RunView {
                status: RunStatus::Completed,
                ..run.clone()
            })
            .await
            .unwrap();
        database.shutdown().await.unwrap();
        let reopened = Database::open(path.clone()).await.unwrap();
        assert_eq!(
            reopened.session_detail(run.session_id).await.unwrap().plans[0].status,
            PlanStatus::Pending
        );
        assert!(reopened
            .decide_plan(run.session_id + 1, card.id, PlanDecision::Approve)
            .await
            .is_err());
        assert_eq!(
            reopened
                .decide_plan(run.session_id, card.id, PlanDecision::Approve)
                .await
                .unwrap()
                .status,
            PlanStatus::Approved
        );
        assert!(reopened
            .decide_plan(run.session_id, card.id, PlanDecision::Reject)
            .await
            .is_err());
        assert_eq!(
            reopened.session_detail(run.session_id).await.unwrap().runs[0].id,
            id
        );
        let count: i64 = reopened
            .call(|connection| {
                connection
                    .query_row(
                        "SELECT COUNT(*) FROM audit WHERE action = 'plan_decided'",
                        [],
                        |row| row.get(0),
                    )
                    .map_err(db_error)
            })
            .await
            .unwrap();
        assert_eq!(count, 1);
        reopened.shutdown().await.unwrap();
        let reopened = Database::open(path).await.unwrap();
        assert_eq!(
            reopened.session_detail(run.session_id).await.unwrap().plans[0].status,
            PlanStatus::Approved
        );
        reopened.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn migration_preserves_v1_runs_and_creates_sessions() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("legacy.sqlite3");
        let thread_path = path.clone();
        // Even test fixtures create/read SQLite only on an OS thread.
        let worker = thread::spawn(move || {
            let connection = Connection::open(thread_path).unwrap();
            connection.execute_batch("CREATE TABLE runs (id INTEGER PRIMARY KEY AUTOINCREMENT, prompt TEXT NOT NULL, mode TEXT NOT NULL, status TEXT NOT NULL, output TEXT NOT NULL DEFAULT '', error TEXT, created_at INTEGER NOT NULL DEFAULT (unixepoch()), finished_at INTEGER);
                CREATE TABLE audit (id INTEGER PRIMARY KEY, run_id INTEGER REFERENCES runs(id), action TEXT NOT NULL, detail TEXT NOT NULL, created_at INTEGER DEFAULT (unixepoch()));
                INSERT INTO runs (prompt, mode, status, output) VALUES ('legacy', 'plan', 'completed', 'old output'); PRAGMA user_version = 1;").unwrap();
        });
        tauri::async_runtime::spawn_blocking(move || worker.join())
            .await
            .unwrap()
            .unwrap();
        let database = Database::open(path).await.unwrap();
        let sessions = database.list_sessions().await.unwrap();
        assert_eq!(sessions.len(), 1);
        let detail = database.session_detail(sessions[0].id).await.unwrap();
        assert_eq!(detail.runs[0].output, "old output");
        assert_eq!(detail.session.title, "legacy");
        database.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn v3_preserves_v2_modes_foreign_keys_and_autoincrement_high_water() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("v2.sqlite3");
        let copy = path.clone();
        let worker = thread::spawn(move || {
            let connection = Connection::open(copy).unwrap();
            connection.execute_batch("CREATE TABLE sessions (id INTEGER PRIMARY KEY AUTOINCREMENT,title TEXT NOT NULL,created_at INTEGER DEFAULT (unixepoch()),updated_at INTEGER DEFAULT (unixepoch()));
                CREATE TABLE runs (id INTEGER PRIMARY KEY AUTOINCREMENT,prompt TEXT NOT NULL,mode TEXT CHECK(mode IN ('plan','execute')),status TEXT,output TEXT,error TEXT,created_at INTEGER,finished_at INTEGER,session_id INTEGER REFERENCES sessions(id));
                CREATE TABLE audit (id INTEGER PRIMARY KEY AUTOINCREMENT,run_id INTEGER REFERENCES runs(id),action TEXT,detail TEXT,created_at INTEGER DEFAULT (unixepoch()));
                CREATE TABLE plan_cards (id INTEGER PRIMARY KEY,session_id INTEGER REFERENCES sessions(id),run_id INTEGER REFERENCES runs(id),draft TEXT,status TEXT,created_at INTEGER,decided_at INTEGER);
                INSERT INTO sessions(id,title) VALUES(1,'legacy');
                INSERT INTO runs VALUES(1,'old','execute','completed','answer',NULL,1,2,1);
                INSERT INTO runs VALUES(2,'plan','plan','completed','plan answer',NULL,1,2,1);
                INSERT INTO runs VALUES(99,'deleted','plan','completed','',NULL,1,2,1); DELETE FROM runs WHERE id=99;
                INSERT INTO audit(run_id,action,detail) VALUES(1,'existing','preserve');
                INSERT INTO plan_cards VALUES(3,1,1,'{\"title\":\"kept\",\"summary\":\"legacy\",\"steps\":[\"review\"],\"diffs\":[]}','approved',1,2);
                PRAGMA user_version=2;").unwrap();
        });
        tauri::async_runtime::spawn_blocking(move || worker.join())
            .await
            .unwrap()
            .unwrap();
        let database = Database::open(path.clone()).await.unwrap();
        let detail = database.session_detail(1).await.unwrap();
        assert_eq!(detail.runs[0].mode, Mode::Execute);
        assert_eq!(detail.runs[1].mode, Mode::Plan);
        assert_eq!(detail.plans[0].status, PlanStatus::Approved);
        let id = database
            .create_run_in_session(1, "new code".into(), Mode::Code)
            .await
            .unwrap();
        assert!(id > 99);
        database
            .save_metrics(RunMetrics {
                run_id: id,
                session_id: 1,
                provider: "openai".into(),
                model: "o3-mini".into(),
                input_tokens: Some(24),
                cache_hit_tokens: None,
                output_tokens: Some(10),
                elapsed_ms: 250,
                first_token_ms: Some(20),
            })
            .await
            .unwrap();
        assert_eq!(
            database.session_detail(1).await.unwrap().metrics[0].output_tokens,
            Some(10)
        );
        database.shutdown().await.unwrap();
        let reopened = Database::open(path).await.unwrap();
        assert_eq!(
            reopened.session_detail(1).await.unwrap().plans[0].status,
            PlanStatus::Approved
        );
        reopened.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn v4_projects_archive_sessions_and_cascade_delete() {
        let directory = tempfile::tempdir().unwrap();
        let database = Database::open(":memory:".into()).await.unwrap();

        // The default project is seeded and owns new sessions.
        let projects = database.list_projects().await.unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].id, DEFAULT_PROJECT_ID);
        let session = database.create_session().await.unwrap();
        assert_eq!(session.project_id, DEFAULT_PROJECT_ID);
        assert!(!session.archived);

        // Sessions require an existing, unarchived project.
        assert!(database.create_session_in_project(999).await.is_err());
        let project = database
            .create_project(
                "  Side  ".into(),
                Some(directory.path().to_string_lossy().into_owned()),
            )
            .await
            .unwrap();
        assert_eq!(project.name, "Side");
        let moved = database
            .create_session_in_project(project.id)
            .await
            .unwrap();
        assert_eq!(moved.project_id, project.id);
        assert!(database.create_project("  ".into(), None).await.is_err());
        assert!(database
            .create_project(
                "bad".into(),
                Some("C:/definitely/missing/path".into())
            )
            .await
            .is_err());

        // Sessions rename and archive.
        let renamed = database
            .rename_session(session.id, "  重命名  ".into())
            .await
            .unwrap();
        assert_eq!(renamed.title, "重命名");
        assert!(database
            .rename_session(session.id, "   ".into())
            .await
            .is_err());
        assert!(database
            .set_session_archived(moved.id, true)
            .await
            .unwrap()
            .archived);
        assert!(!database
            .set_session_archived(moved.id, false)
            .await
            .unwrap()
            .archived);

        // Archived projects reject new sessions; the default project is pinned.
        database
            .set_project_archived(project.id, true)
            .await
            .unwrap();
        assert!(database.list_projects().await.unwrap()[1].archived);
        assert!(database
            .create_session_in_project(project.id)
            .await
            .is_err());
        database
            .set_project_archived(project.id, false)
            .await
            .unwrap();
        assert!(database
            .set_project_archived(DEFAULT_PROJECT_ID, true)
            .await
            .is_err());
        assert!(database.delete_project(DEFAULT_PROJECT_ID).await.is_err());
        assert!(database.delete_project(999).await.is_err());

        // A running run blocks archiving and project deletion.
        let run_id = database
            .create_run_in_session(moved.id, "work".into(), Mode::Plan)
            .await
            .unwrap();
        assert!(database.delete_project(project.id).await.is_err());
        assert!(database
            .set_session_archived(moved.id, true)
            .await
            .is_err());
        database
            .finish_run(RunView {
                id: run_id,
                session_id: moved.id,
                prompt: "work".into(),
                mode: Mode::Plan,
                status: RunStatus::Completed,
                output: "done".into(),
                error: None,
            })
            .await
            .unwrap();
        database
            .save_metrics(RunMetrics {
                run_id,
                session_id: moved.id,
                provider: "openai".into(),
                model: "o3-mini".into(),
                input_tokens: Some(3),
                cache_hit_tokens: None,
                output_tokens: Some(4),
                elapsed_ms: 9,
                first_token_ms: None,
            })
            .await
            .unwrap();

        // Deletion cascades sessions, runs, plans, metrics and audit rows.
        database.delete_project(project.id).await.unwrap();
        assert!(database.session_detail(moved.id).await.is_err());
        let orphans: i64 = database
            .call(move |connection| {
                connection
                    .query_row(
                        "SELECT COUNT(*) FROM run_metrics WHERE session_id = ?1",
                        [moved.id],
                        |row| row.get(0),
                    )
                    .map_err(db_error)
            })
            .await
            .unwrap();
        assert_eq!(orphans, 0);
        let audit_orphans: i64 = database
            .call(move |connection| {
                connection
                    .query_row(
                        "SELECT COUNT(*) FROM audit WHERE run_id = ?1",
                        [run_id],
                        |row| row.get(0),
                    )
                    .map_err(db_error)
            })
            .await
            .unwrap();
        assert_eq!(audit_orphans, 0);
        assert_eq!(
            database
                .session_detail(session.id)
                .await
                .unwrap()
                .session
                .title,
            "重命名"
        );
        database.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn v4_migration_assigns_existing_sessions_to_the_default_project() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("v3.sqlite3");
        let copy = path.clone();
        let worker = thread::spawn(move || {
            let connection = Connection::open(copy).unwrap();
            connection.execute_batch("CREATE TABLE sessions (id INTEGER PRIMARY KEY AUTOINCREMENT,title TEXT NOT NULL,created_at INTEGER DEFAULT (unixepoch()),updated_at INTEGER DEFAULT (unixepoch()));
                CREATE TABLE runs (id INTEGER PRIMARY KEY AUTOINCREMENT,prompt TEXT NOT NULL,mode TEXT CHECK(mode IN ('plan','code','confirm','auto','execute')),status TEXT,output TEXT,error TEXT,created_at INTEGER,finished_at INTEGER,session_id INTEGER REFERENCES sessions(id));
                CREATE INDEX runs_session_id ON runs(session_id);
                CREATE TABLE audit (id INTEGER PRIMARY KEY AUTOINCREMENT,run_id INTEGER REFERENCES runs(id),action TEXT,detail TEXT,created_at INTEGER DEFAULT (unixepoch()));
                CREATE TABLE plan_cards (id INTEGER PRIMARY KEY,session_id INTEGER REFERENCES sessions(id),run_id INTEGER REFERENCES runs(id),draft TEXT,status TEXT,created_at INTEGER,decided_at INTEGER);
                CREATE TABLE run_metrics (run_id INTEGER PRIMARY KEY REFERENCES runs(id), session_id INTEGER NOT NULL REFERENCES sessions(id), provider TEXT NOT NULL, model TEXT NOT NULL, input_tokens INTEGER, output_tokens INTEGER, elapsed_ms INTEGER NOT NULL, first_token_ms INTEGER);
                INSERT INTO sessions(id,title) VALUES(1,'legacy'),(2,'second');
                INSERT INTO runs VALUES(1,'old','code','completed','answer',NULL,1,2,1);
                PRAGMA user_version=3;").unwrap();
        });
        tauri::async_runtime::spawn_blocking(move || worker.join())
            .await
            .unwrap()
            .unwrap();
        let database = Database::open(path).await.unwrap();
        let projects = database.list_projects().await.unwrap();
        assert_eq!(projects.len(), 1);
        assert_eq!(projects[0].name, "Maestro-Agent");
        let sessions = database.list_sessions().await.unwrap();
        assert_eq!(sessions.len(), 2);
        assert!(sessions
            .iter()
            .all(|session| session.project_id == DEFAULT_PROJECT_ID && !session.archived));
        assert_eq!(database.session_detail(1).await.unwrap().runs[0].output, "answer");
        database.shutdown().await.unwrap();
    }
}
