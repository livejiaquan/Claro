import json
import stat

import config


def test_missing_file_creates_defaults_with_private_mode(tmp_path):
    path = tmp_path / "sub" / "config.json"

    cfg = config.load_config(str(path))

    assert cfg == config.DEFAULT_CONFIG
    assert json.loads(path.read_text(encoding="utf-8")) == config.DEFAULT_CONFIG
    assert stat.S_IMODE(path.stat().st_mode) == 0o600


def test_valid_file_overrides_defaults(tmp_path):
    path = tmp_path / "config.json"
    path.write_text(
        json.dumps(
            {
                "whisper_model": "large-v3-turbo",
                "llm_model": "mlx-community/Qwen2.5-1.5B-Instruct-4bit",
                "llm_enabled": False,
            }
        ),
        encoding="utf-8",
    )

    cfg = config.load_config(str(path))

    assert cfg["whisper_model"] == "large-v3-turbo"
    assert cfg["llm_model"] == "mlx-community/Qwen2.5-1.5B-Instruct-4bit"
    assert cfg["llm_enabled"] is False


def test_corrupt_json_returns_defaults_without_exception(tmp_path):
    path = tmp_path / "config.json"
    path.write_text("{not json", encoding="utf-8")

    cfg = config.load_config(str(path))

    assert cfg == config.DEFAULT_CONFIG


def test_unknown_keys_are_preserved_in_returned_config(tmp_path):
    path = tmp_path / "config.json"
    path.write_text(
        json.dumps({"api_endpoint": "http://localhost:8000"}),
        encoding="utf-8",
    )

    cfg = config.load_config(str(path))

    assert cfg["api_endpoint"] == "http://localhost:8000"
    assert cfg["whisper_model"] == config.DEFAULT_CONFIG["whisper_model"]
