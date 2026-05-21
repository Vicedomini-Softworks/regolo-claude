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


def translate_messages_to_completion(messages: list[dict], model: str, extra_params: dict = None) -> dict:
    """
    Translate Claude's /v1/messages format to Regolo's /v1/chat/completions format.
    Pass through any extra parameters (like reasoning_effort) from Claude.
    """
    result = {
        "model": model,
        "messages": messages,
        "max_tokens": extra_params.get("max_tokens", 4096) if extra_params else 4096,
        "temperature": extra_params.get("temperature", 0.7) if extra_params else 0.7,
        "stream": False,
    }

    if extra_params:
        for key in ["reasoning_effort", "top_p", "stop", "seed"]:
            if key in extra_params:
                result[key] = extra_params[key]
    
    return result


def translate_completion_to_messages(completion_response: dict, model: str = "") -> dict:
    """
    Translate Regolo's chat completion response to Anthropic Messages API format.
    Claude Code expects this format, not OpenAI chat completion format.
    """
    content = ""

    if "choices" in completion_response and completion_response["choices"]:
        msg = completion_response["choices"][0].get("message", {})
        content = msg.get("content") or msg.get("reasoning_content") or ""
        if not isinstance(content, str):
            content = str(content) if content else ""

    usage = completion_response.get("usage", {})
    return {
        "id": f"msg_{os.urandom(8).hex()}",
        "type": "message",
        "role": "assistant",
        "model": completion_response.get("model", model or "unknown"),
        "content": [{"type": "text", "text": content}],
        "stop_reason": "end_turn",
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
    
    if DEBUG:
        print(f"[DEBUG] Extra params: {extra_params}", flush=True)
    
    if not messages:
        return web.json_response(
            {"error": "No messages provided"}, 
            status=HTTPStatus.BAD_REQUEST
        )
    
    completion_data = translate_messages_to_completion(messages, model, extra_params)
    
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
