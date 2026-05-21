use actix_web::{get, post, web, App, HttpResponse, HttpServer};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::{env, fs, path::PathBuf, process, sync::mpsc, thread, time::Instant};
use tracing::{error, info, warn};

const REGOLO_API_BASE: &str = "https://api.regolo.ai";

// ── Auth ──────────────────────────────────────────────────────────────────────

fn auth_file() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".regolo")
        .join("auth.json")
}

fn load_api_key() -> Option<String> {
    if let Ok(k) = env::var("REGOLO_API_KEY") {
        if !k.is_empty() {
            return Some(k);
        }
    }
    let path = auth_file();
    if path.exists() {
        if let Ok(content) = fs::read_to_string(&path) {
            if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(k) = json.get("api_key").and_then(|v| v.as_str()) {
                    return Some(k.to_string());
                }
            }
        }
    }
    None
}

fn save_api_key(key: &str) -> Result<(), Box<dyn std::error::Error>> {
    let path = auth_file();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(parent, fs::Permissions::from_mode(0o700))?;
        }
    }
    fs::write(&path, serde_json::json!({"api_key": key}).to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn delete_api_key() {
    let _ = fs::remove_file(auth_file());
}

fn require_api_key() -> String {
    match load_api_key() {
        Some(k) => k,
        None => {
            eprintln!("Error: REGOLO_API_KEY not found.");
            eprintln!("Run 'regolo login' to set your API key.");
            process::exit(1);
        }
    }
}

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(
    name = "regolo",
    about = "Launch Claude Code with Regolo.ai models",
    after_help = "Examples:\n  regolo login\n  regolo claude --model brick-v1-beta\n  regolo list\n  regolo proxy\n  regolo logout"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Login and store API key securely in ~/.regolo/auth.json
    Login,
    /// Logout and remove stored API key
    Logout,
    /// List available Regolo models
    List,
    /// Launch Claude Code with a Regolo model
    Claude {
        #[arg(short, long, default_value = "brick-v1-beta", help = "Model to use")]
        model: String,
    },
    /// Start the messages proxy server in foreground
    Proxy {
        #[arg(short, long, default_value = "0", help = "Port (0 = random)")]
        port: u16,
    },
}

// ── Proxy types ───────────────────────────────────────────────────────────────

#[derive(Deserialize, Debug)]
struct Message {
    role: String,
    content: serde_json::Value,
}

#[derive(Deserialize, Debug)]
struct MessagesRequest {
    model: String,
    messages: Vec<Message>,
    #[serde(default)]
    system: Option<serde_json::Value>,
    #[serde(default)]
    max_tokens: Option<u32>,
    #[serde(default)]
    temperature: Option<f32>,
    #[serde(default)]
    tools: Vec<serde_json::Value>,
}

#[derive(Serialize, Debug)]
struct ChatMessage {
    role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<OaiToolCall>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct OaiToolCall {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: OaiFunction,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct OaiFunction {
    name: String,
    arguments: String,
}

#[derive(Serialize, Debug)]
struct OaiTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: OaiFunctionDef,
}

#[derive(Serialize, Debug)]
struct OaiFunctionDef {
    name: String,
    description: String,
    parameters: serde_json::Value,
}

#[derive(Serialize, Debug)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    max_tokens: u32,
    temperature: f32,
    stream: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<OaiTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_choice: Option<String>,
}

#[derive(Deserialize, Debug)]
struct ChatCompletionResponse {
    choices: Vec<ChatChoice>,
    model: Option<String>,
    usage: Option<ChatUsage>,
}

#[derive(Deserialize, Debug)]
struct ChatChoice {
    message: ChatChoiceMessage,
    finish_reason: Option<String>,
}

#[derive(Deserialize, Debug)]
struct ChatChoiceMessage {
    content: Option<String>,
    reasoning_content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<ResponseToolCall>,
}

#[derive(Deserialize, Debug)]
struct ResponseToolCall {
    id: String,
    function: ResponseFunction,
}

#[derive(Deserialize, Debug)]
struct ResponseFunction {
    name: String,
    arguments: String,
}

#[derive(Deserialize, Debug)]
struct ChatUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
}

#[derive(Serialize)]
struct AnthropicResponse {
    id: String,
    #[serde(rename = "type")]
    msg_type: String,
    role: String,
    model: String,
    content: Vec<serde_json::Value>,
    stop_reason: String,
    stop_sequence: Option<String>,
    usage: AnthropicUsage,
}

#[derive(Serialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
}

// ── Shared app state ──────────────────────────────────────────────────────────

struct AppState {
    client: reqwest::Client,
    api_key: String,
}

// ── Proxy logic ───────────────────────────────────────────────────────────────

fn extract_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|item| item.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => value.to_string(),
    }
}

fn build_chat_messages(req: &MessagesRequest) -> Vec<ChatMessage> {
    let mut out: Vec<ChatMessage> = Vec::new();

    // System prompt as first message
    if let Some(system) = &req.system {
        let text = extract_text(system);
        if !text.is_empty() {
            out.push(ChatMessage {
                role: "system".to_string(),
                content: Some(text),
                tool_call_id: None,
                tool_calls: None,
            });
        }
    }

    for msg in &req.messages {
        if let Some(arr) = msg.content.as_array() {
            let tool_results: Vec<&serde_json::Value> = arr
                .iter()
                .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_result"))
                .collect();
            let other: Vec<&serde_json::Value> = arr
                .iter()
                .filter(|b| b.get("type").and_then(|t| t.as_str()) != Some("tool_result"))
                .collect();

            if !other.is_empty() {
                let mut text_parts: Vec<String> = Vec::new();
                let mut tool_calls: Vec<OaiToolCall> = Vec::new();
                for block in &other {
                    match block.get("type").and_then(|t| t.as_str()) {
                        Some("text") => {
                            if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                                text_parts.push(t.to_string());
                            }
                        }
                        Some("tool_use") => {
                            let id = block
                                .get("id")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let name = block
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let input =
                                block.get("input").cloned().unwrap_or(serde_json::json!({}));
                            tool_calls.push(OaiToolCall {
                                id,
                                call_type: "function".to_string(),
                                function: OaiFunction {
                                    name,
                                    arguments: input.to_string(),
                                },
                            });
                        }
                        _ => {}
                    }
                }
                let content_str = text_parts.join("\n");
                out.push(ChatMessage {
                    role: msg.role.clone(),
                    content: if content_str.is_empty() {
                        None
                    } else {
                        Some(content_str)
                    },
                    tool_call_id: None,
                    tool_calls: if tool_calls.is_empty() {
                        None
                    } else {
                        Some(tool_calls)
                    },
                });
            }

            for tr in tool_results {
                let tool_call_id = tr
                    .get("tool_use_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let content_str = match tr.get("content") {
                    Some(serde_json::Value::String(s)) => s.clone(),
                    Some(v) => extract_text(v),
                    None => String::new(),
                };
                out.push(ChatMessage {
                    role: "tool".to_string(),
                    content: Some(content_str),
                    tool_call_id: Some(tool_call_id),
                    tool_calls: None,
                });
            }
        } else {
            out.push(ChatMessage {
                role: msg.role.clone(),
                content: Some(extract_text(&msg.content)),
                tool_call_id: None,
                tool_calls: None,
            });
        }
    }

    out
}

fn convert_tools(tools: &[serde_json::Value]) -> Vec<OaiTool> {
    tools
        .iter()
        .map(|t| OaiTool {
            tool_type: "function".to_string(),
            function: OaiFunctionDef {
                name: t
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                description: t
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
                parameters: t
                    .get("input_schema")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({"type": "object", "properties": {}})),
            },
        })
        .collect()
}

fn translate_to_anthropic(resp: ChatCompletionResponse, model: &str) -> AnthropicResponse {
    let mut content_blocks: Vec<serde_json::Value> = Vec::new();
    let mut stop_reason = "end_turn".to_string();

    if let Some(choice) = resp.choices.first() {
        let finish_reason = choice.finish_reason.as_deref().unwrap_or("stop");

        let text = choice
            .message
            .content
            .clone()
            .or_else(|| choice.message.reasoning_content.clone())
            .unwrap_or_default();
        if !text.is_empty() {
            content_blocks.push(serde_json::json!({"type": "text", "text": text}));
        }

        if !choice.message.tool_calls.is_empty() {
            stop_reason = "tool_use".to_string();
            for tc in &choice.message.tool_calls {
                let input: serde_json::Value =
                    serde_json::from_str(&tc.function.arguments).unwrap_or(serde_json::json!({}));
                content_blocks.push(serde_json::json!({
                    "type": "tool_use",
                    "id": tc.id,
                    "name": tc.function.name,
                    "input": input,
                }));
            }
        } else if finish_reason == "length" {
            stop_reason = "max_tokens".to_string();
        }
    }

    if content_blocks.is_empty() {
        content_blocks.push(serde_json::json!({"type": "text", "text": ""}));
    }

    let usage = resp.usage.unwrap_or(ChatUsage {
        prompt_tokens: 0,
        completion_tokens: 0,
    });

    AnthropicResponse {
        id: format!("msg_{}", uuid::Uuid::new_v4().simple()),
        msg_type: "message".to_string(),
        role: "assistant".to_string(),
        model: resp.model.unwrap_or_else(|| model.to_string()),
        content: content_blocks,
        stop_reason,
        stop_sequence: None,
        usage: AnthropicUsage {
            input_tokens: usage.prompt_tokens,
            output_tokens: usage.completion_tokens,
        },
    }
}

// ── Actix handlers ────────────────────────────────────────────────────────────

#[post("/v1/messages")]
async fn handle_messages(
    state: web::Data<AppState>,
    req: web::Json<MessagesRequest>,
) -> HttpResponse {
    let request_start = Instant::now();
    let model = req.model.clone();
    let msg_count = req.messages.len();
    let tool_count = req.tools.len();

    info!(model = %model, messages = msg_count, tools = tool_count, "→ /v1/messages");

    let t_build = Instant::now();
    let has_tools = !req.tools.is_empty();
    let chat_req = ChatCompletionRequest {
        model: req.model.clone(),
        messages: build_chat_messages(&req),
        max_tokens: req.max_tokens.unwrap_or(4096),
        temperature: req.temperature.unwrap_or(0.7),
        stream: false,
        tools: convert_tools(&req.tools),
        tool_choice: if has_tools {
            Some("auto".to_string())
        } else {
            None
        },
    };
    info!(
        elapsed_ms = t_build.elapsed().as_millis(),
        "build_chat_messages"
    );

    let t_upstream = Instant::now();
    match state
        .client
        .post(format!("{}/v1/chat/completions", REGOLO_API_BASE))
        .header("Authorization", format!("Bearer {}", state.api_key))
        .json(&chat_req)
        .send()
        .await
    {
        Ok(response) => {
            let upstream_ms = t_upstream.elapsed().as_millis();
            let status = response.status();
            info!(
                status = status.as_u16(),
                upstream_ms, "← upstream responded"
            );

            if status.is_success() {
                let t_parse = Instant::now();
                match response.json::<ChatCompletionResponse>().await {
                    Ok(chat_resp) => {
                        info!(elapsed_ms = t_parse.elapsed().as_millis(), "parse response");
                        let anthropic_resp = translate_to_anthropic(chat_resp, &model);
                        info!(
                            total_ms = request_start.elapsed().as_millis(),
                            stop_reason = %anthropic_resp.stop_reason,
                            "✓ request complete"
                        );
                        HttpResponse::Ok().json(anthropic_resp)
                    }
                    Err(e) => {
                        error!(error = %e, "failed to parse upstream response");
                        HttpResponse::InternalServerError().json(
                            serde_json::json!({"error": format!("Failed to parse response: {}", e)}),
                        )
                    }
                }
            } else {
                let body = response.text().await.unwrap_or_default();
                warn!(status = status.as_u16(), body = %body, "upstream error");
                HttpResponse::build(status).json(serde_json::json!({
                    "error": format!("Regolo API error: {}", status.as_u16()),
                    "details": body
                }))
            }
        }
        Err(e) => {
            error!(error = %e, elapsed_ms = t_upstream.elapsed().as_millis(), "upstream request failed");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": format!("Proxy error: {}", e)}))
        }
    }
}

#[get("/v1/models")]
async fn handle_models(state: web::Data<AppState>) -> HttpResponse {
    let t = Instant::now();
    match state
        .client
        .get(format!("{}/models", REGOLO_API_BASE))
        .header("Authorization", format!("Bearer {}", state.api_key))
        .send()
        .await
    {
        Ok(response) => {
            let status = response.status();
            info!(
                status = status.as_u16(),
                elapsed_ms = t.elapsed().as_millis(),
                "GET /models"
            );
            match response.text().await {
                Ok(body) => HttpResponse::build(status).body(body),
                Err(e) => HttpResponse::InternalServerError()
                    .json(serde_json::json!({"error": format!("Failed to read response: {}", e)})),
            }
        }
        Err(e) => {
            error!(error = %e, "GET /models failed");
            HttpResponse::InternalServerError()
                .json(serde_json::json!({"error": format!("Failed to fetch models: {}", e)}))
        }
    }
}

#[get("/health")]
async fn handle_health() -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({"status": "ok", "service": "regolo-messages-proxy"}))
}

#[get("/")]
async fn handle_root() -> HttpResponse {
    HttpResponse::Ok().json(serde_json::json!({"status": "ok", "service": "regolo-messages-proxy"}))
}

// ── Proxy server runner ───────────────────────────────────────────────────────

fn make_app_state(api_key: String) -> web::Data<AppState> {
    let client = reqwest::Client::builder()
        .pool_max_idle_per_host(32)
        .tcp_keepalive(std::time::Duration::from_secs(90))
        .build()
        .expect("failed to build reqwest client");
    web::Data::new(AppState { client, api_key })
}

fn run_proxy(port: u16) -> std::io::Result<()> {
    let api_key = match load_api_key() {
        Some(k) => k,
        None => {
            eprintln!("Error: REGOLO_API_KEY not found. Run 'regolo login' first.");
            std::process::exit(1);
        }
    };

    actix_web::rt::System::new().block_on(async move {
        let state = make_app_state(api_key);
        let bound = HttpServer::new(move || {
            App::new()
                .app_data(state.clone())
                .service(handle_messages)
                .service(handle_models)
                .service(handle_health)
                .service(handle_root)
        })
        .bind(format!("0.0.0.0:{}", port))?;

        let actual_port = bound.addrs()[0].port();
        println!("Starting Regolo Messages Proxy on port {}", actual_port);
        println!("Regolo API: {}", REGOLO_API_BASE);
        println!("\nEndpoints:");
        println!("  POST /v1/messages  -> /v1/chat/completions");
        println!("  GET  /v1/models    -> Regolo");
        println!("  GET  /health");
        println!("\nTo use with Claude:");
        println!(
            "  ANTHROPIC_BASE_URL=http://localhost:{} ANTHROPIC_API_KEY=<key> claude",
            actual_port
        );
        println!("  (set RUST_LOG=info for timing logs)");
        println!();

        bound.run().await
    })
}

/// Spawn proxy in a background thread and return the port it bound to.
fn spawn_proxy(port: u16, api_key: String) -> u16 {
    let (tx, rx) = mpsc::channel::<u16>();

    thread::spawn(move || {
        actix_web::rt::System::new().block_on(async move {
            let state = make_app_state(api_key);
            let bound = HttpServer::new(move || {
                App::new()
                    .app_data(state.clone())
                    .service(handle_messages)
                    .service(handle_models)
                    .service(handle_health)
                    .service(handle_root)
            })
            .bind(format!("0.0.0.0:{}", port))
            .expect("Failed to bind proxy port");

            let actual_port = bound.addrs()[0].port();
            tx.send(actual_port).ok();
            bound.run().await.ok();
        });
    });

    rx.recv().expect("Proxy failed to start")
}

// ── Commands ──────────────────────────────────────────────────────────────────

fn cmd_login() {
    println!("Regolo.ai API Login");
    println!("{}", "-".repeat(40));

    let key = rpassword::prompt_password("Enter your Regolo API key: ").unwrap_or_else(|_| {
        eprint!("Enter your Regolo API key: ");
        let mut buf = String::new();
        std::io::stdin().read_line(&mut buf).ok();
        buf
    });
    let key = key.trim().to_string();

    if key.is_empty() {
        eprintln!("Error: API key cannot be empty.");
        process::exit(1);
    }

    match save_api_key(&key) {
        Ok(_) => {
            println!("\n✓ API key saved to: {}", auth_file().display());
        }
        Err(e) => {
            eprintln!("Error saving API key: {}", e);
            eprintln!("Set it via: export REGOLO_API_KEY=<key>");
        }
    }
}

fn cmd_logout() {
    delete_api_key();
    println!("✓ API key removed. You are now logged out.");
}

fn cmd_list() {
    let api_key = require_api_key();

    let rt = tokio::runtime::Runtime::new().expect("Failed to create runtime");
    rt.block_on(async move {
        let client = reqwest::Client::new();
        match client
            .get(format!("{}/models", REGOLO_API_BASE))
            .header("Authorization", format!("Bearer {}", api_key))
            .send()
            .await
        {
            Ok(resp) => match resp.json::<serde_json::Value>().await {
                Ok(data) => {
                    println!("Available Regolo models:");
                    if let Some(models) = data.get("data").and_then(|d| d.as_array()) {
                        for model in models {
                            if let Some(id) = model.get("id").and_then(|v| v.as_str()) {
                                println!("  {}", id);
                            }
                        }
                    }
                }
                Err(e) => eprintln!("Failed to parse models response: {}", e),
            },
            Err(e) => eprintln!("Failed to fetch models: {}", e),
        }
    });
}

fn cmd_claude(model: &str) {
    let api_key = require_api_key();

    println!("Starting proxy server...");
    let port = spawn_proxy(0, api_key.clone());
    println!("Proxy ready at http://localhost:{}", port);
    println!("Launching Claude Code with model: {}", model);
    println!("{}", "-".repeat(50));

    let status = process::Command::new("claude")
        .env("ANTHROPIC_API_KEY", &api_key)
        .env("ANTHROPIC_MODEL", model)
        .env("ANTHROPIC_BASE_URL", format!("http://localhost:{}", port))
        .status();

    match status {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            eprintln!("Error: 'claude' command not found.");
            eprintln!("Install: npm install -g @anthropic-ai/claude-code");
            process::exit(1);
        }
        Err(e) => {
            eprintln!("Error launching Claude: {}", e);
            process::exit(1);
        }
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();

    let cli = Cli::parse();
    match cli.command {
        Commands::Login => cmd_login(),
        Commands::Logout => cmd_logout(),
        Commands::List => cmd_list(),
        Commands::Claude { model } => cmd_claude(&model),
        Commands::Proxy { port } => {
            if let Err(e) = run_proxy(port) {
                eprintln!("Proxy error: {}", e);
                process::exit(1);
            }
        }
    }
}
