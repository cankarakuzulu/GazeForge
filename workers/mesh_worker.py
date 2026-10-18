"""Image-to-mesh and image-to-image calls to an external server with a ComfyUI-compatible HTTP API.

usage: mesh_worker.py mesh <args.json>    args: server, graph, front, profile, out
       mesh_worker.py image <args.json>   args: server, graph, image, out

The graph is an API-format JSON template. String values that equal a placeholder are replaced:
$FRONT, $PROFILE, $IMAGE (uploaded image names), $FRONT_W, $FRONT_H, $PROFILE_W, $PROFILE_H, $IMAGE_W, $IMAGE_H
(integers); a string that starts with $PREFIX gets a per-run prefix. The first output file with the wanted extension
(.glb for mesh, an image for image) is downloaded to out. One JSON object is printed; the exit code is non-zero on failure.
Standard library only.
"""
import json
import os
import struct
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid


def emit(obj, code=0):
    print(json.dumps(obj), flush=True)
    sys.exit(code)


def fail(message, code="worker_failed", **details):
    emit({"ok": False, "error": {"code": code, "message": message, "details": details}}, 1)


def png_size(path):
    with open(path, "rb") as fh:
        head = fh.read(24)
    if head[:8] != b"\x89PNG\r\n\x1a\n":
        fail(f"{path} is not a PNG", "invalid_input")
    return struct.unpack(">II", head[16:24])


def request(server, path, body=None, headers=None, timeout=60):
    req = urllib.request.Request(server.rstrip("/") + path, body, headers or {})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as resp:
            return resp.read()
    except urllib.error.HTTPError as e:
        fail(f"{path}: HTTP {e.code}: {e.read().decode(errors='replace')[:2000]}")
    except OSError as e:
        fail(f"{server}{path}: {e}", "worker_failed")


def upload(server, path, name):
    boundary = uuid.uuid4().hex
    with open(path, "rb") as fh:
        data = fh.read()
    body = (f"--{boundary}\r\nContent-Disposition: form-data; name=\"image\"; filename=\"{name}\"\r\n"
            f"Content-Type: image/png\r\n\r\n").encode() + data + (
            f"\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"overwrite\"\r\n\r\ntrue\r\n--{boundary}--\r\n").encode()
    reply = json.loads(request(server, "/upload/image", body, {"Content-Type": f"multipart/form-data; boundary={boundary}"}))
    return (reply.get("subfolder") + "/" if reply.get("subfolder") else "") + reply["name"]


def substitute(node, values, prefix):
    if isinstance(node, dict):
        return {k: substitute(v, values, prefix) for k, v in node.items()}
    if isinstance(node, list):
        return [substitute(v, values, prefix) for v in node]
    if isinstance(node, str):
        if node in values:
            return values[node]
        if node.startswith("$PREFIX"):
            return prefix + node[len("$PREFIX"):]
    return node


def run_graph(server, graph, timeout):
    reply = json.loads(request(server, "/prompt", json.dumps({"prompt": graph, "client_id": uuid.uuid4().hex}).encode(),
                               {"Content-Type": "application/json"}))
    if reply.get("node_errors"):
        fail("the server refused the graph (node_errors)", node_errors=reply["node_errors"])
    pid = reply["prompt_id"]
    started = time.time()
    while time.time() - started < timeout:
        history = json.loads(request(server, f"/history/{pid}"))
        if pid in history:
            status = history[pid].get("status", {})
            if status.get("status_str") == "error":
                fail("the graph failed on the server", messages=[m for m in status.get("messages", []) if m[0] == "execution_error"])
            files = []
            for node in history[pid].get("outputs", {}).values():
                for items in node.values():
                    for item in items if isinstance(items, list) else []:
                        if isinstance(item, dict) and "filename" in item and item.get("type", "output") == "output":
                            files.append(item)
            return pid, files, time.time() - started
        time.sleep(2)
    fail(f"the graph did not finish within {timeout} s", prompt_id=pid)


def download(server, item, out):
    query = urllib.parse.urlencode({"filename": item["filename"], "subfolder": item.get("subfolder", ""), "type": "output"})
    data = request(server, f"/view?{query}", timeout=600)
    os.makedirs(os.path.dirname(os.path.abspath(out)), exist_ok=True)
    with open(out, "wb") as fh:
        fh.write(data)
    return len(data)


def main(argv):
    if len(argv) != 3 or argv[1] not in ("mesh", "image"):
        fail("usage: mesh_worker.py mesh|image <args.json>", "invalid_input")
    with open(argv[2], encoding="utf-8") as fh:
        args = json.load(fh)
    server, out = args["server"], args["out"]
    with open(args["graph"], encoding="utf-8") as fh:
        template = json.load(fh)
    tag = uuid.uuid4().hex[:8]
    values = {}
    roles = ("front", "profile") if argv[1] == "mesh" else ("image",)
    for role in roles:
        path = args[role]
        w, h = png_size(path)
        values[f"${role.upper()}"] = upload(server, path, f"gf_{tag}_{role}.png")
        values[f"${role.upper()}_W"], values[f"${role.upper()}_H"] = w, h
    graph = substitute(template, values, f"gf_{tag}")
    pid, files, seconds = run_graph(server, graph, int(args.get("timeout", 2400)))
    wanted = (".glb",) if argv[1] == "mesh" else (".png", ".jpg", ".jpeg", ".webp")
    picked = [f for f in files if f["filename"].lower().endswith(wanted)]
    if not picked:
        fail("the server finished but returned no matching output file", outputs=[f["filename"] for f in files])
    size = download(server, picked[0], out)
    emit({"ok": True, "out": out, "bytes": size, "seconds": seconds, "prompt_id": pid, "outputs": [f["filename"] for f in files]})


if __name__ == "__main__":
    main(sys.argv)
