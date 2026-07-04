import json
import history


def test_append_creates_file_and_returns_entry(tmp_path):
    path = str(tmp_path / "sub" / "history.jsonl")
    entry = history.append_entry(
        raw="原始", text="潤飾", duration_s=2.34, status="pasted", path=path
    )
    assert entry["raw"] == "原始"
    assert entry["text"] == "潤飾"
    assert entry["duration_s"] == 2.3
    assert entry["status"] == "pasted"
    assert "ts" in entry
    with open(path, encoding="utf-8") as f:
        lines = f.readlines()
    assert len(lines) == 1
    assert json.loads(lines[0])["text"] == "潤飾"


def test_read_recent_returns_last_n_in_order(tmp_path):
    path = str(tmp_path / "history.jsonl")
    for i in range(5):
        history.append_entry(raw=f"r{i}", text=f"t{i}", duration_s=1, status="pasted", path=path)
    recent = history.read_recent(3, path=path)
    assert [e["text"] for e in recent] == ["t2", "t3", "t4"]


def test_read_recent_missing_file_returns_empty(tmp_path):
    assert history.read_recent(path=str(tmp_path / "nope.jsonl")) == []


def test_read_recent_skips_corrupt_lines(tmp_path):
    path = str(tmp_path / "history.jsonl")
    history.append_entry(raw="r", text="ok", duration_s=1, status="pasted", path=path)
    with open(path, "a", encoding="utf-8") as f:
        f.write("not json\n")
    recent = history.read_recent(path=path)
    assert len(recent) == 1 and recent[0]["text"] == "ok"
