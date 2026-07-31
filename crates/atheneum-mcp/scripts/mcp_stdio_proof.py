#!/usr/bin/env python3
"""One-shot stdio MCP session against the freshly built atheneum-mcp binary.

Drives: initialize -> notifications/initialized -> tools/call code_query
(magellan status, project=magellan) -> tools/call search kind=all.
Prints the verbatim JSON-RPC transcript so it can be pasted as evidence.
"""
import json
import subprocess
import sys

BIN = sys.argv[1]


def send(proc, msg):
    line = json.dumps(msg)
    print(f">>> {line}", flush=True)
    proc.stdin.write(line + "\n")
    proc.stdin.flush()


def recv(proc):
    line = proc.stdout.readline()
    if not line:
        raise RuntimeError("server closed stdout unexpectedly")
    print(f"<<< {line.rstrip()}", flush=True)
    return json.loads(line)


proc = subprocess.Popen(
    [BIN],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.DEVNULL,
    text=True,
    bufsize=1,
)

send(proc, {
    "jsonrpc": "2.0",
    "id": 1,
    "method": "initialize",
    "params": {
        "protocolVersion": "2025-03-26",
        "capabilities": {},
        "clientInfo": {"name": "stdio-proof", "version": "0.1.0"},
    },
})
init = recv(proc)
assert "result" in init, f"initialize failed: {init}"
print(f"# server: {init['result'].get('serverInfo')}", flush=True)

send(proc, {"jsonrpc": "2.0", "method": "notifications/initialized"})

send(proc, {
    "jsonrpc": "2.0",
    "id": 2,
    "method": "tools/call",
    "params": {
        "name": "code_query",
        "arguments": {"project": "magellan", "tool": "magellan", "subcommand": "status"},
    },
})
cq = recv(proc)
assert "result" in cq, f"code_query failed: {cq}"

send(proc, {
    "jsonrpc": "2.0",
    "id": 3,
    "method": "tools/call",
    "params": {
        "name": "search",
        "arguments": {"query": "CrossRouter", "kind": "all", "limit": 5},
    },
})
sr = recv(proc)
assert "result" in sr, f"search failed: {sr}"

# ---- verdicts ----
def payload(resp):
    content = resp["result"]["content"][0]["text"]
    return json.loads(content)

cq_data = payload(cq)
print("\n# === code_query verdict ===", flush=True)
cq_text = cq["result"]["content"][0]["text"]
print(f"# isError: {cq['result'].get('isError')}", flush=True)
has_backend_unavail = "BACKEND_UNAVAILABLE" in cq_text
print(f"# contains BACKEND_UNAVAILABLE: {has_backend_unavail}", flush=True)
items = cq_data.get("items", []) if isinstance(cq_data, dict) else []
print(f"# items count: {len(items)}", flush=True)
if items:
    print(f"# first item keys: {sorted(items[0].keys())}", flush=True)
    print(f"# first item provenance: {items[0].get('provenance')}", flush=True)
print("# code_query raw payload (first 1200 chars):", flush=True)
print(f"# {cq_text[:1200]}", flush=True)

sr_data = payload(sr)
print("\n# === search kind=all verdict ===", flush=True)
sr_text = sr["result"]["content"][0]["text"]
sitems = sr_data.get("items", []) if isinstance(sr_data, dict) else []
code_items = [i for i in sitems if i.get("provenance") == "EXTRACTED" or i.get("source") == "code"]
print(f"# items count: {len(sitems)}; code-side items: {len(code_items)}", flush=True)
print(f"# code_stale field: {sr_data.get('code_stale') if isinstance(sr_data, dict) else None}", flush=True)
print(f"# errors: {sr_data.get('errors') if isinstance(sr_data, dict) else None}", flush=True)
print("# search raw payload (first 1200 chars):", flush=True)
print(f"# {sr_text[:1200]}", flush=True)

ok = not has_backend_unavail and items and not cq["result"].get("isError")
print(f"\n# PROOF {'PASS' if ok else 'FAIL'}", flush=True)

proc.stdin.close()
proc.terminate()
sys.exit(0 if ok else 1)
