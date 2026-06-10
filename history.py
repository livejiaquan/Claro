import json
import os
from datetime import datetime

HISTORY_PATH = os.path.expanduser("~/.claro/history.jsonl")


def append_entry(raw: str, text: str, duration_s: float, status: str,
                 path: str = HISTORY_PATH) -> dict:
    os.makedirs(os.path.dirname(path), exist_ok=True)
    entry = {
        "ts": datetime.now().astimezone().isoformat(timespec="seconds"),
        "duration_s": round(duration_s, 1),
        "raw": raw,
        "text": text,
        "status": status,
    }
    with open(path, "a", encoding="utf-8") as f:
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
