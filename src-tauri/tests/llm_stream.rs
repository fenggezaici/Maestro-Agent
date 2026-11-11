use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use maestro_lib::{
    adapters::llm::{ChatConfig, Provider},
    domain::{ChatMessage, EventData, KernelEvent, Mode, RunStatus, RunView, SessionDetail, SessionView},
    services::{agent_loop::conversation, kernel::Kernel},
};
use serde_json::Value;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::oneshot,
};

async fn provider(
    status: u16,
    parts: Vec<(u64, &'static str)>,
) -> (
    String,
    oneshot::Receiver<Value>,
    tokio::task::JoinHandle<()>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let (sent, received) = oneshot::channel();
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        socket.set_nodelay(true).unwrap();
        let mut request = Vec::new();
        let mut buffer = [0u8; 4096];
        let body = loop {
            let count = socket.read(&mut buffer).await.unwrap();
            assert!(count > 0);
            request.extend_from_slice(&buffer[..count]);
            if let Some(end) = request.windows(4).position(|value| value == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
                let length: usize = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .unwrap()
                    .trim()
                    .parse()
                    .unwrap();
                if request.len() >= end + 4 + length {
                    break serde_json::from_slice(&request[end + 4..end + 4 + length]).unwrap();
                }
            }
        };
        let _ = sent.send(body);
        let length: usize = parts.iter().map(|(_, body)| body.len()).sum();
        let header = format!("HTTP/1.1 {status} Test\r\nContent-Type: text/event-stream\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n");
        if socket.write_all(header.as_bytes()).await.is_err() {
            return;
        }
        for (delay, part) in parts {
            tokio::time::sleep(Duration::from_millis(delay)).await;
            if socket.write_all(part.as_bytes()).await.is_err() {
                return;
            }
        }
    });
    (url, received, task)
}

fn config(base_url: String) -> ChatConfig {
    ChatConfig {
        api_key: "test-secret".into(),
        base_url,
        model: "local-test-model".into(),
        provider: Provider::Openai,
        reasoning: None,
        max_output: None,
        session: None,
    }
}

async fn idle(kernel: &Kernel) {
    tokio::time::timeout(Duration::from_secs(6), async {
        while kernel.snapshot().await.active_run.is_some() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn real_http_sse_streams_before_completion_and_persists_with_rust_plan_prompt() {
    let (url, request, server) = provider(
        200,
        vec![
            (
                0,
                "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n",
            ),
            (
                250,
                "data: {\"choices\":[{\"delta\":{\"content\":\" 世界\"}}]}\n\n",
            ),
            (20, "data: [DONE]\n\n"),
        ],
    )
    .await;
    let events: Arc<Mutex<Vec<KernelEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let captured = events.clone();
    let kernel = Kernel::open(
        ":memory:".into(),
        tempfile::tempdir().unwrap().keep(),
        Arc::new(move |event| captured.lock().unwrap().push(event)),
    )
    .await
    .unwrap();
    let session = kernel.create_session(None).await.unwrap();
    let id = kernel
        .start_model(session.id, "current user question".into(), config(url))
        .await
        .unwrap();
    let body = tokio::time::timeout(Duration::from_secs(3), request)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(body["stream"], true);
    assert_eq!(body["model"], "local-test-model");
    assert!(body["messages"][0]["content"]
        .as_str()
        .unwrap()
        .contains("你处于只读计划模式"));
    assert!(body["messages"][0]["content"]
        .as_str()
        .unwrap()
        .contains("工作区之外"));
    assert!(!body.to_string().contains("test-secret"));
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(run) = kernel.snapshot().await.active_run {
                if run.output == "Hello" {
                    assert_eq!(run.status, RunStatus::Running);
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    idle(&kernel).await;
    let detail = kernel.session_detail(session.id).await.unwrap();
    assert_eq!(detail.runs[0].output, "Hello 世界");
    assert_eq!(detail.runs[0].status, RunStatus::Completed);
    let terminal_count = events.lock().unwrap().iter().filter(|event| matches!(&event.data, EventData::Status { run } if run.id == id && run.status != RunStatus::Running)).count();
    assert_eq!(terminal_count, 1);
    kernel.shutdown().await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn http_errors_and_cancellation_finish_without_panicking() {
    let (url, _, server) = provider(401, vec![]).await;
    let kernel = Kernel::open(
        ":memory:".into(),
        tempfile::tempdir().unwrap().keep(),
        Arc::new(|_| {}),
    )
    .await
    .unwrap();
    let session = kernel.create_session(None).await.unwrap();
    kernel
        .start_model(session.id, "auth failure".into(), config(url))
        .await
        .unwrap();
    idle(&kernel).await;
    assert_eq!(
        kernel.session_detail(session.id).await.unwrap().runs[0].status,
        RunStatus::Failed
    );
    assert!(kernel.snapshot().await.ready);
    server.await.unwrap();
    let (url, request, server) = provider(200, vec![(1500, "data: [DONE]\n\n")]).await;
    let id = kernel
        .start_model(session.id, "cancel request".into(), config(url))
        .await
        .unwrap();
    request.await.unwrap();
    kernel.cancel(id).await.unwrap();
    idle(&kernel).await;
    assert_eq!(
        kernel.snapshot().await.last_run.unwrap().status,
        RunStatus::Cancelled
    );
    kernel.shutdown().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn anthropic_messages_sse_and_usage_are_adapted_without_mixing_system_roles() {
    let (url, request, server) = provider(
        200,
        vec![
            (
                0,
                "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"usage\":{\"input_tokens\":24,\"output_tokens\":1}}}\n\n",
            ),
            (
                0,
                "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Bonjour\"}}\n\n",
            ),
            (
                30,
                "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\" monde\"}}\n\n",
            ),
            (
                20,
                "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":7}}\n\n",
            ),
            (
                0,
                "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
            ),
        ],
    )
    .await;
    let kernel = Kernel::open(
        ":memory:".into(),
        tempfile::tempdir().unwrap().keep(),
        Arc::new(|_| {}),
    )
    .await
    .unwrap();
    let session = kernel.create_session(None).await.unwrap();
    let mut settings = config(url);
    settings.provider = Provider::Anthropic;
    settings.model = "claude-3-7-sonnet-latest".into();
    settings.reasoning = Some("medium".into());
    kernel
        .start_model(session.id, "anthropic request".into(), settings)
        .await
        .unwrap();
    let body = tokio::time::timeout(Duration::from_secs(3), request)
        .await
        .unwrap()
        .unwrap();
    assert!(body["system"].as_str().unwrap().contains("只读计划模式"));
    assert!(body["messages"]
        .as_array()
        .unwrap()
        .iter()
        .all(|message| message["role"] != "system"));
    assert_eq!(body["thinking"]["budget_tokens"], 4096);
    assert_eq!(body["max_tokens"], 8192);
    idle(&kernel).await;
    let detail = kernel.session_detail(session.id).await.unwrap();
    assert_eq!(detail.runs[0].output, "Bonjour monde");
    assert_eq!(detail.runs[0].status, RunStatus::Completed);
    assert_eq!(detail.metrics[0].input_tokens, Some(24));
    assert_eq!(detail.metrics[0].output_tokens, Some(7));
    kernel.shutdown().await.unwrap();
    server.await.unwrap();
}

#[test]
fn history_is_session_scoped_and_the_current_user_message_is_inserted_once() {
    let current = RunView {
        id: 3,
        session_id: 1,
        prompt: "current".into(),
        mode: Mode::Plan,
        status: RunStatus::Running,
        output: String::new(),
        error: None,
        created_at: 0,
    };
    let prior = RunView {
        id: 1,
        prompt: "prior".into(),
        status: RunStatus::Completed,
        output: "answer".into(),
        ..current.clone()
    };
    let foreign = RunView {
        id: 2,
        session_id: 2,
        prompt: "foreign".into(),
        ..prior.clone()
    };
    let detail = SessionDetail {
        session: SessionView {
            id: 1,
            title: "session".into(),
            updated_at: 1,
            run_count: 2,
            project_id: 1,
            archived: false,
        },
        runs: vec![prior, foreign, current.clone()],
        plans: Vec::new(),
        metrics: Vec::new(),
        tools: Vec::new(),
    };
    let messages: Vec<ChatMessage> = conversation(&detail, &current);
    assert_eq!(messages.len(), 4);
    assert_eq!(messages[0].role, "system");
    assert_eq!(
        messages
            .iter()
            .filter(|message| message.content == "current")
            .count(),
        1
    );
    assert!(messages.iter().all(|message| message.content != "foreign"));
}
