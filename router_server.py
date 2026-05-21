#!/usr/bin/env python3
"""
Proxy server that translates Claude's /v1/messages API to Regolo's /v1/completion API.

This allows Claude Code (which expects /v1/messages) to communicate with Regolo.ai
(which only supports /v1/completion).

Start the server:
    python router_server.py

Then launch Claude with:
    ANTHROPIC_BASE_URL=http://localhost:8789 ANTHROPIC_API_KEY=your_key claude
"""

import asyncio
import json
import os
import sys
from http import HTTPStatus
from typing import Any

try:
    from aiohttp import web
except ImportError:
    print("Error: aiohttp is required. Install with: pip install aiohttp", file=sys.stderr)
    sys.exit(1)

REGOLO_BASE_URL = "https://api.regolo.ai"
REGOLO_API_KEY = os.environ.get("REGOLO_API_KEY", "")
AUTH_FILE = os.path.expanduser("~/.regolo/auth.json")
DEBUG = os.environ.get("DEBUG", "").lower() in ("true", "1", "yes")


def get_api_key_from_storage():
    """Retrieve API key from auth.json file."""
    try:
        if os.path.exists(AUTH_FILE):
            with open(AUTH_FILE, 'r') as f:
                data = json.load(f)
                return data.get('api_key')
    except (json.JSONDecodeError, IOError):
        pass
    return None


def check_api_key():
    """Check if REGOLO_API_KEY is set."""
    if REGOLO_API_KEY:
        return REGOLO_API_KEY
    stored_key = get_api_key_from_storage()
    if stored_key:
        return stored_key
    print("Error: REGOLO_API_KEY not found.", file=sys.stderr)
    print("Set it via: export REGOLO_API_KEY=your_key", file=sys.stderr)
    sys.exit(1)


def convert_anthropic_tools_to_openai(tools: list) -> list:
    """Convert Anthropic tool definitions to OpenAI function format."""
    result = []
    for tool in tools:
        result.append({
            "type": "function",
            "function": {
                "name": tool.get("name", ""),
                "description": tool.get("description", ""),
                "parameters": tool.get("input_schema", {"type": "object", "properties": {}}),
            }
        })
    return result


def convert_messages_to_openai(messages: list) -> list:
    """
    Convert Anthropic message format to OpenAI format.
    Handles tool_result content blocks → tool role messages.
    Handles content arrays → string content.
    """
    result = []
    for msg in messages:
        role = msg.get("role", "user")
        content = msg.get("content", "")

        if isinstance(content, list):
            tool_results = [b for b in content if isinstance(b, dict) and b.get("type") == "tool_result"]
            other_blocks = [b for b in content if not (isinstance(b, dict) and b.get("type") == "tool_result")]

            # Emit assistant message for any text/tool_use blocks first
            if other_blocks:
                text_parts = []
                tool_calls = []
                for block in other_blocks:
                    if isinstance(block, dict):
                        if block.get("type") == "text":
                            text_parts.append(block.get("text", ""))
                        elif block.get("type") == "tool_use":
                            tool_calls.append({
                                "id": block.get("id", f"call_{os.urandom(4).hex()}"),
                                "type": "function",
                                "function": {
                                    "name": block.get("name", ""),
                                    "arguments": json.dumps(block.get("input", {})),
                                }
                            })
                        else:
                            text_parts.append(str(block))
                    else:
                        text_parts.append(str(block))
                msg_out = {"role": role, "content": "\n".join(text_parts) or None}
                if tool_calls:
                    msg_out["tool_calls"] = tool_calls
                result.append(msg_out)

            # Emit tool result messages
            for tr in tool_results:
                tr_content = tr.get("content", "")
                if isinstance(tr_content, list):
                    tr_content = "\n".join(
                        b.get("text", str(b)) if isinstance(b, dict) else str(b)
                        for b in tr_content
                    )
                result.append({
                    "role": "tool",
                    "tool_call_id": tr.get("tool_use_id", ""),
                    "content": str(tr_content) if tr_content is not None else "",
                })
        else:
            result.append({"role": role, "content": content})

    return result


def translate_messages_to_completion(messages: list[dict], model: str, extra_params: dict = None, tools: list = None) -> dict:
    """
    Translate Claude's /v1/messages format to Regolo's /v1/chat/completions format.
    """
    result = {
        "model": model,
        "messages": convert_messages_to_openai(messages),
        "max_tokens": extra_params.get("max_tokens", 4096) if extra_params else 4096,
        "temperature": extra_params.get("temperature", 0.7) if extra_params else 0.7,
        "stream": False,
    }

    if tools:
        result["tools"] = convert_anthropic_tools_to_openai(tools)
        result["tool_choice"] = "auto"

    if extra_params:
        for key in ["reasoning_effort", "top_p", "stop", "seed"]:
            if key in extra_params:
                result[key] = extra_params[key]

    return result


def translate_completion_to_messages(completion_response: dict, model: str = "") -> dict:
    """
    Translate Regolo's chat completion response to Anthropic Messages API format.
    Handles both text responses and tool_calls.
    """
    content_blocks = []
    stop_reason = "end_turn"

    if "choices" in completion_response and completion_response["choices"]:
        choice = completion_response["choices"][0]
        msg = choice.get("message", {})
        finish_reason = choice.get("finish_reason", "stop")

        text = msg.get("content") or msg.get("reasoning_content") or ""
        if text and isinstance(text, str):
            content_blocks.append({"type": "text", "text": text})

        tool_calls = msg.get("tool_calls", [])
        if tool_calls:
            stop_reason = "tool_use"
            for tc in tool_calls:
                fn = tc.get("function", {})
                try:
                    input_data = json.loads(fn.get("arguments", "{}"))
                except (json.JSONDecodeError, TypeError):
                    input_data = {}
                content_blocks.append({
                    "type": "tool_use",
                    "id": tc.get("id", f"toolu_{os.urandom(4).hex()}"),
                    "name": fn.get("name", ""),
                    "input": input_data,
                })
        elif finish_reason == "length":
            stop_reason = "max_tokens"

    if not content_blocks:
        content_blocks.append({"type": "text", "text": ""})

    usage = completion_response.get("usage", {})
    return {
        "id": f"msg_{os.urandom(8).hex()}",
        "type": "message",
        "role": "assistant",
        "model": completion_response.get("model", model or "unknown"),
        "content": content_blocks,
        "stop_reason": stop_reason,
        "stop_sequence": None,
        "usage": {
            "input_tokens": usage.get("prompt_tokens", 0),
            "output_tokens": usage.get("completion_tokens", 0),
        },
    }


async def handle_messages(request: web.Request) -> web.Response:
    """Handle /v1/messages endpoint by translating to /v1/chat/completions."""
    api_key = check_api_key()
    
    try:
        data = await request.json()
    except json.JSONDecodeError:
        return web.json_response(
            {"error": "Invalid JSON"}, 
            status=HTTPStatus.BAD_REQUEST
        )
    
    if DEBUG:
        print(f"\n{'='*60}", flush=True)
        print(f"[DEBUG] Received from Claude:", flush=True)
        print(f"[DEBUG] {json.dumps(data, indent=2)}", flush=True)
        print(f"{'='*60}\n", flush=True)
    
    model = data.get("model", "brick-v1-beta")
    messages = data.get("messages", [])

    # Prepend system prompt if present (Claude sends it as top-level field)
    system = data.get("system", "")
    if system:
        if isinstance(system, list):
            system = " ".join(b.get("text", "") for b in system if isinstance(b, dict))
        messages = [{"role": "system", "content": system}] + messages

    # Extract extra parameters that Claude sends
    extra_params = {}
    for key in ["reasoning_effort", "top_p", "stop", "seed", "max_tokens", "temperature"]:
        if key in data:
            extra_params[key] = data[key]

    tools = data.get("tools", [])

    if DEBUG:
        print(f"[DEBUG] Extra params: {extra_params}", flush=True)
        print(f"[DEBUG] Tools: {len(tools)} defined", flush=True)

    if not messages:
        return web.json_response(
            {"error": "No messages provided"},
            status=HTTPStatus.BAD_REQUEST
        )

    completion_data = translate_messages_to_completion(messages, model, extra_params, tools)
    
    if DEBUG:
        print(f"[DEBUG] Sending to Regolo:", flush=True)
        print(f"[DEBUG] {json.dumps(completion_data, indent=2)}", flush=True)
    
    try:
        import aiohttp
        import ssl
        
        try:
            import certifi
            ssl_context = ssl.create_default_context(cafile=certifi.where())
        except:
            ssl_context = ssl.create_default_context()
            ssl_context.check_hostname = False
            ssl_context.verify_mode = ssl.CERT_NONE
        
        connector = aiohttp.TCPConnector(ssl=ssl_context)
        
        async with aiohttp.ClientSession(connector=connector) as session:
            async with session.post(
                f"{REGOLO_BASE_URL}/v1/chat/completions",
                json=completion_data,
                headers={"Authorization": f"Bearer {api_key}"},
                timeout=aiohttp.ClientTimeout(total=120)
            ) as resp:
                response_text = await resp.text()
                
                if DEBUG:
                    print(f"[DEBUG] Regolo response status: {resp.status}", flush=True)
                    try:
                        response_json = json.loads(response_text)
                        print(f"[DEBUG] Regolo response: {json.dumps(response_json, indent=2)}", flush=True)
                    except:
                        print(f"[DEBUG] Regolo response (raw): {response_text[:1000]}", flush=True)
                    print(f"{'='*60}\n", flush=True)
                
                if resp.status >= 400:
                    return web.json_response(
                        {"error": f"Regolo API error: {resp.status}", "details": response_text},
                        status=resp.status
                    )
                
                completion_response = json.loads(response_text)
                messages_response = translate_completion_to_messages(completion_response, model)
                
                if DEBUG:
                    print(f"[DEBUG] Response to Claude:", flush=True)
                    print(f"[DEBUG] {json.dumps(messages_response, indent=2)[:2000]}", flush=True)
                    print(f"{'='*60}\n", flush=True)
                
                return web.json_response(messages_response)
                
    except asyncio.TimeoutError:
        return web.json_response(
            {"error": "Regolo API timeout"},
            status=HTTPStatus.GATEWAY_TIMEOUT
        )
    except Exception as e:
        if DEBUG:
            print(f"[DEBUG] Proxy error: {str(e)}", flush=True)
            import traceback
            traceback.print_exc()
        return web.json_response(
            {"error": f"Proxy error: {str(e)}"},
            status=HTTPStatus.INTERNAL_SERVER_ERROR
        )


async def handle_models(request: web.Request) -> web.Response:
    """Handle /v1/models endpoint by forwarding to Regolo."""
    api_key = check_api_key()
    
    import urllib.request
    
    try:
        req = urllib.request.Request(
            f"{REGOLO_BASE_URL}/models",
            headers={"Authorization": f"Bearer {api_key}"}
        )
        
        with urllib.request.urlopen(req, timeout=30) as response:
            models_response = json.loads(response.read().decode('utf-8'))
        
        return web.json_response(models_response)
        
    except Exception as e:
        return web.json_response(
            {"error": f"Failed to fetch models: {str(e)}"},
            status=HTTPStatus.INTERNAL_SERVER_ERROR
        )


async def handle_health(request: web.Request) -> web.Response:
    """Health check endpoint."""
    return web.json_response({"status": "ok", "service": "regolo-messages-proxy"})


def create_app():
    """Create the proxy application."""
    app = web.Application()
    app.router.add_post('/v1/messages', handle_messages)
    app.router.add_get('/v1/models', handle_models)
    app.router.add_get('/health', handle_health)
    app.router.add_get('/', handle_health)
    return app


def main():
    """Start the proxy server."""
    import signal
    
    port = int(os.environ.get("PORT", 0))
    host = "0.0.0.0"
    
    app = create_app()
    
    runner = web.AppRunner(app)
    asyncio.get_event_loop().run_until_complete(runner.setup())
    site = web.TCPSite(runner, host, port)
    
    try:
        asyncio.get_event_loop().run_until_complete(site.start())
    except OSError as e:
        if "Address already in use" in str(e):
            site = web.TCPSite(runner, host, 0)
            asyncio.get_event_loop().run_until_complete(site.start())
    
    actual_port = site._server.sockets[0].getsockname()[1]
    
    print(f"Starting Regolo Messages Proxy on port {actual_port}", flush=True)
    print(f"Regolo API: {REGOLO_BASE_URL}", flush=True)
    print(f"\nEndpoints:", flush=True)
    print(f"  POST /v1/messages  - Translated to /v1/chat/completions", flush=True)
    print(f"  GET  /v1/models    - Forwarded to Regolo", flush=True)
    print(f"  GET  /health       - Health check", flush=True)
    print(f"\nTo use with Claude:", flush=True)
    print(f"  ANTHROPIC_BASE_URL=http://localhost:{actual_port} ANTHROPIC_API_KEY=your_key claude", flush=True)
    print(f"", flush=True)
    
    sys.stdout.flush()
    
    def signal_handler(signum, frame):
        print(f"\nReceived signal {signum}, shutting down...", flush=True)
        asyncio.get_event_loop().stop()
    
    signal.signal(signal.SIGINT, signal_handler)
    signal.signal(signal.SIGTERM, signal_handler)
    
    try:
        asyncio.get_event_loop().run_forever()
    except KeyboardInterrupt:
        pass
    finally:
        asyncio.get_event_loop().run_until_complete(runner.cleanup())


if __name__ == "__main__":
    main()
