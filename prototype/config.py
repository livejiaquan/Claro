import json
import os


DEFAULT_CONFIG = {
    "whisper_model": "large-v3-mlx",
    "llm_model": "mlx-community/Qwen2.5-7B-Instruct-4bit",
    "llm_enabled": True,
}


def _write_defaults(path: str) -> None:
    directory = os.path.dirname(path)
    if directory:
        os.makedirs(directory, mode=0o700, exist_ok=True)
        os.chmod(directory, 0o700)

    fd = os.open(path, os.O_CREAT | os.O_WRONLY | os.O_TRUNC, 0o600)
    os.fchmod(fd, 0o600)
    with os.fdopen(fd, "w", encoding="utf-8") as f:
        json.dump(DEFAULT_CONFIG, f, ensure_ascii=False, indent=2)
        f.write("\n")


def load_config(path: str = os.path.expanduser("~/.claro/config.json")) -> dict:
    path = os.path.expanduser(path)
    if not os.path.exists(path):
        _write_defaults(path)
        return dict(DEFAULT_CONFIG)

    try:
        with open(path, encoding="utf-8") as f:
            data = json.load(f)
        if not isinstance(data, dict):
            raise ValueError("config root must be an object")
    except Exception as e:
        print(f"Warning: could not parse config {path}: {e}", flush=True)
        return dict(DEFAULT_CONFIG)

    cfg = dict(DEFAULT_CONFIG)
    cfg.update(data)
    return cfg
