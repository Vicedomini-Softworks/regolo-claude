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
    tool_calls: Option<Vec<OpenAiToolCall>>,
}

#[derive(Serialize, Debug, Clone)]
struct OpenAiToolCall {
    id: String,
    #[serde(rename = "type")]
    call_type: String,
    function: OpenAiFunction,
}

#[derive(Serialize, Debug, Clone)]
struct OpenAiFunction {
    name: String,
    arguments: String,
}

#[derive(Serialize, Debug)]
struct OpenAiTool {
    #[serde(rename = "type")]
    tool_type: String,
    function: OpenAiFunctionDef,
}

#[derive(Serialize, Debug)]
struct OpenAiFunctionDef {
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
    tools: Vec<OpenAiTool>,
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

// Anthropic Messages API response format
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

    if let Some(system) = &req.system {
        let text = extract_text_content(system);
        if !text.is_empty() {
            messages.push(ChatMessage {
                role: "system".to_string(),
                content: Some(text),
                tool_call_id: None,
                tool_calls: None,
            });
        }
    }

    for msg in &req.messages {
        if let Some(arr) = msg.content.as_array() {
            // Check for tool_result blocks → emit as tool role messages
            let tool_results: Vec<&serde_json::Value> = arr
                .iter()
                .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("tool_result"))
                .collect();
            let other_blocks: Vec<&serde_json::Value> = arr
                .iter()
                .filter(|b| b.get("type").and_then(|t| t.as_str()) != Some("tool_result"))
                .collect();

            if !other_blocks.is_empty() {
                let mut text_parts: Vec<String> = Vec::new();
                let mut tool_calls: Vec<OpenAiToolCall> = Vec::new();
                for block in &other_blocks {
                    match block.get("type").and_then(|t| t.as_str()) {
                        Some("text") => {
                            if let Some(t) = block.get("text").and_then(|v| v.as_str()) {
                                text_parts.push(t.to_string());
                            }
                        }
                        Some("tool_use") => {
                            let id = block.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                            let input = block.get("input").cloned().unwrap_or(serde_json::json!({}));
                            tool_calls.push(OpenAiToolCall {
                                id,
                                call_type: "function".to_string(),
                                function: OpenAiFunction {
                                    name,
                                    arguments: input.to_string(),
                                },
                            });
                        }
                        _ => {}
                    }
                }
                let content_str = text_parts.join("\n");
                messages.push(ChatMessage {
                    role: msg.role.clone(),
                    content: if content_str.is_empty() { None } else { Some(content_str) },
                    tool_call_id: None,
                    tool_calls: if tool_calls.is_empty() { None } else { Some(tool_calls) },
                });
            }

            for tr in tool_results {
                let tool_call_id = tr.get("tool_use_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let content_val = tr.get("content");
                let content_str = match content_val {
                    Some(serde_json::Value::String(s)) => s.clone(),
                    Some(v) => extract_text_content(v),
                    None => String::new(),
                };
                messages.push(ChatMessage {
                    role: "tool".to_string(),
                    content: Some(content_str),
                    tool_call_id: Some(tool_call_id),
                    tool_calls: None,
                });
            }
        } else {
            messages.push(ChatMessage {
                role: msg.role.clone(),
                content: Some(extract_text_content(&msg.content)),
                tool_call_id: None,
                tool_calls: None,
            });
        }
    }

    messages
}

fn convert_tools(tools: &[serde_json::Value]) -> Vec<OpenAiTool> {
    tools.iter().map(|t| OpenAiTool {
        tool_type: "function".to_string(),
        function: OpenAiFunctionDef {
            name: t.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            description: t.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            parameters: t.get("input_schema").cloned()
                .unwrap_or_else(|| serde_json::json!({"type": "object", "properties": {}})),
        },
    }).collect()
}

fn translate_chat_to_anthropic(resp: ChatCompletionResponse, model: &str) -> AnthropicResponse {
    let mut content_blocks: Vec<serde_json::Value> = Vec::new();
    let mut stop_reason = "end_turn".to_string();

    if let Some(choice) = resp.choices.first() {
        let finish_reason = choice.finish_reason.as_deref().unwrap_or("stop");

        let text = choice.message.content.clone()
            .or_else(|| choice.message.reasoning_content.clone())
            .unwrap_or_default();
        if !text.is_empty() {
            content_blocks.push(serde_json::json!({"type": "text", "text": text}));
        }

        if !choice.message.tool_calls.is_empty() {
            stop_reason = "tool_use".to_string();
            for tc in &choice.message.tool_calls {
                let input: serde_json::Value = serde_json::from_str(&tc.function.arguments)
                    .unwrap_or(serde_json::json!({}));
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

    let usage = resp.usage.unwrap_or(ChatUsage { prompt_tokens: 0, completion_tokens: 0 });

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
        tools: convert_tools(&req.tools),
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
