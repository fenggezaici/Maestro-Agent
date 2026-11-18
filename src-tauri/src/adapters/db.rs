//! SQLite is confined to one OS thread. Async callers only exchange messages.
use std::{path::PathBuf, sync::Mutex, thread, time::Duration};

use rusqlite::OptionalExtension;
use rusqlite::{params, Connection};
use tokio::sync::{mpsc, oneshot};

use crate::domain::{
    ActivityDay, ActivitySnapshot, AppResult, ContextCompaction, RunChangeSummary, RunFileChange, Mode, PlanCard,
    RunMessage,
    PlanDecision, PlanDraft, PlanStatus, ProjectView, RunMetrics, RunStatus, RunView, SessionDetail,
    SessionView, SubagentStatus, SubagentView, ToolCallView,
};

/// Seed id for the project that owns sessions migrated from pre-v4 databases.
/// The row is a normal project: it can be archived or deleted like any other.
pub const DEFAULT_PROJECT_ID: i64 = 1;

type Operation = Box<dyn FnOnce(&mut Connection) + Send>;

/// (seq, kind, text, tool_index, started_at, ended_at) for a run_parts insert.
type RunPartRow = (i64, String, String, Option<i64>, i64, Option<i64>);

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

    /// First unarchived project in creation order; the fallback home for a new
    /// session when the caller did not pin a project.
    pub async fn first_active_project_id(&self) -> AppResult<Option<i64>> {
        self.call(|connection| {
            connection
                .query_row(
                    "SELECT id FROM projects WHERE archived_at IS NULL ORDER BY created_at, id LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .optional()
                .map_err(db_error)
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
                "DELETE FROM run_file_changes WHERE session_id = ?1",
                "DELETE FROM subagent_traces WHERE session_id = ?1",
                "DELETE FROM session_compactions WHERE session_id = ?1",
                "DELETE FROM run_messages WHERE session_id = ?1",
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
                "DELETE FROM run_file_changes WHERE project_id = ?1 OR session_id IN
                 (SELECT id FROM sessions WHERE project_id = ?1)",
                "DELETE FROM subagent_traces WHERE session_id IN
                 (SELECT id FROM sessions WHERE project_id = ?1)",
                "DELETE FROM session_compactions WHERE session_id IN
                 (SELECT id FROM sessions WHERE project_id = ?1)",
                "DELETE FROM run_messages WHERE session_id IN
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
            let mut parts: std::collections::HashMap<i64, Vec<crate::domain::RunPart>> = std::collections::HashMap::new();
            let mut parts_statement = connection.prepare(
                "SELECT run_id, kind, text, tool_index, started_at, ended_at FROM run_parts
                 WHERE run_id IN (SELECT id FROM runs WHERE session_id = ?1) ORDER BY run_id, seq"
            ).map_err(db_error)?;
            let parts_rows = parts_statement
                .query_map([session_id], |row| {
                    let run_id: i64 = row.get(0)?;
                    Ok((run_id, crate::domain::RunPart {
                        kind: row.get(1)?,
                        text: row.get(2)?,
                        index: row.get::<_, Option<i64>>(3)?.map(|value| value as usize),
                        started_at: row.get(4)?,
                        ended_at: row.get(5)?,
                    }))
                })
                .map_err(db_error)?;
            for row in parts_rows {
                let (run_id, part) = row.map_err(db_error)?;
                parts.entry(run_id).or_default().push(part);
            }
            let mut reasoning: std::collections::HashMap<i64, Vec<crate::domain::ReasoningSegment>> = std::collections::HashMap::new();
            let mut reasoning_statement = connection.prepare(
                "SELECT id, reasoning_json FROM runs WHERE session_id = ?1 AND reasoning_json != '[]'"
            ).map_err(db_error)?;
            let reasoning_rows = reasoning_statement
                .query_map([session_id], |row| {
                    let run_id: i64 = row.get(0)?;
                    let json: String = row.get(1)?;
                    Ok((run_id, serde_json::from_str::<Vec<crate::domain::ReasoningSegment>>(&json).unwrap_or_default()))
                })
                .map_err(db_error)?;
            for row in reasoning_rows {
                let (run_id, segments) = row.map_err(db_error)?;
                if !segments.is_empty() { reasoning.insert(run_id, segments); }
            }
            let mut tool_statement = connection.prepare(
                "SELECT run_id, action, detail FROM audit WHERE action IN ('tool_allowed','tool_blocked')
                 AND run_id IN (SELECT id FROM runs WHERE session_id = ?1) ORDER BY id"
            ).map_err(db_error)?;
            let tools = tool_statement
                .query_map([session_id], |row| {
                    let detail: String = row.get(2)?;
                    let (tool, reason) = detail.split_once(": ").unwrap_or((detail.as_str(), ""));
                    let (subagent_id, reason) = if tool == "spawn_subagent" {
                        match reason.split_once(" · ") {
                            Some((id, rest)) if id.starts_with("sa-") => {
                                (Some(id.to_string()), rest.to_string())
                            }
                            _ => (None, reason.to_string()),
                        }
                    } else {
                        (None, reason.to_string())
                    };
                    Ok(crate::domain::ToolCallView {
                        run_id: row.get(0)?,
                        tool: tool.to_string(),
                        detail: reason,
                        allowed: row.get::<_, String>(1)? == "tool_allowed",
                        subagent_id,
                        pending: None,
                        target: None,
                    })
                })
                .map_err(db_error)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(db_error)?;
            let compactions = {
                let mut statement = connection
                    .prepare("SELECT operation_id, session_id, through_run_id, summary, trigger, status, pre_tokens, post_tokens, created_at FROM session_compactions WHERE session_id=?1 ORDER BY created_at")
                    .map_err(db_error)?;
                let rows = statement.query_map([session_id], compaction_row).map_err(db_error)?;
                rows.collect::<Result<Vec<_>, _>>().map_err(db_error)?
            };
            let run_messages = {
                let mut statement = connection
                    .prepare("SELECT run_id, session_id, seq, role, content, tool_calls, tool_call_id, name FROM run_messages WHERE session_id=?1 ORDER BY run_id, seq")
                    .map_err(db_error)?;
                let rows = statement.query_map([session_id], run_message_row).map_err(db_error)?;
                rows.collect::<Result<Vec<_>, _>>().map_err(db_error)?
            };
            Ok(SessionDetail { session, runs, plans, metrics, tools, compactions, run_messages, reasoning, parts })
        }).await
    }

    pub async fn insert_subagent_trace(&self, trace: SubagentView) -> AppResult<()> {
        self.call(move |connection| {
            let tools = serde_json::to_string(&trace.tools).map_err(|error| error.to_string())?;
            connection
                .execute(
                    "INSERT INTO subagent_traces
                     (id, run_id, session_id, agent, description, prompt, model, status, summary,
                      error, content, tool_count, input_tokens, output_tokens,
                      reasoning_tokens, first_token_ms, elapsed_ms, decode_ms, decode_tokens,
                      last_input_tokens, started_at, finished_at, tools)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                             ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)",
                    params![
                        trace.id,
                        trace.run_id,
                        trace.session_id,
                        trace.agent,
                        trace.description,
                        trace.prompt,
                        trace.model,
                        trace.status.as_str(),
                        trace.summary,
                        trace.error,
                        trace.content,
                        trace.tool_count as i64,
                        trace.input_tokens.map(|value| value as i64),
                        trace.output_tokens.map(|value| value as i64),
                        trace.reasoning_tokens.map(|value| value as i64),
                        trace.first_token_ms.map(|value| value as i64),
                        trace.elapsed_ms as i64,
                        trace.decode_ms as i64,
                        trace.decode_tokens as i64,
                        trace.last_input_tokens.map(|value| value as i64),
                        trace.started_at,
                        trace.finished_at,
                        tools,
                    ],
                )
                .map_err(db_error)?;
            Ok(())
        })
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn finish_subagent_trace(
        &self,
        id: String,
        status: SubagentStatus,
        summary: String,
        error: Option<String>,
        content: String,
        tools: Vec<ToolCallView>,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
        reasoning_tokens: Option<u64>,
        first_token_ms: Option<u64>,
        elapsed_ms: u64,
        decode_ms: u64,
        decode_tokens: u64,
        last_input_tokens: Option<u64>,
        finished_at: i64,
    ) -> AppResult<()> {
        self.call(move |connection| {
            let tool_count = tools.len() as i64;
            let tools = serde_json::to_string(&tools).map_err(|error| error.to_string())?;
            let changed = connection
                .execute(
                    "UPDATE subagent_traces SET status = ?2, summary = ?3, error = ?4, content = ?5,
                     tool_count = ?6, input_tokens = ?7, output_tokens = ?8,
                     reasoning_tokens = ?11, first_token_ms = ?12, elapsed_ms = ?13, decode_ms = ?15,
                     decode_tokens = ?16, last_input_tokens = ?14,
                     finished_at = ?9, tools = ?10
                     WHERE id = ?1",
                    params![
                        id,
                        status.as_str(),
                        summary,
                        error,
                        content,
                        tool_count,
                        input_tokens.map(|value| value as i64),
                        output_tokens.map(|value| value as i64),
                        finished_at,
                        tools,
                        reasoning_tokens.map(|value| value as i64),
                        first_token_ms.map(|value| value as i64),
                        elapsed_ms as i64,
                        last_input_tokens.map(|value| value as i64),
                        decode_ms as i64,
                        decode_tokens as i64,
                    ],
                )
                .map_err(db_error)?;
            if changed != 1 {
                return Err("Subagent trace not found".into());
            }
            Ok(())
        })
        .await
    }

    pub async fn list_subagent_traces(&self, session_id: i64) -> AppResult<Vec<SubagentView>> {
        self.call(move |connection| {
            let mut statement = connection
                .prepare(
                    "SELECT id, run_id, session_id, agent, description, prompt, model, status, summary,
                            error, content, tool_count, input_tokens, output_tokens,
                            reasoning_tokens, first_token_ms, elapsed_ms, decode_ms, decode_tokens,
                            last_input_tokens,
                            started_at, finished_at, tools
                     FROM subagent_traces WHERE session_id = ?1 ORDER BY started_at, id",
                )
                .map_err(db_error)?;
            let rows = statement.query_map([session_id], subagent_row).map_err(db_error)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(db_error)
        })
        .await
    }

    /// Newest reports first, capped for conversation-context injection.
    pub async fn save_run_messages(&self, messages: Vec<RunMessage>) -> AppResult<()> {
        self.call(move |connection| {
            let transaction = connection.transaction().map_err(db_error)?;
            for message in messages {
                let tool_calls = message
                    .tool_calls
                    .as_ref()
                    .map(serde_json::to_string)
                    .transpose()
                    .map_err(|error| error.to_string())?;
                transaction
                    .execute(
                        "INSERT INTO run_messages (run_id, session_id, seq, role, content, tool_calls, tool_call_id, name)
                         VALUES (?1,?2,?3,?4,?5,?6,?7,?8)
                         ON CONFLICT(run_id, seq) DO UPDATE SET role=excluded.role, content=excluded.content,
                         tool_calls=excluded.tool_calls, tool_call_id=excluded.tool_call_id, name=excluded.name",
                        params![message.run_id, message.session_id, message.seq, message.role,
                            message.content, tool_calls, message.tool_call_id, message.name],
                    )
                    .map_err(db_error)?;
            }
            transaction.commit().map_err(db_error)
        })
        .await
    }

    pub async fn save_compaction(&self, compaction: ContextCompaction) -> AppResult<()> {
        self.call(move |connection| {
            let pre = i64::try_from(compaction.pre_tokens).map_err(|_| "Invalid compaction tokens")?;
            let post = compaction.post_tokens.map(i64::try_from).transpose().map_err(|_| "Invalid compaction tokens")?;
            connection
                .execute(
                    "INSERT INTO session_compactions
                     (operation_id, session_id, through_run_id, summary, trigger, status, pre_tokens, post_tokens, created_at)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)
                     ON CONFLICT(operation_id) DO UPDATE SET status=excluded.status, summary=excluded.summary,
                     through_run_id=excluded.through_run_id, post_tokens=excluded.post_tokens",
                    params![compaction.operation_id, compaction.session_id, compaction.through_run_id,
                        compaction.summary, compaction.trigger, compaction.status, pre, post, compaction.created_at],
                )
                .map_err(db_error)?;
            Ok(())
        })
        .await
    }

    pub async fn list_compactions(&self, session_id: i64) -> AppResult<Vec<ContextCompaction>> {
        self.call(move |connection| {
            let mut statement = connection
                .prepare("SELECT operation_id, session_id, through_run_id, summary, trigger, status, pre_tokens, post_tokens, created_at FROM session_compactions WHERE session_id=?1 ORDER BY created_at")
                .map_err(db_error)?;
            let rows = statement.query_map([session_id], compaction_row).map_err(db_error)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(db_error)
        })
        .await
    }

    pub async fn recent_subagent_reports(
        &self,
        session_id: i64,
        limit: i64,
    ) -> AppResult<Vec<SubagentView>> {
        self.call(move |connection| {
            let mut statement = connection
                .prepare(
                    "SELECT id, run_id, session_id, agent, description, prompt, model, status, summary,
                            error, content, tool_count, input_tokens, output_tokens,
                            reasoning_tokens, first_token_ms, elapsed_ms, decode_ms, decode_tokens,
                            last_input_tokens,
                            started_at, finished_at, tools
                     FROM subagent_traces
                     WHERE session_id = ?1 AND summary != ''
                     ORDER BY started_at DESC, id DESC LIMIT ?2",
                )
                .map_err(db_error)?;
            let rows = statement.query_map(params![session_id, limit], subagent_row).map_err(db_error)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(db_error)
        })
        .await
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

    /// Persist one run's reasoning bursts (called once when the run finishes).
    pub async fn save_run_reasoning(&self, run_id: i64, segments: &[crate::domain::ReasoningSegment]) -> AppResult<()> {
        let json = serde_json::to_string(segments).map_err(|_| "reasoning serialize failed".to_string())?;
        self.call(move |connection| {
            connection
                .execute("UPDATE runs SET reasoning_json = ?1 WHERE id = ?2", params![json, run_id])
                .map_err(db_error)?;
            Ok(())
        })
        .await
    }

    /// Persist one run's ordered timeline parts (called once when it finishes).
    pub async fn save_run_parts(&self, run_id: i64, parts: &[crate::domain::RunPart]) -> AppResult<()> {
        let rows: Vec<RunPartRow> = parts
            .iter()
            .enumerate()
            .map(|(seq, part)| {
                (
                    seq as i64,
                    part.kind.clone(),
                    part.text.chars().take(16_384).collect(),
                    part.index.map(|value| value as i64),
                    part.started_at,
                    part.ended_at,
                )
            })
            .collect();
        self.call(move |connection| {
            let transaction = connection.transaction().map_err(db_error)?;
            transaction
                .execute("DELETE FROM run_parts WHERE run_id = ?1", [run_id])
                .map_err(db_error)?;
            for (seq, kind, text, tool_index, started_at, ended_at) in rows {
                transaction
                    .execute(
                        "INSERT INTO run_parts (run_id, seq, kind, text, tool_index, started_at, ended_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        params![run_id, seq, kind, text, tool_index, started_at, ended_at],
                    )
                    .map_err(db_error)?;
            }
            transaction.commit().map_err(db_error)?;
            Ok(())
        })
        .await
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
            let cache_write = metrics.cache_write_tokens.map(i64::try_from).transpose().map_err(|_| "Provider cache write exceeds database range")?;
            let reasoning = metrics.reasoning_tokens.map(i64::try_from).transpose().map_err(|_| "Provider reasoning usage exceeds database range")?;
            let attempts = i64::from(metrics.attempts);
            let last_input = metrics.last_input_tokens.map(i64::try_from).transpose().map_err(|_| "Provider token usage exceeds database range")?;
            let last_output = metrics.last_output_tokens.map(i64::try_from).transpose().map_err(|_| "Provider token usage exceeds database range")?;
            let last_elapsed = i64::try_from(metrics.last_elapsed_ms).map_err(|_| "Invalid elapsed time")?;
            let last_first = metrics.last_first_token_ms.map(i64::try_from).transpose().map_err(|_| "Invalid first-token time")?;
            let ttft_steps = i64::from(metrics.ttft_steps);
            let decode_ms = i64::try_from(metrics.decode_ms).map_err(|_| "Invalid decode time")?;
            let decode_tokens = i64::try_from(metrics.decode_tokens).map_err(|_| "Provider token usage exceeds database range")?;
            let tool_ms = i64::try_from(metrics.tool_ms).map_err(|_| "Invalid tool time")?;
            connection.execute("INSERT INTO run_metrics (run_id, session_id, provider, model, input_tokens, output_tokens, cache_hit_tokens, elapsed_ms, first_token_ms, cache_write_tokens, reasoning_tokens, attempts, last_input_tokens, last_output_tokens, last_elapsed_ms, last_first_token_ms, ttft_steps, decode_ms, decode_tokens, tool_ms) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20) ON CONFLICT(run_id) DO UPDATE SET input_tokens=excluded.input_tokens, output_tokens=excluded.output_tokens, cache_hit_tokens=excluded.cache_hit_tokens, elapsed_ms=excluded.elapsed_ms, first_token_ms=excluded.first_token_ms, cache_write_tokens=excluded.cache_write_tokens, reasoning_tokens=excluded.reasoning_tokens, attempts=excluded.attempts, last_input_tokens=excluded.last_input_tokens, last_output_tokens=excluded.last_output_tokens, last_elapsed_ms=excluded.last_elapsed_ms, last_first_token_ms=excluded.last_first_token_ms, ttft_steps=excluded.ttft_steps, decode_ms=excluded.decode_ms, decode_tokens=excluded.decode_tokens, tool_ms=excluded.tool_ms",
                params![metrics.run_id, metrics.session_id, metrics.provider, metrics.model, input, output, cache_hit, elapsed, first, cache_write, reasoning, attempts, last_input, last_output, last_elapsed, last_first, ttft_steps, decode_ms, decode_tokens, tool_ms]).map_err(db_error)?;
            Ok(())
        }).await
    }


    #[allow(clippy::too_many_arguments)]
    pub async fn record_file_change(&self, run_id: i64, session_id: i64, path: String, added: i64, removed: i64, before: String, after: String) -> AppResult<()> {
        self.call(move |connection| {
            connection.execute(
                "INSERT INTO run_file_changes (run_id, session_id, project_id, path, added, removed, before, after)
                 SELECT ?1, ?2, s.project_id, ?3, ?4, ?5, ?6, ?7 FROM sessions s WHERE s.id = ?2",
                params![run_id, session_id, path, added, removed, before, after],
            ).map_err(db_error)?;
            Ok(())
        }).await
    }

    pub async fn run_change_summary(&self, run_id: i64) -> AppResult<RunChangeSummary> {
        self.call(move |connection| {
            connection.query_row(
                "SELECT COUNT(*), COALESCE(SUM(added),0), COALESCE(SUM(removed),0),
                        CASE WHEN COUNT(*) > 0 AND COUNT(reverted_at) = COUNT(*) THEN 'reverted' ELSE 'applied' END
                 FROM (SELECT path, SUM(added) added, SUM(removed) removed, MAX(reverted_at) reverted_at
                       FROM run_file_changes WHERE run_id = ?1 GROUP BY path)",
                [run_id],
                |row| Ok(RunChangeSummary {
                    files: row.get(0)?, additions: row.get(1)?, deletions: row.get(2)?, state: row.get(3)?,
                }),
            ).map_err(db_error)
        }).await
    }

    pub async fn mark_run_changes_reverted(&self, run_id: i64, paths: &[String]) -> AppResult<()> {
        let paths = paths.to_vec();
        self.call(move |connection| {
            for path in paths {
                connection.execute(
                    "UPDATE run_file_changes SET reverted_at = unixepoch() WHERE run_id = ?1 AND path = ?2 AND reverted_at IS NULL",
                    params![run_id, path],
                ).map_err(db_error)?;
            }
            Ok(())
        }).await
    }

    pub async fn run_changes(&self, run_id: i64) -> AppResult<Vec<RunFileChange>> {
        self.call(move |connection| {
            let mut statement = connection.prepare(
                "SELECT path, added, removed, before, after, created_at FROM run_file_changes WHERE run_id = ?1 ORDER BY id"
            ).map_err(db_error)?;
            let rows = statement.query_map([run_id], |row| Ok(RunFileChange {
                path: row.get(0)?, added: row.get(1)?, removed: row.get(2)?,
                before: row.get(3)?, after: row.get(4)?, created_at: row.get(5)?,
            })).map_err(db_error)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(db_error)
        }).await
    }

    pub async fn activity_snapshot(&self, project_id: Option<i64>) -> AppResult<ActivitySnapshot> {
        self.call(move |connection| {
            let mut statement = connection.prepare(
                "WITH RECURSIVE dates(d) AS (
                    SELECT date('now','-179 days')
                    UNION ALL SELECT date(d,'+1 day') FROM dates WHERE d < date('now')
                 )
                 SELECT d,
                   COALESCE((SELECT SUM(f.added + f.removed) FROM run_file_changes f
                     WHERE (?1 IS NULL OR f.project_id = ?1) AND date(f.created_at,'unixepoch') = d), 0),
                   COALESCE((SELECT SUM(COALESCE(m.input_tokens,0) + COALESCE(m.output_tokens,0))
                     FROM run_metrics m JOIN runs r ON r.id = m.run_id JOIN sessions s ON s.id = m.session_id
                     WHERE (?1 IS NULL OR s.project_id = ?1) AND date(r.created_at,'unixepoch') = d), 0),
                   COALESCE((SELECT COUNT(*) FROM plan_cards p JOIN sessions s ON s.id = p.session_id
                     WHERE (?1 IS NULL OR s.project_id = ?1) AND p.status = 'approved'
                       AND p.decided_at IS NOT NULL AND date(p.decided_at,'unixepoch') = d), 0)
                 FROM dates",
            ).map_err(db_error)?;
            let rows = statement.query_map([project_id], |row| {
                let date: String = row.get(0)?;
                let lines: i64 = row.get(1)?;
                Ok(ActivityDay {
                    short_date: date.get(5..).unwrap_or(&date).to_string(),
                    date,
                    lines,
                    tokens: row.get(2)?,
                    plans: row.get(3)?,
                    level: if lines == 0 { 0 } else if lines < 45 { 1 } else if lines < 75 { 2 } else if lines < 100 { 3 } else { 4 },
                })
            }).map_err(db_error)?;
            let days = rows.collect::<Result<Vec<_>, _>>().map_err(db_error)?;
            let start = days.first().map(|day| day.date.clone()).unwrap_or_default();
            let end = days.last().map(|day| day.date.clone()).unwrap_or_default();
            let total_lines = days.iter().map(|day| day.lines).sum();
            let total_tokens = days.iter().map(|day| day.tokens).sum();
            let approved_plans = days.iter().map(|day| day.plans).sum();
            let today_lines = days.last().map(|day| day.lines).unwrap_or(0);
            let trend = days.iter().rev().take(30).rev().cloned().collect();
            Ok(ActivitySnapshot {
                source: "live", start, end, total_lines, approved_plans, total_tokens,
                cost: None, today_lines, days, trend,
            })
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
    let mut statement = connection.prepare("SELECT run_id, session_id, provider, model, input_tokens, output_tokens, cache_hit_tokens, elapsed_ms, first_token_ms, cache_write_tokens, reasoning_tokens, attempts, last_input_tokens, last_output_tokens, last_elapsed_ms, last_first_token_ms, ttft_steps, decode_ms, decode_tokens, tool_ms FROM run_metrics WHERE session_id=?1 ORDER BY run_id").map_err(db_error)?;
    let rows = statement
        .query_map([session_id], |row| {
            let elapsed = number(row, 7)?.unwrap_or(0);
            let first = number(row, 8)?;
            let output = number(row, 5)?;
            let stored_attempts = number(row, 11)?.unwrap_or(0) as u32;
            let attempts = if stored_attempts == 0 && first.is_some() { 1 } else { stored_attempts };
            let stored_ttft = number(row, 16)?.unwrap_or(0) as u32;
            let stored_decode_ms = number(row, 17)?.unwrap_or(0);
            let stored_decode_tokens = number(row, 18)?.unwrap_or(0);
            // v15 写入器保证 first>0 ⇒ ttft_steps≥1；first>0 而 ttft_steps=0
            // 只能是迁移前的旧行，对其按旧口径回退（不回填）。
            let legacy = first.is_some() && stored_ttft == 0;
            Ok(RunMetrics {
                run_id: row.get(0)?,
                session_id: row.get(1)?,
                provider: row.get(2)?,
                model: row.get(3)?,
                input_tokens: number(row, 4)?,
                output_tokens: output,
                cache_hit_tokens: number(row, 6)?,
                elapsed_ms: elapsed,
                first_token_ms: first,
                cache_write_tokens: number(row, 9)?,
                reasoning_tokens: number(row, 10)?,
                attempts,
                last_input_tokens: number(row, 12)?.or(number(row, 4)?),
                last_output_tokens: number(row, 13)?.or(output),
                last_elapsed_ms: number(row, 14)?.filter(|value| *value > 0).unwrap_or(elapsed),
                last_first_token_ms: number(row, 15)?.or(first),
                ttft_steps: if legacy { attempts } else { stored_ttft },
                decode_ms: if legacy && output.is_some() {
                    elapsed.saturating_sub(first.unwrap_or(0))
                } else {
                    stored_decode_ms
                },
                decode_tokens: if legacy { output.unwrap_or(0) } else { stored_decode_tokens },
                tool_ms: number(row, 19)?.unwrap_or(0),
                live_surface_tokens: None,
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

fn run_message_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RunMessage> {
    let tool_calls: Option<String> = row.get(5)?;
    Ok(RunMessage {
        run_id: row.get(0)?,
        session_id: row.get(1)?,
        seq: row.get(2)?,
        role: row.get(3)?,
        content: row.get(4)?,
        tool_calls: tool_calls.and_then(|value| serde_json::from_str(&value).ok()),
        tool_call_id: row.get(6)?,
        name: row.get(7)?,
    })
}

fn compaction_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ContextCompaction> {
    Ok(ContextCompaction {
        operation_id: row.get(0)?,
        session_id: row.get(1)?,
        through_run_id: row.get(2)?,
        summary: row.get(3)?,
        trigger: row.get(4)?,
        status: row.get(5)?,
        pre_tokens: row.get::<_, i64>(6)? as u64,
        post_tokens: row.get::<_, Option<i64>>(7)?.map(|value| value as u64),
        created_at: row.get(8)?,
    })
}

fn subagent_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SubagentView> {
    let tools_json: String = row.get(22)?;
    let tools: Vec<ToolCallView> = serde_json::from_str(&tools_json).unwrap_or_default();
    Ok(SubagentView {
        id: row.get(0)?,
        run_id: row.get(1)?,
        session_id: row.get(2)?,
        agent: row.get(3)?,
        description: row.get(4)?,
        prompt: row.get(5)?,
        model: row.get(6)?,
        status: SubagentStatus::from_str(&row.get::<_, String>(7)?),
        summary: row.get(8)?,
        error: row.get(9)?,
        content: row.get(10)?,
        tool_count: row.get::<_, i64>(11)? as usize,
        input_tokens: row.get::<_, Option<i64>>(12)?.map(|value| value as u64),
        output_tokens: row.get::<_, Option<i64>>(13)?.map(|value| value as u64),
        reasoning_tokens: row.get::<_, Option<i64>>(14)?.map(|value| value as u64),
        first_token_ms: row.get::<_, Option<i64>>(15)?.map(|value| value as u64),
        elapsed_ms: row.get::<_, i64>(16)?.max(0) as u64,
        decode_ms: row.get::<_, Option<i64>>(17)?.unwrap_or(0).max(0) as u64,
        decode_tokens: row.get::<_, Option<i64>>(18)?.unwrap_or(0).max(0) as u64,
        last_input_tokens: row.get::<_, Option<i64>>(19)?.map(|value| value as u64),
        started_at: row.get(20)?,
        finished_at: row.get(21)?,
        reasoning: String::new(),
        tools,
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
        "SELECT id, session_id, prompt, mode, status, output, error, created_at FROM runs WHERE session_id = ?1 ORDER BY id"
    } else {
        "SELECT id, session_id, prompt, mode, status, output, error, created_at FROM runs WHERE ?1 IS NULL ORDER BY id DESC LIMIT 50"
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
                created_at: row.get(7)?,
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
    if version > 15 {
        return Err(format!(
            "Database schema {version} is newer than supported version 15"
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
    if version < 6 {
        transaction
            .execute_batch(
                "CREATE TABLE run_file_changes (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    run_id INTEGER NOT NULL REFERENCES runs(id),
                    session_id INTEGER NOT NULL REFERENCES sessions(id),
                    project_id INTEGER REFERENCES projects(id),
                    path TEXT NOT NULL,
                    added INTEGER NOT NULL,
                    removed INTEGER NOT NULL,
                    created_at INTEGER NOT NULL DEFAULT (unixepoch())
                 );
                 CREATE INDEX run_file_changes_project_date ON run_file_changes(project_id, created_at);
                 PRAGMA user_version = 6;",
            )
            .map_err(db_error)?;
    }
    if version < 7 {
        transaction
            .execute_batch(
                "ALTER TABLE run_file_changes ADD COLUMN before TEXT NOT NULL DEFAULT '';
                 ALTER TABLE run_file_changes ADD COLUMN after TEXT NOT NULL DEFAULT '';
                 PRAGMA user_version = 7;",
            )
            .map_err(db_error)?;
    }
    if version < 8 {
        transaction
            .execute_batch(
                "ALTER TABLE run_file_changes ADD COLUMN reverted_at INTEGER;
                 PRAGMA user_version = 8;",
            )
            .map_err(db_error)?;
    }
    if version < 9 {
        transaction
            .execute_batch(
                "CREATE TABLE subagent_traces (
                    id TEXT PRIMARY KEY,
                    run_id INTEGER NOT NULL REFERENCES runs(id),
                    session_id INTEGER NOT NULL REFERENCES sessions(id),
                    agent TEXT NOT NULL,
                    description TEXT NOT NULL,
                    prompt TEXT NOT NULL,
                    model TEXT,
                    status TEXT NOT NULL,
                    summary TEXT NOT NULL DEFAULT '',
                    error TEXT,
                    content TEXT NOT NULL DEFAULT '',
                    tool_count INTEGER NOT NULL DEFAULT 0,
                    input_tokens INTEGER,
                    output_tokens INTEGER,
                    started_at INTEGER NOT NULL DEFAULT (unixepoch()),
                    finished_at INTEGER,
                    tools TEXT NOT NULL DEFAULT '[]'
                 );
                 CREATE INDEX subagent_traces_session ON subagent_traces(session_id, started_at);
                 PRAGMA user_version = 9;",
            )
            .map_err(db_error)?;
    }
    if version < 10 {
        transaction
            .execute_batch(
                "ALTER TABLE run_metrics ADD COLUMN cache_write_tokens INTEGER;
                 ALTER TABLE run_metrics ADD COLUMN reasoning_tokens INTEGER;
                 ALTER TABLE run_metrics ADD COLUMN attempts INTEGER NOT NULL DEFAULT 0;
                 ALTER TABLE run_metrics ADD COLUMN last_input_tokens INTEGER;
                 ALTER TABLE run_metrics ADD COLUMN last_output_tokens INTEGER;
                 ALTER TABLE run_metrics ADD COLUMN last_elapsed_ms INTEGER NOT NULL DEFAULT 0;
                 ALTER TABLE run_metrics ADD COLUMN last_first_token_ms INTEGER;
                 PRAGMA user_version = 10;",
            )
            .map_err(db_error)?;
    }
    if version < 11 {
        transaction
            .execute_batch(
                "CREATE TABLE session_compactions (
                    operation_id TEXT PRIMARY KEY,
                    session_id INTEGER NOT NULL REFERENCES sessions(id),
                    through_run_id INTEGER NOT NULL,
                    summary TEXT NOT NULL,
                    trigger TEXT NOT NULL,
                    status TEXT NOT NULL,
                    pre_tokens INTEGER NOT NULL DEFAULT 0,
                    post_tokens INTEGER,
                    created_at INTEGER NOT NULL DEFAULT (unixepoch())
                 );
                 CREATE INDEX session_compactions_session ON session_compactions(session_id, created_at);
                 PRAGMA user_version = 11;",
            )
            .map_err(db_error)?;
    }
    if version < 12 {
        transaction
            .execute_batch(
                "CREATE TABLE run_messages (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    run_id INTEGER NOT NULL REFERENCES runs(id),
                    session_id INTEGER NOT NULL REFERENCES sessions(id),
                    seq INTEGER NOT NULL,
                    role TEXT NOT NULL,
                    content TEXT NOT NULL DEFAULT '',
                    tool_calls TEXT,
                    tool_call_id TEXT,
                    name TEXT,
                    created_at INTEGER NOT NULL DEFAULT (unixepoch())
                 );
                 CREATE UNIQUE INDEX run_messages_run_seq ON run_messages(run_id, seq);
                 CREATE INDEX run_messages_session ON run_messages(session_id, run_id, seq);
                 PRAGMA user_version = 12;",
            )
            .map_err(db_error)?;
    }
    if version < 13 {
        transaction
            .execute_batch(
                "ALTER TABLE runs ADD COLUMN reasoning_json TEXT NOT NULL DEFAULT '[]';
                 CREATE TABLE run_parts (
                    run_id INTEGER NOT NULL REFERENCES runs(id) ON DELETE CASCADE,
                    seq INTEGER NOT NULL,
                    kind TEXT NOT NULL,
                    text TEXT NOT NULL DEFAULT '',
                    tool_index INTEGER,
                    started_at INTEGER NOT NULL DEFAULT 0,
                    ended_at INTEGER,
                    PRIMARY KEY (run_id, seq)
                 );
                 CREATE INDEX run_parts_run ON run_parts(run_id, seq);
                 PRAGMA user_version = 13;",
            )
            .map_err(db_error)?;
    }
    if version < 14 {
        transaction
            .execute_batch(
                "ALTER TABLE subagent_traces ADD COLUMN reasoning_tokens INTEGER;
                 ALTER TABLE subagent_traces ADD COLUMN first_token_ms INTEGER;
                 ALTER TABLE subagent_traces ADD COLUMN elapsed_ms INTEGER NOT NULL DEFAULT 0;
                 ALTER TABLE subagent_traces ADD COLUMN last_input_tokens INTEGER;
                 PRAGMA user_version = 14;",
            )
            .map_err(db_error)?;
    }
    if version < 15 {
        transaction
            .execute_batch(
                "ALTER TABLE run_metrics ADD COLUMN ttft_steps INTEGER;
                 ALTER TABLE run_metrics ADD COLUMN decode_ms INTEGER;
                 ALTER TABLE run_metrics ADD COLUMN decode_tokens INTEGER;
                 ALTER TABLE run_metrics ADD COLUMN tool_ms INTEGER;
                 ALTER TABLE subagent_traces ADD COLUMN decode_ms INTEGER;
                 ALTER TABLE subagent_traces ADD COLUMN decode_tokens INTEGER;
                 PRAGMA user_version = 15;",
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
    transaction
        .execute(
            "UPDATE subagent_traces SET status = 'failed', error = 'Application exited before completion',
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
                        created_at: 0,
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
                created_at: 0,
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
            created_at: 0,
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
    async fn migrating_a_v13_database_adds_subagent_usage_columns() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("v13.sqlite3");
        let database = Database::open(path.clone()).await.unwrap();
        database
            .call(|connection| {
                connection
                    .execute_batch(
                        "ALTER TABLE subagent_traces DROP COLUMN reasoning_tokens;
                         ALTER TABLE subagent_traces DROP COLUMN first_token_ms;
                         ALTER TABLE subagent_traces DROP COLUMN elapsed_ms;
                         ALTER TABLE subagent_traces DROP COLUMN last_input_tokens;
                         ALTER TABLE subagent_traces DROP COLUMN decode_ms;
                         ALTER TABLE subagent_traces DROP COLUMN decode_tokens;
                         ALTER TABLE run_metrics DROP COLUMN ttft_steps;
                         ALTER TABLE run_metrics DROP COLUMN decode_ms;
                         ALTER TABLE run_metrics DROP COLUMN decode_tokens;
                         ALTER TABLE run_metrics DROP COLUMN tool_ms;
                         PRAGMA user_version = 13;",
                    )
                    .map_err(db_error)
            })
            .await
            .unwrap();
        database.shutdown().await.unwrap();

        let reopened = Database::open(path).await.unwrap();
        let (columns, version) = reopened
            .call(|connection| {
                let mut statement = connection
                    .prepare("PRAGMA table_info(subagent_traces)")
                    .map_err(db_error)?;
                let names = statement
                    .query_map([], |row| row.get::<_, String>(1))
                    .map_err(db_error)?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(db_error)?;
                let version: i64 = connection
                    .pragma_query_value(None, "user_version", |row| row.get(0))
                    .map_err(db_error)?;
                Ok((names, version))
            })
            .await
            .unwrap();
        for column in ["reasoning_tokens", "first_token_ms", "elapsed_ms", "last_input_tokens"] {
            assert!(columns.iter().any(|name| name == column), "missing {column}");
        }
        assert_eq!(version, 15);
        reopened.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn migrating_a_v12_database_adds_run_parts_and_reasoning() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("v12.sqlite3");
        let database = Database::open(path.clone()).await.unwrap();
        // Simulate a database that stopped at v12, before run_parts existed.
        database
            .call(|connection| {
                connection
                    .execute_batch(
                        "DROP TABLE run_parts;
                         ALTER TABLE runs DROP COLUMN reasoning_json;
                         ALTER TABLE subagent_traces DROP COLUMN reasoning_tokens;
                         ALTER TABLE subagent_traces DROP COLUMN first_token_ms;
                         ALTER TABLE subagent_traces DROP COLUMN elapsed_ms;
                         ALTER TABLE subagent_traces DROP COLUMN last_input_tokens;
                         ALTER TABLE subagent_traces DROP COLUMN decode_ms;
                         ALTER TABLE subagent_traces DROP COLUMN decode_tokens;
                         ALTER TABLE run_metrics DROP COLUMN ttft_steps;
                         ALTER TABLE run_metrics DROP COLUMN decode_ms;
                         ALTER TABLE run_metrics DROP COLUMN decode_tokens;
                         ALTER TABLE run_metrics DROP COLUMN tool_ms;
                         PRAGMA user_version = 12;",
                    )
                    .map_err(db_error)
            })
            .await
            .unwrap();
        database.shutdown().await.unwrap();

        let reopened = Database::open(path).await.unwrap();
        let (has_parts, version) = reopened
            .call(|connection| {
                let has_parts: i64 = connection
                    .query_row(
                        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='run_parts'",
                        [],
                        |row| row.get(0),
                    )
                    .map_err(db_error)?;
                let version: i64 = connection
                    .pragma_query_value(None, "user_version", |row| row.get(0))
                    .map_err(db_error)?;
                Ok((has_parts, version))
            })
            .await
            .unwrap();
        assert_eq!(has_parts, 1, "run_parts must be created when upgrading from v12");
        assert_eq!(version, 15);
        reopened.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn newer_database_schema_is_rejected_without_downgrading_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("future.sqlite3");
        let database = Database::open(path.clone()).await.unwrap();
        database
            .call(|connection| {
                connection
                    .pragma_update(None, "user_version", 16)
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
                cache_write_tokens: None,
                reasoning_tokens: None,
                attempts: 1,
                last_input_tokens: None,
                last_output_tokens: Some(10),
                last_elapsed_ms: 250,
                last_first_token_ms: Some(20),
                ttft_steps: 1,
                decode_ms: 230,
                decode_tokens: 10,
                tool_ms: 0,
                live_surface_tokens: None,
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

        // Archived projects reject new sessions.
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
        assert!(database.delete_project(999).await.is_err());

        // The seeded project is a normal project: it can be archived and restored.
        database
            .set_project_archived(DEFAULT_PROJECT_ID, true)
            .await
            .unwrap();
        assert!(database.list_projects().await.unwrap()[0].archived);
        database
            .set_project_archived(DEFAULT_PROJECT_ID, false)
            .await
            .unwrap();

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
                created_at: 0,
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
                cache_write_tokens: None,
                reasoning_tokens: None,
                attempts: 1,
                last_input_tokens: None,
                last_output_tokens: Some(4),
                last_elapsed_ms: 9,
                last_first_token_ms: None,
                ttft_steps: 0,
                decode_ms: 0,
                decode_tokens: 0,
                tool_ms: 0,
                live_surface_tokens: None,
            })
            .await
            .unwrap();
        database
            .record_file_change(
                run_id,
                moved.id,
                "a.txt".into(),
                1,
                0,
                String::new(),
                "x".into(),
            )
            .await
            .unwrap();

        // Deletion cascades sessions, runs, plans, metrics, file changes and audit rows.
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
        let change_orphans: i64 = database
            .call(move |connection| {
                connection
                    .query_row(
                        "SELECT COUNT(*) FROM run_file_changes WHERE session_id = ?1",
                        [moved.id],
                        |row| row.get(0),
                    )
                    .map_err(db_error)
            })
            .await
            .unwrap();
        assert_eq!(change_orphans, 0);
        assert_eq!(
            database
                .session_detail(session.id)
                .await
                .unwrap()
                .session
                .title,
            "重命名"
        );

        // The seeded project is deletable as well, and its sessions go with it.
        database.delete_project(DEFAULT_PROJECT_ID).await.unwrap();
        assert!(database.session_detail(session.id).await.is_err());
        assert!(database.list_projects().await.unwrap().is_empty());
        assert!(database.first_active_project_id().await.unwrap().is_none());
        database.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn subagent_traces_persist_finish_and_cascade() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("traces.sqlite3");
        let database = Database::open(path.clone()).await.unwrap();
        let session = database.create_session().await.unwrap();
        let run_id = database
            .create_run_in_session(session.id, "work".into(), Mode::Plan)
            .await
            .unwrap();
        let trace = SubagentView {
            id: "sa-1-1".into(),
            run_id,
            session_id: session.id,
            agent: "explore".into(),
            description: "调研".into(),
            prompt: "读文件".into(),
            model: Some("m".into()),
            status: SubagentStatus::Running,
            summary: String::new(),
            error: None,
            content: String::new(),
            reasoning: String::new(),
            tool_count: 0,
            input_tokens: None,
            output_tokens: None,
            reasoning_tokens: None,
            first_token_ms: None,
            elapsed_ms: 0,
            decode_ms: 0,
            decode_tokens: 0,
            last_input_tokens: None,
            started_at: 1,
            finished_at: None,
            tools: Vec::new(),
        };
        database.insert_subagent_trace(trace.clone()).await.unwrap();
        let listed = database.list_subagent_traces(session.id).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].status, SubagentStatus::Running);

        let tools = vec![ToolCallView {
            run_id,
            tool: "read_file".into(),
            detail: "a.rs".into(),
            allowed: true,
            subagent_id: Some("sa-1-1".into()),
            pending: None,
            target: Some("a.rs".into()),
        }];
        database
            .finish_subagent_trace(
                "sa-1-1".into(),
                SubagentStatus::Completed,
                "【子智能体 explore】完成".into(),
                None,
                "结论".into(),
                tools,
                Some(3),
                Some(4),
                Some(5),
                Some(100),
                1_000,
                800,
                4,
                Some(3),
                2,
            )
            .await
            .unwrap();
        let listed = database.list_subagent_traces(session.id).await.unwrap();
        assert_eq!(listed[0].status, SubagentStatus::Completed);
        assert_eq!(listed[0].tool_count, 1);
        assert_eq!(listed[0].input_tokens, Some(3));
        assert_eq!(listed[0].tools[0].subagent_id.as_deref(), Some("sa-1-1"));
        assert_eq!(database.recent_subagent_reports(session.id, 6).await.unwrap().len(), 1);

        database.shutdown().await.unwrap();
        let database = Database::open(path).await.unwrap();
        database.delete_session(session.id).await.unwrap();
        assert!(database.list_subagent_traces(session.id).await.unwrap().is_empty());
        database.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn running_subagent_traces_are_failed_on_restart() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("interrupted.sqlite3");
        let database = Database::open(path.clone()).await.unwrap();
        let session = database.create_session().await.unwrap();
        let run_id = database
            .create_run_in_session(session.id, "work".into(), Mode::Plan)
            .await
            .unwrap();
        database
            .insert_subagent_trace(SubagentView {
                id: "sa-1-2".into(),
                run_id,
                session_id: session.id,
                agent: "explore".into(),
                description: "调研".into(),
                prompt: "读文件".into(),
                model: None,
                status: SubagentStatus::Running,
                summary: String::new(),
                error: None,
                content: String::new(),
                reasoning: String::new(),
                tool_count: 0,
                input_tokens: None,
                output_tokens: None,
                reasoning_tokens: None,
                first_token_ms: None,
                elapsed_ms: 0,
                decode_ms: 0,
                decode_tokens: 0,
                last_input_tokens: None,
                started_at: 1,
                finished_at: None,
                tools: Vec::new(),
            })
            .await
            .unwrap();
        database.shutdown().await.unwrap();
        let database = Database::open(path).await.unwrap();
        let listed = database.list_subagent_traces(session.id).await.unwrap();
        assert_eq!(listed[0].status, SubagentStatus::Failed);
        assert!(listed[0].error.as_deref().unwrap_or_default().contains("Application exited"));
        database.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn first_active_project_skips_archived_projects() {
        let database = Database::open(":memory:".into()).await.unwrap();
        assert_eq!(
            database.first_active_project_id().await.unwrap(),
            Some(DEFAULT_PROJECT_ID)
        );
        let side = database.create_project("Side".into(), None).await.unwrap();
        database
            .set_project_archived(DEFAULT_PROJECT_ID, true)
            .await
            .unwrap();
        assert_eq!(database.first_active_project_id().await.unwrap(), Some(side.id));
        database.set_project_archived(side.id, true).await.unwrap();
        assert_eq!(database.first_active_project_id().await.unwrap(), None);
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

    #[tokio::test]
    async fn run_change_summary_tracks_files_and_reverted_state() {
        let directory = tempfile::tempdir().unwrap();
        let database = Database::open(directory.path().join("changes.sqlite3")).await.unwrap();
        database
            .call(|connection| {
                connection
                    .execute_batch(
                        "INSERT INTO sessions(id,title,project_id) VALUES(1,'s',1);
                         INSERT INTO runs(id,prompt,mode,status,session_id) VALUES(1,'p','code','completed',1);
                         INSERT INTO run_file_changes(run_id,session_id,project_id,path,added,removed,before,after)
                         VALUES(1,1,1,'a.ts',2,1,'old','new');",
                    )
                    .map_err(db_error)
            })
            .await
            .unwrap();
        let summary = database.run_change_summary(1).await.unwrap();
        assert_eq!(summary.files, 1);
        assert_eq!(summary.additions, 2);
        assert_eq!(summary.deletions, 1);
        assert_eq!(summary.state, "applied");
        database
            .mark_run_changes_reverted(1, &["a.ts".to_string()])
            .await
            .unwrap();
        assert_eq!(database.run_change_summary(1).await.unwrap().state, "reverted");
        assert_eq!(database.run_changes(1).await.unwrap()[0].before, "old");
        database.shutdown().await.unwrap();
    }
}
