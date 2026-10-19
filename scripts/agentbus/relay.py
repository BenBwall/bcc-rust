"""Forward sandbox MCP, hooks and CLI calls to the host over a shared folder.

Only the host opens SQLite. Requests and responses are atomically published
JSON files; no network listener, shell command, or arbitrary environment is
accepted. Run `python scripts/agentbus/relay.py --help` on the host for setup.
"""

import argparse
from concurrent.futures import ThreadPoolExecutor
from contextlib import contextmanager
import json
import os
from pathlib import Path
import subprocess
import sys
import threading
import time
import uuid


ENVIRONMENT_KEYS = ("AGENTBUS_AGENT", "CODEX_THREAD_ID", "CLAUDE_CODE_SESSION_ID")
MAX_REQUEST_BYTES = 1024 * 1024
HEARTBEAT_MAX_AGE = 15


def client_main(directory, arguments):
    try:
        if arguments == ["serve"]:
            # Agentbus has no connection-local protocol state. Keep the client
            # stream open, including across notifications, forwarding one line
            # at a time to the same host bus.
            for line in sys.stdin:
                result = exchange(directory, arguments, line)
                emit(result)
                if result["returncode"]:
                    return result["returncode"]
            return 0
        incoming = sys.stdin.read() if arguments[:1] == ["hook"] else ""
        result = exchange(directory, arguments, incoming, timeout=3660 if arguments[:1] == ["hook"] else 30)
        emit(result)
        return result["returncode"]
    except (OSError, ValueError, TimeoutError) as error:
        print(f"agentbus relay: {error}", file=sys.stderr)
        return 1


def exchange(directory, arguments, incoming, timeout=30):
    directory = Path(directory)
    instance = check_heartbeat(directory)
    identifier = uuid.uuid4().hex
    request = directory / "requests" / f"{identifier}.json"
    response = directory / "responses" / f"{identifier}.json"
    waiting = directory / "requests" / f"{identifier}.waiting"
    payload = {
        "args": arguments,
        "stdin": incoming,
        "env": {key: os.environ[key] for key in ENVIRONMENT_KEYS if key in os.environ},
        "expires": time.time() + timeout,
    }
    if len(json.dumps(payload).encode("utf-8")) > MAX_REQUEST_BYTES:
        raise ValueError("request exceeds 1 MiB")
    deadline = time.monotonic() + timeout
    try:
        waiting.touch()
        publish(request, payload)
        while True:
            if response.is_file():
                return json.loads(response.read_text(encoding="utf-8"))
            if check_heartbeat(directory) != instance:
                raise OSError("host relay restarted; the request may have run (do not retry blindly)")
            if time.monotonic() >= deadline:
                raise TimeoutError("response timed out; the request may have run (do not retry blindly)")
            time.sleep(0.1)
    finally:
        waiting.unlink(missing_ok=True)
        request.unlink(missing_ok=True)
        response.unlink(missing_ok=True)


def relay_directory(checkout):
    explicit = os.environ.get("AGENTBUS_RELAY_DIR")
    if explicit:
        return Path(explicit)
    common = subprocess.check_output(
        ["git", "rev-parse", "--path-format=absolute", "--git-common-dir"],
        cwd=checkout, text=True, encoding="utf-8",
    ).strip()
    directory = Path(common) / "agentbus" / "relay"
    config = directory / "config.json"
    if config.is_file():
        sandboxes = json.loads(config.read_text(encoding="utf-8"))["sandboxes"]
        if os.environ.get("SANDBOX_NAME") in sandboxes:
            return directory
    return None


def emit(result):
    sys.stdout.write(result["stdout"])
    sys.stdout.flush()
    sys.stderr.write(result["stderr"])
    sys.stderr.flush()


def check_heartbeat(directory):
    try:
        heartbeat = json.loads((directory / "heartbeat").read_text(encoding="utf-8"))
        timestamp = heartbeat["timestamp"]
    except (OSError, ValueError, KeyError, TypeError) as error:
        raise OSError("host relay is unavailable; start relay.py on the host") from error
    if time.time() - timestamp > HEARTBEAT_MAX_AGE:
        raise OSError("host relay stopped; start relay.py on the host")
    return heartbeat["instance"]


def publish(path, value):
    temporary = path.with_suffix(f".{uuid.uuid4().hex}.tmp")
    try:
        temporary.write_text(json.dumps(value, ensure_ascii=False), encoding="utf-8")
        temporary.replace(path)
    finally:
        temporary.unlink(missing_ok=True)


def serve(directory, executable, checkout, database, sandboxes, stop=None):
    directory = Path(directory)
    directory.mkdir(parents=True, exist_ok=True)
    for name in ("requests", "responses"):
        (directory / name).mkdir(exist_ok=True)
    stop = stop or threading.Event()
    with exclusive_host_lock(directory), ThreadPoolExecutor(max_workers=16) as workers:
        # Only selected sandboxes route through this relay automatically.
        publish(directory / "config.json", {"sandboxes": sandboxes})
        # Never replay a request that might have executed before a host crash.
        for abandoned in (directory / "requests").glob("*.running"):
            publish(directory / "responses" / abandoned.with_suffix(".json").name, {
                "returncode": 1, "stdout": "",
                "stderr": "agentbus relay restarted; the request may have run (do not retry blindly)\n",
            })
            abandoned.unlink()
        pending = set()
        instance = uuid.uuid4().hex
        heartbeat_at = 0
        try:
            while not stop.is_set():
                if time.monotonic() >= heartbeat_at:
                    publish(directory / "heartbeat", {"timestamp": time.time(), "instance": instance, "pid": os.getpid()})
                    heartbeat_at = time.monotonic() + 1
                # Bound concurrency while leaving AFK hooks independent of sends.
                for future in list(pending):
                    if future.done():
                        pending.remove(future)
                        if future.exception() is not None:
                            print(f"agentbus relay worker: {future.exception()}", file=sys.stderr, flush=True)
                for request in (directory / "requests").glob("*.json"):
                    if len(pending) >= 16:
                        break
                    running = request.with_suffix(".running")
                    try:
                        request.replace(running)
                    except FileNotFoundError:
                        continue
                    pending.add(workers.submit(
                        respond, running, directory / "responses" / request.name,
                        executable, checkout, database, stop,
                    ))
                stop.wait(0.1)
        finally:
            (directory / "heartbeat").unlink(missing_ok=True)


def respond(request, response, executable, checkout, database, stop=None):
    try:
        if request.stat().st_size > MAX_REQUEST_BYTES:
            raise ValueError("request exceeds 1 MiB")
        payload = json.loads(request.read_text(encoding="utf-8"))
        if not request.with_suffix(".waiting").is_file():
            request.unlink(missing_ok=True)
            return
        arguments = payload["args"]
        if not isinstance(arguments, list) or not all(isinstance(arg, str) for arg in arguments):
            raise ValueError("args must be a list of strings")
        if time.time() >= payload["expires"]:
            raise ValueError("request expired before execution")
        environment = {key: value for key, value in os.environ.items() if key not in ENVIRONMENT_KEYS}
        environment.update({key: value for key, value in payload["env"].items() if key in ENVIRONMENT_KEYS})
        environment["AGENTBUS_DB"] = str(database)
        if stop is not None and stop.is_set():
            raise ValueError("host relay is stopping")
        result = subprocess.run(
            [str(executable), *arguments], input=payload["stdin"],
            cwd=checkout, env=environment, text=True, encoding="utf-8",
            capture_output=True, timeout=min(3600, payload["expires"] - time.time()),
        )
        output = {"returncode": result.returncode, "stdout": result.stdout, "stderr": result.stderr}
    except (OSError, ValueError, KeyError, TypeError, subprocess.TimeoutExpired) as error:
        output = {"returncode": 1, "stdout": "", "stderr": f"agentbus relay: {error}\n"}
    try:
        publish(response, output)
    finally:
        request.unlink(missing_ok=True)


@contextmanager
def exclusive_host_lock(directory):
    # OS locks disappear when the process exits, including after a crash.
    with (directory / "host.lock").open("a+b") as lock:
        lock.seek(0)
        lock.write(b"0")
        lock.flush()
        lock.seek(0)
        if os.name == "nt":
            import msvcrt
            msvcrt.locking(lock.fileno(), msvcrt.LK_NBLCK, 1)
        else:
            import fcntl
            fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        yield


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", required=True, type=Path)
    parser.add_argument("--executable", required=True, type=Path)
    parser.add_argument("--database", required=True, type=Path)
    parser.add_argument("--sandbox", action="append", default=[])
    options = parser.parse_args()
    serve(
        options.directory.resolve(), options.executable.resolve(),
        Path(__file__).resolve().parents[2], options.database.resolve(), options.sandbox,
    )


if __name__ == "__main__":
    main()
