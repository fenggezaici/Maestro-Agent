use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use maestro_lib::{
    adapters::llm::{ChatConfig, Provider},
    domain::{ChatMessage, EventData, KernelEvent, Mode, RunStatus, RunView, SessionDetail, SessionView, ToolPhase},
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

async fn provider_truncated_then_ok() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let mut attempt = 0;
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { return };
            let _ = socket.set_nodelay(true);
            // Consume the whole request before replying: the chat body is far
            // larger than one read, and answering while the client is still
            // uploading makes reqwest report a send error instead of the
            // truncation this test is about. An aborted connection (no bytes)
            // is skipped rather than treated as the request.
            let mut received = false;
            loop {
                let mut buffer = [0u8; 8192];
                match tokio::time::timeout(Duration::from_millis(100), socket.read(&mut buffer)).await {
                    Ok(Ok(0)) | Ok(Err(_)) => break,
                    Ok(Ok(_)) => received = true,
                    Err(_) => break,
                }
            }
            if !received {
                continue;
            }
            if attempt == 0 {
                let _ = socket
                    .write_all(
                        b"HTTP/1.1 200 OK
Content-Type: text/event-stream
Transfer-Encoding: chunked
Connection: close

",
                    )
                    .await;
                // Close without the terminating zero chunk: reqwest reports a
                // transport error before any SSE frame is delivered.
                attempt += 1;
                continue;
            }
            let body = "data: {\"choices\":[{\"delta\":{\"content\":\"Recovered\"}}]}

data: [DONE]

";
            let header = format!(
                "HTTP/1.1 200 OK
Content-Type: text/event-stream
Content-Length: {}
Connection: close

",
                body.len()
            );
            let _ = socket.write_all(header.as_bytes()).await;
            let _ = socket.write_all(body.as_bytes()).await;
            break;
        }
    });
    (url, task)
}

async fn provider_reasoning_then_truncated_then_ok() -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let mut attempt = 0;
        loop {
            let Ok((mut socket, _)) = listener.accept().await else { return };
            let _ = socket.set_nodelay(true);
            let mut received = false;
            loop {
                let mut buffer = [0u8; 8192];
                match tokio::time::timeout(Duration::from_millis(100), socket.read(&mut buffer)).await {
                    Ok(Ok(0)) | Ok(Err(_)) => break,
                    Ok(Ok(_)) => received = true,
                    Err(_) => break,
                }
            }
            if !received {
                continue;
            }
            if attempt == 0 {
                attempt += 1;
                // 思考 delta 已交付后 chunked 截断：正文 Token 未流出 →
                // 客户端应整步重发（reasoning 折叠段重复为已知代价）。
                let frame = "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"thinking…\"}}]}\n\n";
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{frame}\r\n",
                    frame.len()
                );
                let _ = socket.write_all(head.as_bytes()).await;
                continue;
            }
            let body = "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"thinking2\"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"Recovered\"}}]}\n\ndata: [DONE]\n\n";
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = socket.write_all(header.as_bytes()).await;
            let _ = socket.write_all(body.as_bytes()).await;
            break;
        }
    });
    (url, task)
}

/// 读完整条请求（headers 结束 + Content-Length 字节）才允许回包——
/// 大请求体（长上下文、多轮历史）分多段到达，静默等待式读法会提前
/// 断供，reqwest 可能把响应前残余上传误报为发送错误。
async fn consume_request(socket: &mut tokio::net::TcpStream) {
    let mut request = Vec::new();
    let mut buffer = [0u8; 8192];
    loop {
        let count = match tokio::time::timeout(Duration::from_secs(5), socket.read(&mut buffer)).await {
            Ok(Ok(0)) | Ok(Err(_)) | Err(_) => return,
            Ok(Ok(count)) => count,
        };
        request.extend_from_slice(&buffer[..count]);
        if let Some(end) = request.windows(4).position(|value| value == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&request[..end]).to_ascii_lowercase();
            let length: usize = headers
                .lines()
                .find_map(|line| line.strip_prefix("content-length:"))
                .unwrap_or("0")
                .trim()
                .parse()
                .unwrap_or(0);
            if request.len() >= end + 4 + length {
                return;
            }
        }
    }
}

async fn write_sse(socket: &mut tokio::net::TcpStream, body: &str) -> bool {
    let header = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    socket.write_all(header.as_bytes()).await.is_ok() && socket.write_all(body.as_bytes()).await.is_ok()
}

/// 依次给每个连接回 bodies[i]（超出后重复最后一个），用于多轮响应场景。
async fn provider_sequence(bodies: Vec<&'static str>) -> (String, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let mut attempt = 0usize;
        while let Ok((mut socket, _)) = listener.accept().await {
            let _ = socket.set_nodelay(true);
            consume_request(&mut socket).await;
            let body = bodies[attempt.min(bodies.len() - 1)];
            attempt += 1;
            if !write_sse(&mut socket, body).await {
                return;
            }
        }
    });
    (url, task)
}

/// 每次请求都回同一个 body 的假 provider；按请求数返回 attempts 计数。
async fn provider_repeating(body: &'static str) -> (String, Arc<std::sync::atomic::AtomicUsize>, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = attempts.clone();
    let task = tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let _ = socket.set_nodelay(true);
            consume_request(&mut socket).await;
            if !write_sse(&mut socket, body).await {
                return;
            }
        }
    });
    (url, attempts, task)
}

/// 首个连接回「无名 tool-call 碎片 + finish_reason: tool_calls」，后续连接回正文。
async fn provider_nameless_fragments_then_text() -> (String, tokio::task::JoinHandle<()>) {
    provider_sequence(vec![
        concat!(
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"先去读文件\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c9\",\"function\":{\"arguments\":\"{\\\"path\\\":\\\"a\\\"}\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n"
        ),
        "data: {\"choices\":[{\"delta\":{\"content\":\"Recovered\"}}]}\n\ndata: [DONE]\n\n",
    ])
    .await
}

fn config(base_url: String) -> ChatConfig {
    ChatConfig {
        api_key: "test-secret".into(),
        base_url,
        model: "local-test-model".into(),
        provider: Provider::Openai,
        reasoning: None,
        max_output: None,
        context_window: None,
        session: None,
    }
}

async fn idle(kernel: &Kernel) {
    // The gateway retry path backs off ~2s + ~4s (plus jitter) before the
    // second attempt, so a 6s ceiling was flaky under parallel test load.
    tokio::time::timeout(Duration::from_secs(15), async {
        while kernel.snapshot().await.active_run.is_some() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn retries_once_when_the_gateway_closes_before_the_first_chunk() {
    let (url, server) = provider_truncated_then_ok().await;
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
    kernel
        .start_model(session.id, "retry this".into(), Vec::new(), config(url))
        .await
        .unwrap();
    idle(&kernel).await;
    let runs = kernel.list_runs().await.unwrap();
    assert_eq!(runs[0].status, RunStatus::Completed, "{:?}", runs[0].error);
    assert!(runs[0].output.contains("Recovered"));
    server.await.unwrap();
    kernel.shutdown().await.unwrap();
}

#[tokio::test]
async fn retries_mid_stream_when_only_reasoning_has_flown() {
    // DSH：正文 Token 未流出时传输失败整步重发；失败 attempt 的思考
    // 保留在折叠段内，不重试正文。
    let (url, server) = provider_reasoning_then_truncated_then_ok().await;
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
    kernel
        .start_model(session.id, "retry this".into(), Vec::new(), config(url))
        .await
        .unwrap();
    idle(&kernel).await;
    let runs = kernel.list_runs().await.unwrap();
    assert_eq!(runs[0].status, RunStatus::Completed, "{:?}", runs[0].error);
    assert!(runs[0].output.contains("Recovered"));
    let reasoning: Vec<String> = events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match &event.data {
            EventData::Reasoning { token, .. } => Some(token.clone()),
            _ => None,
        })
        .collect();
    assert!(reasoning.iter().any(|token| token.contains("thinking…")), "{reasoning:?}");
    assert!(reasoning.iter().any(|token| token.contains("thinking2")), "{reasoning:?}");
    server.await.unwrap();
    kernel.shutdown().await.unwrap();
}

#[tokio::test]
async fn nameless_tool_fragments_nudge_the_model_instead_of_silent_completion() {
    // vLLM/DSML 长上下文缺陷复现：provider 流出发了工具调用碎片却始终
    // 缺 function.name → 全部丢弃。空轮应触发追问而不是静默完成；
    // 追问后第二轮 provider 回正文，run 带正文完成。
    let (url, server) = provider_nameless_fragments_then_text().await;
    let kernel = Kernel::open(
        ":memory:".into(),
        tempfile::tempdir().unwrap().keep(),
        Arc::new(|_| {}),
    )
    .await
    .unwrap();
    let session = kernel.create_session(None).await.unwrap();
    kernel
        .start_model(session.id, "do work".into(), Vec::new(), config(url))
        .await
        .unwrap();
    idle(&kernel).await;
    let runs = kernel.list_runs().await.unwrap();
    assert_eq!(runs[0].status, RunStatus::Completed, "{:?}", runs[0].error);
    assert!(runs[0].output.contains("Recovered"), "{}", runs[0].output);
    kernel.shutdown().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn persistent_empty_turns_finish_with_a_visible_marker() {
    // 追问耗尽仍零正文零可解析调用 → 以用户可见标记收尾，绝不静默。
    let (url, attempts, server) = provider_repeating(concat!(
        "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"…\"}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"{}\"}}]}}]}\n\n",
        "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
        "data: [DONE]\n\n"
    )).await;
    let kernel = Kernel::open(
        ":memory:".into(),
        tempfile::tempdir().unwrap().keep(),
        Arc::new(|_| {}),
    )
    .await
    .unwrap();
    let session = kernel.create_session(None).await.unwrap();
    kernel
        .start_model(session.id, "do work".into(), Vec::new(), config(url))
        .await
        .unwrap();
    idle(&kernel).await;
    let runs = kernel.list_runs().await.unwrap();
    assert_eq!(runs[0].status, RunStatus::Completed, "{:?}", runs[0].error);
    assert!(runs[0].output.contains("模型本轮未产生可见输出"), "{}", runs[0].output);
    // 初始请求 + 两次追问续跑。
    assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 3);
    kernel.shutdown().await.unwrap();
    server.abort();
}

const LIST_DIR_CALL: &str = concat!(
    "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"function\":{\"name\":\"list_dir\",\"arguments\":\"{\\\"path\\\":\\\".\\\"}\"}}]}}]}\n\n",
    "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
    "data: [DONE]\n\n"
);

#[tokio::test]
async fn doom_loop_blocks_then_aborts_repeated_identical_calls() {
    // 模型每轮原样重发同一调用：1–2 次静默执行、第 3 次执行+警告、
    // 第 4–5 次被拦截（不执行）、第 6 次硬终止——真实执行恰好 3 次。
    let (url, attempts, server) = provider_repeating(LIST_DIR_CALL).await;
    let events: Arc<Mutex<Vec<KernelEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let captured = events.clone();
    let dir = tempfile::tempdir().unwrap().keep();
    let kernel = Kernel::open(
        ":memory:".into(),
        dir.clone(),
        Arc::new(move |event| captured.lock().unwrap().push(event)),
    )
    .await
    .unwrap();
    kernel.set_workspace_root(Some(dir)).await;
    let session = kernel.create_session(None).await.unwrap();
    kernel
        .start_model(session.id, "loop it".into(), Vec::new(), config(url))
        .await
        .unwrap();
    idle(&kernel).await;
    let runs = kernel.list_runs().await.unwrap();
    assert_eq!(runs[0].status, RunStatus::Completed, "{:?}", runs[0].error);
    assert!(runs[0].output.contains("检测到工具调用停滞"), "{}", runs[0].output);
    // 六轮请求：第 6 轮的 pre-pass 触发 abort。
    assert_eq!(attempts.load(std::sync::atomic::Ordering::SeqCst), 6);
    // 仅 streak 1–3 真实执行（streak 3 附警告）；4–6 被拦截无 ToolCall 事件。
    let list_dir_finished = events
        .lock()
        .unwrap()
        .iter()
        .filter(|event| matches!(&event.data, EventData::ToolCall { tool, phase: ToolPhase::Finished, .. } if tool == "list_dir"))
        .count();
    assert_eq!(list_dir_finished, 3);
    kernel.shutdown().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn alternating_call_arguments_reset_the_doom_streak() {
    // 字节不同的参数即不同签名：A,A,B,正文——无停滞标记，三个调用全执行。
    let (url, server) = provider_sequence(vec![
        LIST_DIR_CALL,
        LIST_DIR_CALL,
        concat!(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c1\",\"function\":{\"name\":\"list_dir\",\"arguments\":\"{\\\"path\\\": \\\".\\\"}\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n"
        ),
        "data: {\"choices\":[{\"delta\":{\"content\":\"done\"}}]}\n\ndata: [DONE]\n\n",
    ])
    .await;
    let events: Arc<Mutex<Vec<KernelEvent>>> = Arc::new(Mutex::new(Vec::new()));
    let captured = events.clone();
    let dir = tempfile::tempdir().unwrap().keep();
    let kernel = Kernel::open(
        ":memory:".into(),
        dir.clone(),
        Arc::new(move |event| captured.lock().unwrap().push(event)),
    )
    .await
    .unwrap();
    kernel.set_workspace_root(Some(dir)).await;
    let session = kernel.create_session(None).await.unwrap();
    kernel
        .start_model(session.id, "loop it".into(), Vec::new(), config(url))
        .await
        .unwrap();
    idle(&kernel).await;
    let runs = kernel.list_runs().await.unwrap();
    assert_eq!(runs[0].status, RunStatus::Completed, "{:?}", runs[0].error);
    assert!(!runs[0].output.contains("检测到工具调用停滞"), "{}", runs[0].output);
    assert!(runs[0].output.contains("done"), "{}", runs[0].output);
    let list_dir_finished = events
        .lock()
        .unwrap()
        .iter()
        .filter(|event| matches!(&event.data, EventData::ToolCall { tool, phase: ToolPhase::Finished, .. } if tool == "list_dir"))
        .count();
    assert_eq!(list_dir_finished, 3);
    kernel.shutdown().await.unwrap();
    server.abort();
}

#[tokio::test]
async fn reasoning_deltas_are_coalesced_into_one_chunk() {
    // 首轮纯 reasoning 零正文触发空轮追问；第二轮回正文使 run 收尾。
    let (url, server) = provider_sequence(vec![
        concat!(
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"think \"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"reasoning_content\":\"more\"}}]}\n\n",
            "data: [DONE]\n\n"
        ),
        "data: {\"choices\":[{\"delta\":{\"content\":\"done\"}}]}\n\ndata: [DONE]\n\n",
    ])
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
    kernel
        .start_model(session.id, "reason".into(), Vec::new(), config(url))
        .await
        .unwrap();
    idle(&kernel).await;
    let chunks: Vec<String> = events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match &event.data {
            EventData::Reasoning { token, .. } => Some(token.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(chunks, vec!["think more".to_string()]);
    server.abort();
    kernel.shutdown().await.unwrap();
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
        .start_model(session.id, "current user question".into(), Vec::new(), config(url))
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
        .contains("你处于计划模式"));
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
        .start_model(session.id, "auth failure".into(), Vec::new(), config(url))
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
        .start_model(session.id, "cancel request".into(), Vec::new(), config(url))
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
        .start_model(session.id, "anthropic request".into(), Vec::new(), settings)
        .await
        .unwrap();
    let body = tokio::time::timeout(Duration::from_secs(3), request)
        .await
        .unwrap()
        .unwrap();
    // The system prompt ships as text blocks carrying an ephemeral cache
    // breakpoint on the last block.
    let system_blocks = body["system"].as_array().unwrap();
    assert!(system_blocks
        .iter()
        .any(|block| block["text"].as_str().unwrap().contains("你处于计划模式")));
    assert_eq!(
        system_blocks.last().unwrap()["cache_control"]["type"],
        "ephemeral"
    );
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
        reasoning: Default::default(),
        parts: Default::default(),
        run_images: Default::default(),
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
        compactions: Vec::new(),
        run_messages: Vec::new(),
        tools: Vec::new(),
    };
    let messages: Vec<ChatMessage> = conversation(&detail, &current, None);
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
