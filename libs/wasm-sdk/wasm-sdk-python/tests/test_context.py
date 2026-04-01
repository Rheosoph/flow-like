from __future__ import annotations

import json

import pytest

from flow_like_wasm_sdk.context import Context
from flow_like_wasm_sdk.host import MockHostBridge
from flow_like_wasm_sdk.types import ExecutionInput, LogLevel


def _ctx(
    inputs: dict | None = None,
    stream: bool = False,
    log_level: int = LogLevel.DEBUG,
) -> tuple[Context, MockHostBridge]:
    host = MockHostBridge()
    ei = ExecutionInput(
        inputs=inputs or {},
        node_id="n1",
        run_id="r1",
        app_id="a1",
        board_id="b1",
        user_id="u1",
        node_name="TestNode",
        stream_state=stream,
        log_level=log_level,
    )
    return Context(ei, host), host


class TestMetadata:
    def test_properties(self) -> None:
        ctx, _ = _ctx()
        assert ctx.node_id == "n1"
        assert ctx.node_name == "TestNode"
        assert ctx.run_id == "r1"
        assert ctx.app_id == "a1"
        assert ctx.board_id == "b1"
        assert ctx.user_id == "u1"

    def test_from_dict(self) -> None:
        host = MockHostBridge()
        ctx = Context.from_dict({"inputs": {"x": 1}, "node_id": "abc"}, host)
        assert ctx.node_id == "abc"
        assert ctx.get_input("x") == 1

    def test_from_json(self) -> None:
        host = MockHostBridge()
        ctx = Context.from_json(json.dumps({"inputs": {"y": 2}}), host)
        assert ctx.get_input("y") == 2


class TestInputGetters:
    def test_get_string(self) -> None:
        ctx, _ = _ctx({"s": "hello"})
        assert ctx.get_string("s") == "hello"
        assert ctx.get_string("missing") is None
        assert ctx.get_string("missing", "fallback") == "fallback"

    def test_get_i64(self) -> None:
        ctx, _ = _ctx({"n": 42})
        assert ctx.get_i64("n") == 42
        assert ctx.get_i64("missing", -1) == -1

    def test_get_f64(self) -> None:
        ctx, _ = _ctx({"f": 3.14})
        assert ctx.get_f64("f") == pytest.approx(3.14)
        assert ctx.get_f64("missing") is None

    def test_get_bool(self) -> None:
        ctx, _ = _ctx({"b": True})
        assert ctx.get_bool("b") is True
        assert ctx.get_bool("missing") is None

    def test_require_input_present(self) -> None:
        ctx, _ = _ctx({"x": 99})
        assert ctx.require_input("x") == 99

    def test_require_input_missing(self) -> None:
        ctx, _ = _ctx()
        with pytest.raises(ValueError, match="Required input"):
            ctx.require_input("x")


class TestOutputSetters:
    def test_set_output(self) -> None:
        ctx, _ = _ctx()
        ctx.set_output("result", "ok")
        r = ctx.finish()
        assert r.outputs["result"] == "ok"

    def test_activate_exec(self) -> None:
        ctx, _ = _ctx()
        ctx.activate_exec("branch_a")
        r = ctx.finish()
        assert "branch_a" in r.activate_exec

    def test_set_pending(self) -> None:
        ctx, _ = _ctx()
        ctx.set_pending(True)
        r = ctx.finish()
        assert r.pending is True


class TestLogging:
    def test_debug_at_debug_level(self) -> None:
        ctx, host = _ctx(log_level=LogLevel.DEBUG)
        ctx.debug("msg")
        assert len(host.logs) == 1
        assert host.logs[0] == (LogLevel.DEBUG, "msg")

    def test_debug_suppressed_at_info_level(self) -> None:
        ctx, host = _ctx(log_level=LogLevel.INFO)
        ctx.debug("msg")
        assert len(host.logs) == 0

    def test_info_at_info_level(self) -> None:
        ctx, host = _ctx(log_level=LogLevel.INFO)
        ctx.info("msg")
        assert len(host.logs) == 1

    def test_warn(self) -> None:
        ctx, host = _ctx(log_level=LogLevel.WARN)
        ctx.warn("w")
        ctx.info("i")
        assert len(host.logs) == 1

    def test_error(self) -> None:
        ctx, host = _ctx(log_level=LogLevel.ERROR)
        ctx.error("e")
        ctx.warn("w")
        assert len(host.logs) == 1


class TestStreaming:
    def test_stream_text_enabled(self) -> None:
        ctx, host = _ctx(stream=True)
        ctx.stream_text("hello")
        assert host.streams == [("text", "hello")]

    def test_stream_text_disabled(self) -> None:
        ctx, host = _ctx(stream=False)
        ctx.stream_text("hello")
        assert host.streams == []

    def test_stream_json(self) -> None:
        ctx, host = _ctx(stream=True)
        ctx.stream_json({"k": "v"})
        assert len(host.streams) == 1
        assert json.loads(host.streams[0][1]) == {"k": "v"}

    def test_stream_progress(self) -> None:
        ctx, host = _ctx(stream=True)
        ctx.stream_progress(0.5, "halfway")
        payload = json.loads(host.streams[0][1])
        assert payload["progress"] == 0.5
        assert payload["message"] == "halfway"


class TestVariables:
    def test_get_set(self) -> None:
        ctx, host = _ctx()
        assert ctx.get_variable("x") is None
        assert ctx.set_variable("x", 42) is True
        assert ctx.get_variable("x") == 42


class TestFinalize:
    def test_success(self) -> None:
        ctx, _ = _ctx()
        ctx.set_output("v", 1)
        r = ctx.success()
        assert "exec_out" in r.activate_exec
        assert r.outputs["v"] == 1
        assert r.error is None

    def test_fail(self) -> None:
        ctx, _ = _ctx()
        r = ctx.fail("broken")
        assert r.error == "broken"
        assert "exec_out" not in r.activate_exec

    def test_finish(self) -> None:
        ctx, _ = _ctx()
        ctx.set_output("a", 1)
        r = ctx.finish()
        assert r.outputs["a"] == 1
        assert r.error is None
        assert r.activate_exec == []


class TestStorageContext:
    def test_storage_dir(self) -> None:
        ctx, _ = _ctx()
        result = ctx.storage_dir()
        assert result["path"] == "storage"
        result_scoped = ctx.storage_dir(node_scoped=True)
        assert result_scoped["path"] == "storage/node"

    def test_upload_dir(self) -> None:
        ctx, _ = _ctx()
        assert ctx.upload_dir()["path"] == "upload"

    def test_cache_dir(self) -> None:
        ctx, _ = _ctx()
        assert ctx.cache_dir()["path"] == "tmp/cache"

    def test_user_dir(self) -> None:
        ctx, _ = _ctx()
        assert ctx.user_dir()["path"] == "users/mock"

    def test_storage_read_write(self) -> None:
        ctx, _ = _ctx()
        fp = {"path": "data.bin", "store_ref": "s", "cache_store_ref": None}
        assert ctx.storage_read(fp) is None
        assert ctx.storage_write(fp, b"content") is True
        assert ctx.storage_read(fp) == b"content"

    def test_storage_list(self) -> None:
        ctx, _ = _ctx()
        ctx.storage_write({"path": "dir/a"}, b"a")
        ctx.storage_write({"path": "dir/b"}, b"b")
        result = ctx.storage_list({"path": "dir/", "store_ref": "s"})
        paths = [r["path"] for r in result]
        assert "dir/a" in paths
        assert "dir/b" in paths

    def test_embed_text(self) -> None:
        ctx, _ = _ctx()
        result = ctx.embed_text({"id": "m"}, ["test"])
        assert len(result) == 1
        assert result[0] == [0.1, 0.2, 0.3]
