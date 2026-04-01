from __future__ import annotations

from flow_like_wasm_sdk.host import HostBridge, MockHostBridge, get_host, set_host


class TestHostBridge:
    def test_defaults(self) -> None:
        h = HostBridge()
        h.log(0, "msg")
        h.stream("t", "d")
        assert h.get_variable("x") is None
        assert h.set_variable("x", 1) is False
        assert h.time_now() == 0
        assert h.random() == 0

    def test_storage_defaults(self) -> None:
        h = HostBridge()
        assert h.storage_dir(True) is None
        assert h.upload_dir() is None
        assert h.cache_dir(True, False) is None
        assert h.user_dir(False) is None
        assert h.storage_read({"path": "x"}) is None
        assert h.storage_write({"path": "x"}, b"data") is False
        assert h.storage_list({"path": "x"}) is None
        assert h.embed_text({}, ["hello"]) is None


class TestMockHostBridge:
    def test_log(self) -> None:
        h = MockHostBridge()
        h.log(1, "info msg")
        assert h.logs == [(1, "info msg")]

    def test_stream(self) -> None:
        h = MockHostBridge()
        h.stream("text", "data")
        assert h.streams == [("text", "data")]

    def test_variables(self) -> None:
        h = MockHostBridge()
        assert h.get_variable("k") is None
        assert h.set_variable("k", "v") is True
        assert h.get_variable("k") == "v"

    def test_time_and_random(self) -> None:
        h = MockHostBridge()
        assert h.time_now() == 0
        assert h.random() == 42
        h._time = 1000
        h._random_value = 7
        assert h.time_now() == 1000
        assert h.random() == 7


class TestGlobalHost:
    def test_set_and_get(self) -> None:
        original = get_host()
        try:
            mock = MockHostBridge()
            set_host(mock)
            assert get_host() is mock
        finally:
            set_host(original)


class TestMockStorageFunctions:
    def test_storage_dir(self) -> None:
        h = MockHostBridge()
        result = h.storage_dir(False)
        assert result["path"] == "storage"
        assert result["store_ref"] == "mock_store"
        result_scoped = h.storage_dir(True)
        assert result_scoped["path"] == "storage/node"

    def test_upload_dir(self) -> None:
        h = MockHostBridge()
        result = h.upload_dir()
        assert result["path"] == "upload"

    def test_cache_dir(self) -> None:
        h = MockHostBridge()
        result = h.cache_dir(True, False)
        assert result["path"] == "tmp/cache"

    def test_user_dir(self) -> None:
        h = MockHostBridge()
        result = h.user_dir(False)
        assert result["path"] == "users/mock"

    def test_storage_read_write(self) -> None:
        h = MockHostBridge()
        fp = {"path": "test/file.bin", "store_ref": "s", "cache_store_ref": None}
        assert h.storage_read(fp) is None
        assert h.storage_write(fp, b"hello") is True
        assert h.storage_read(fp) == b"hello"

    def test_storage_list(self) -> None:
        h = MockHostBridge()
        fp = {"path": "dir/", "store_ref": "s", "cache_store_ref": None}
        h.storage_write({"path": "dir/a.txt", "store_ref": "s"}, b"a")
        h.storage_write({"path": "dir/b.txt", "store_ref": "s"}, b"b")
        h.storage_write({"path": "other/c.txt", "store_ref": "s"}, b"c")
        result = h.storage_list(fp)
        paths = [r["path"] for r in result]
        assert "dir/a.txt" in paths
        assert "dir/b.txt" in paths
        assert "other/c.txt" not in paths

    def test_embed_text(self) -> None:
        h = MockHostBridge()
        result = h.embed_text({"id": "model"}, ["hello", "world"])
        assert len(result) == 2
        assert result[0] == [0.1, 0.2, 0.3]
        result[0][0] = 999.0
        assert h._embeddings[0][0] == 0.1
