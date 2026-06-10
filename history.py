import json
import os
from datetime import datetime

HISTORY_PATH = os.path.expanduser("~/.claro/history.jsonl")


def append_entry(raw: str, text: str, duration_s: float, status: str,
                 path: str = HISTORY_PATH) -> dict:
    os.makedirs(os.path.dirname(path), exist_ok=True)
    os.chmod(os.path.dirname(path), 0o700)
    entry = {
        "ts": datetime.now().astimezone().isoformat(timespec="seconds"),
        "duration_s": round(duration_s, 1),
        "raw": raw,
        "text": text,
        "status": status,
    }
    fd = os.open(path, os.O_CREAT | os.O_APPEND | os.O_WRONLY, 0o600)
    os.fchmod(fd, 0o600)
    with os.fdopen(fd, "a", encoding="utf-8") as f:
        f.write(json.dumps(entry, ensure_ascii=False) + "\n")
    return entry


def read_recent(n: int = 20, path: str = HISTORY_PATH) -> list[dict]:
    try:
        with open(path, encoding="utf-8") as f:
            lines = f.readlines()
    except FileNotFoundError:
        return []
    out = []
    for line in lines[-n:]:
        try:
            out.append(json.loads(line))
        except json.JSONDecodeError:
            continue
    return out
