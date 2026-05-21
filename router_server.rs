use actix_web::{web, App, HttpServer, HttpResponse, get, post};
use serde::{Deserialize, Serialize};
use std::env;
use std::fs;
use std::path::Path;

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
}

#[derive(Serialize, Debug)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Serialize, Debug)]
struct ChatCompletionRequest {
    model: String,
    messages: Vec<ChatMessage>,
    max_tokens: u32,
    temperature: f32,
    stream: bool,
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
}

#[derive(Deserialize, Debug)]
struct ChatUsage {
    prompt_tokens: u32,
    completion_tokens: u32,
}

// Anthropic Messages API response format
#[derive(Serialize)]
struct AnthropicResponse {
    id: String,
    #[serde(rename = "type")]
    msg_type: String,
    role: String,
    model: String,
    content: Vec<ContentBlock>,
    stop_reason: String,
    stop_sequence: Option<String>,
    usage: AnthropicUsage,
}

#[derive(Serialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    block_type: String,
    text: String,
}

#[derive(Serialize)]
struct AnthropicUsage {
    input_tokens: u32,
    output_tokens: u32,
}

#[derive(Serialize, Deserialize)]
struct HealthResponse {
    status: String,
    service: String,
}

fn get_api_key() -> String {
    env::var("REGOLO_API_KEY").unwrap_or_else(|_| {
        let auth_file = dirs::home_dir()
            .map(|h| h.join(".regolo").join("auth.json"))
            .unwrap_or_else(|| Path::new("~/.regolo/auth.json").to_path_buf());

        if auth_file.exists() {
            if let Ok(content) = fs::read_to_string(&auth_file) {
                if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(key) = json.get("api_key").and_then(|v| v.as_str()) {
                        return key.to_string();
                    }
                }
            }
        }
        String::new()
    })
}

fn extract_text_content(value: &serde_json::Value) -> String {
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
    let mut messages: Vec<ChatMessage> = Vec::new();

    // Prepend system prompt if present (Claude sends as top-level field)
    if let Some(system) = &req.system {
        let text = extract_text_content(system);
        if !text.is_empty() {
            messages.push(ChatMessage { role: "system".to_string(), content: text });
        }
    }

    for msg in &req.messages {
        messages.push(ChatMessage {
            role: msg.role.clone(),
            content: extract_text_content(&msg.content),
        });
    }

    messages
}

fn translate_chat_to_anthropic(resp: ChatCompletionResponse, model: &str) -> AnthropicResponse {
    let text = resp.choices.first()
        .map(|c| {
            c.message.content.clone()
                .or_else(|| c.message.reasoning_content.clone())
                .unwrap_or_default()
        })
        .unwrap_or_default();

    let usage = resp.usage.unwrap_or(ChatUsage { prompt_tokens: 0, completion_tokens: 0 });

    AnthropicResponse {
        id: format!("msg_{}", uuid::Uuid::new_v4().simple()),
        msg_type: "message".to_string(),
        role: "assistant".to_string(),
        model: resp.model.unwrap_or_else(|| model.to_string()),
        content: vec![ContentBlock { block_type: "text".to_string(), text }],
        stop_reason: "end_turn".to_string(),
        stop_sequence: None,
        usage: AnthropicUsage {
            input_tokens: usage.prompt_tokens,
            output_tokens: usage.completion_tokens,
        },
    }
}

#[post("/v1/messages")]
async fn handle_messages(req: web::Json<MessagesRequest>) -> HttpResponse {
    let api_key = get_api_key();

    if api_key.is_empty() {
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({"error": "REGOLO_API_KEY not found"}));
    }

    let chat_req = ChatCompletionRequest {
        model: req.model.clone(),
        messages: build_chat_messages(&req),
        max_tokens: req.max_tokens.unwrap_or(4096),
        temperature: req.temperature.unwrap_or(0.7),
        stream: false,
    };

    let client = reqwest::Client::new();

    match client
        .post("https://api.regolo.ai/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&chat_req)
        .send()
        .await
    {
        Ok(response) => {
            let status = response.status();
            if status.is_success() {
                match response.json::<ChatCompletionResponse>().await {
                    Ok(chat_resp) => {
                        let anthropic_resp = translate_chat_to_anthropic(chat_resp, &req.model);
                        HttpResponse::Ok().json(anthropic_resp)
                    }
                    Err(e) => HttpResponse::InternalServerError()
                        .json(serde_json::json!({"error": format!("Failed to parse response: {}", e)})),
                }
            } else {
                let body = response.text().await.unwrap_or_default();
                HttpResponse::build(status)
                    .json(serde_json::json!({"error": format!("Regolo API error: {}", status.as_u16()), "details": body}))
            }
        }
        Err(e) => HttpResponse::InternalServerError()
            .json(serde_json::json!({"error": format!("Proxy error: {}", e)})),
    }
}

#[get("/v1/models")]
async fn handle_models() -> HttpResponse {
    let api_key = get_api_key();

    if api_key.is_empty() {
        return HttpResponse::Unauthorized()
            .json(serde_json::json!({"error": "REGOLO_API_KEY not found"}));
    }

    let client = reqwest::Client::new();

    match client
        .get("https://api.regolo.ai/models")
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
    {
        Ok(response) => {
            let status = response.status();
            match response.text().await {
                Ok(body) => HttpResponse::build(status).body(body),
                Err(e) => HttpResponse::InternalServerError()
                    .json(serde_json::json!({"error": format!("Failed to read response: {}", e)})),
            }
        }
        Err(e) => HttpResponse::InternalServerError()
            .json(serde_json::json!({"error": format!("Failed to fetch models: {}", e)})),
    }
}

#[get("/health")]
async fn health() -> HttpResponse {
    HttpResponse::Ok().json(HealthResponse {
        status: "ok".to_string(),
        service: "regolo-messages-proxy".to_string(),
    })
}

#[get("/")]
async fn root() -> HttpResponse {
    health().await
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    let port = env::var("PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(0);

    let server = HttpServer::new(|| {
        App::new()
            .service(handle_messages)
            .service(handle_models)
            .service(health)
            .service(root)
    })
    .bind(format!("0.0.0.0:{}", port))?
    .run();

    let actual_port = server.addrs()[0].port();
    println!("Starting Regolo Messages Proxy on port {}", actual_port);
    println!("Regolo API: https://api.regolo.ai");
    println!("\nEndpoints:");
    println!("  POST /v1/messages  - Translated to /v1/chat/completions");
    println!("  GET  /v1/models    - Forwarded to Regolo");
    println!("  GET  /health       - Health check");
    println!("\nTo use with Claude:");
    println!("  ANTHROPIC_BASE_URL=http://localhost:{} ANTHROPIC_API_KEY=your_key claude", actual_port);
    println!();

    server.await
}
