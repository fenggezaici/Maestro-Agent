//! SQLite is confined to one OS thread. Async callers only exchange messages.
use std::{path::PathBuf, sync::Mutex, thread, time::Duration};

use rusqlite::{params, Connection};
use tokio::sync::{mpsc, oneshot};

use crate::model::{
    AppResult, Mode, PlanCard, PlanDecision, PlanDraft, PlanStatus, RunStatus, RunView,
    SessionDetail, SessionView,
};

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

    pub async fn create_session(&self) -> AppResult<SessionView> {
        self.call(|connection| {
            connection
                .execute("INSERT INTO sessions (title) VALUES ('新会话')", [])
                .map_err(db_error)?;
            read_session_view(connection, connection.last_insert_rowid())
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
            let mut statement = connection.prepare(
                "SELECT s.id, s.title, s.updated_at, (SELECT COUNT(*) FROM runs WHERE session_id = s.id)
                 FROM sessions s ORDER BY s.updated_at DESC, s.id DESC"
            ).map_err(db_error)?;
            let rows = statement.query_map([], session_row).map_err(db_error)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(db_error)
        }).await
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
            Ok(SessionDetail { session, runs, plans })
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

fn session_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionView> {
    Ok(SessionView {
        id: row.get(0)?,
        title: row.get(1)?,
        updated_at: row.get(2)?,
        run_count: row.get(3)?,
    })
}

fn read_session_view(connection: &Connection, id: i64) -> AppResult<SessionView> {
    connection.query_row("SELECT s.id, s.title, s.updated_at, (SELECT COUNT(*) FROM runs WHERE session_id = s.id) FROM sessions s WHERE s.id = ?1", [id], session_row).map_err(db_error)
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
                mode: if mode == "execute" {
                    Mode::Execute
                } else {
                    Mode::Plan
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
    if version > 2 {
        return Err(format!(
            "Database schema {version} is newer than supported version 2"
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
                    .pragma_update(None, "user_version", 3)
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
}
