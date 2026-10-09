"""File metadata and signed storage transfers."""
from __future__ import annotations

import os
import io
from collections.abc import AsyncIterator, Iterator
from typing import IO, Any

import httpx

from ._errors import FlowLikeError
from ._http import HTTPClient, segment
from ._types import FileInfo, PresignResult


def _data_path(app_id: str, user: bool, suffix: str = "") -> str:
    return f"/apps/{segment(app_id)}/data" + ("/user" if user else "") + suffix


def _file_info(item: dict[str, Any]) -> FileInfo:
    return FileInfo(key=item.get("location", item.get("key", "")), size=item.get("size"),
                    last_modified=item.get("last_modified"), raw=item)


def _grant(grants: list[dict[str, Any]], key: str) -> dict[str, Any]:
    grant = next((item for item in grants if item.get("prefix") == key), None)
    if not grant or grant.get("error") or not grant.get("url"):
        raise FlowLikeError(str(grant.get("error", "Storage grant is missing")) if grant else "Storage grant is missing")
    url = httpx.URL(grant["url"])
    if url.scheme not in ("https", "http") or not url.host:
        raise FlowLikeError("Storage grant must contain an absolute HTTP(S) URL")
    return grant


def _size(file: IO[bytes], size: int | None) -> int:
    if size is not None:
        if size < 0:
            raise ValueError("size must be nonnegative")
        return size
    try:
        position = file.tell()
        file.seek(0, os.SEEK_END)
        remaining = file.tell() - position
        file.seek(position)
        return remaining
    except (OSError, AttributeError) as error:
        raise ValueError("Provide size for a file that cannot seek") from error


def _chunks(file: IO[bytes]) -> Iterator[bytes]:
    while chunk := file.read(64 * 1024):
        yield chunk


class _FileSlice(io.RawIOBase):
    """Expose the selected bytes so multipart encoders cannot rewind before them."""

    def __init__(self, file: IO[bytes], length: int):
        self.file = file
        self.length = length
        self.position = 0
        try:
            self.start: int | None = file.tell()
        except (OSError, AttributeError):
            self.start = None

    def read(self, size: int = -1) -> bytes:
        remaining = self.length - self.position
        chunk = self.file.read(remaining if size < 0 else min(size, remaining))
        self.position += len(chunk)
        return chunk

    def tell(self) -> int:
        return self.position

    def seek(self, offset: int, whence: int = os.SEEK_SET) -> int:
        target = offset + (self.length if whence == os.SEEK_END else self.position if whence == os.SEEK_CUR else 0)
        if target < 0 or target > self.length:
            raise ValueError("Cannot seek outside the upload slice")
        if self.start is None:
            if target != self.position:
                raise io.UnsupportedOperation("Upload source cannot seek")
        else:
            self.file.seek(self.start + target)
        self.position = target
        return target


async def _achunks(file: IO[bytes]) -> AsyncIterator[bytes]:
    for chunk in _chunks(file):
        yield chunk


def _presign(data: dict[str, Any]) -> PresignResult:
    return PresignResult(raw=data, shared_credentials=data.get("shared_credentials", {}),
                         path=data.get("path", ""), access_mode=data.get("access_mode", "read"),
                         expiration=data.get("expiration"))


class FilesMixin(HTTPClient):
    """Sign transfers with the API, then send file bytes without platform credentials."""

    def list_files(self, app_id: str, *, prefix: str = "", user: bool = False,
                   refresh: bool = False, **kwargs: Any) -> list[FileInfo]:
        params = {**(kwargs.pop("params", None) or {}), "refresh": refresh}
        data = self._json("POST", _data_path(app_id, user, "/list"), json={"prefix": prefix}, params=params, **kwargs)
        return [_file_info(item) for item in data]

    async def alist_files(self, app_id: str, *, prefix: str = "", user: bool = False,
                         refresh: bool = False, **kwargs: Any) -> list[FileInfo]:
        params = {**(kwargs.pop("params", None) or {}), "refresh": refresh}
        data = await self._ajson("POST", _data_path(app_id, user, "/list"), json={"prefix": prefix}, params=params, **kwargs)
        return [_file_info(item) for item in data]

    def get_upload_urls(self, app_id: str, prefixes: list[str], sizes: list[int], *, user: bool = False) -> list[dict[str, Any]]:
        """Request ordered upload grants. Sizes must match prefixes; a grant can contain an error."""
        if len(prefixes) != len(sizes):
            raise ValueError("sizes must contain one size per prefix")
        return self._json("PUT", _data_path(app_id, user), json={"prefixes": prefixes, "sizes": sizes})

    async def aget_upload_urls(self, app_id: str, prefixes: list[str], sizes: list[int], *, user: bool = False) -> list[dict[str, Any]]:
        if len(prefixes) != len(sizes):
            raise ValueError("sizes must contain one size per prefix")
        return await self._ajson("PUT", _data_path(app_id, user), json={"prefixes": prefixes, "sizes": sizes})

    def get_download_urls(self, app_id: str, prefixes: list[str], *, user: bool = False) -> list[dict[str, Any]]:
        """Request ordered download grants; inspect per-prefix errors before transferring."""
        return self._json("POST", _data_path(app_id, user, "/download"), json={"prefixes": prefixes})

    async def aget_download_urls(self, app_id: str, prefixes: list[str], *, user: bool = False) -> list[dict[str, Any]]:
        return await self._ajson("POST", _data_path(app_id, user, "/download"), json={"prefixes": prefixes})

    def upload_file(self, app_id: str, file: IO[bytes], *, key: str | None = None,
                    user: bool = False, size: int | None = None, **kwargs: Any) -> dict[str, Any]:
        """Upload the remaining file bytes using a signed PUT or multipart POST grant."""
        key = key or os.path.basename(str(getattr(file, "name", "upload")))
        size = _size(file, size)
        grant = _grant(self.get_upload_urls(app_id, [key], [size], user=user), key)
        file = _FileSlice(file, size)
        method = grant.get("method", "PUT").upper()
        headers = {**(kwargs.pop("headers", None) or {}), **grant.get("headers", {})}
        if method == "POST":
            self._request(method, grant["url"], data=grant.get("fields", {}),
                          files={"file": (os.path.basename(key), file)}, headers=headers,
                          authenticated=False, **kwargs)
        elif method == "PUT":
            headers["Content-Length"] = str(size)
            self._request(method, grant["url"], content=_chunks(file), headers=headers,
                          authenticated=False, **kwargs)
        else:
            raise FlowLikeError(f"Unsupported signed upload method: {method}")
        return grant

    async def aupload_file(self, app_id: str, file: IO[bytes], *, key: str | None = None,
                          user: bool = False, size: int | None = None, **kwargs: Any) -> dict[str, Any]:
        key = key or os.path.basename(str(getattr(file, "name", "upload")))
        size = _size(file, size)
        grant = _grant(await self.aget_upload_urls(app_id, [key], [size], user=user), key)
        file = _FileSlice(file, size)
        method = grant.get("method", "PUT").upper()
        headers = {**(kwargs.pop("headers", None) or {}), **grant.get("headers", {})}
        if method == "POST":
            await self._arequest(method, grant["url"], data=grant.get("fields", {}),
                                files={"file": (os.path.basename(key), file)}, headers=headers,
                                authenticated=False, **kwargs)
        elif method == "PUT":
            headers["Content-Length"] = str(size)
            await self._arequest(method, grant["url"], content=_achunks(file), headers=headers,
                                authenticated=False, **kwargs)
        else:
            raise FlowLikeError(f"Unsupported signed upload method: {method}")
        return grant

    def download_file(self, app_id: str, key: str, *, user: bool = False, **kwargs: Any) -> bytes:
        grant = _grant(self.get_download_urls(app_id, [key], user=user), key)
        return self._request("GET", grant["url"], authenticated=False, **kwargs).content

    async def adownload_file(self, app_id: str, key: str, *, user: bool = False, **kwargs: Any) -> bytes:
        grant = _grant(await self.aget_download_urls(app_id, [key], user=user), key)
        return (await self._arequest("GET", grant["url"], authenticated=False, **kwargs)).content

    def delete_files(self, app_id: str, prefixes: list[str], *, user: bool = False) -> None:
        self._request("DELETE", _data_path(app_id, user), json={"prefixes": prefixes})

    async def adelete_files(self, app_id: str, prefixes: list[str], *, user: bool = False) -> None:
        await self._arequest("DELETE", _data_path(app_id, user), json={"prefixes": prefixes})

    def delete_file(self, app_id: str, key: str, *, user: bool = False) -> None:
        self.delete_files(app_id, [key], user=user)

    async def adelete_file(self, app_id: str, key: str, *, user: bool = False) -> None:
        await self.adelete_files(app_id, [key], user=user)

    def presign_data(self, app_id: str, *, prefix: str | None = None,
                     access_mode: str = "read", user: bool = False, **kwargs: Any) -> PresignResult:
        """Return scoped cloud credentials, the storage path, and their expiration."""
        body = kwargs.pop("json", {"prefix": prefix, "access_mode": access_mode})
        return _presign(self._json("POST", _data_path(app_id, user, "/presign"), json=body, **kwargs))

    async def apresign_data(self, app_id: str, *, prefix: str | None = None,
                           access_mode: str = "read", user: bool = False, **kwargs: Any) -> PresignResult:
        body = kwargs.pop("json", {"prefix": prefix, "access_mode": access_mode})
        return _presign(await self._ajson("POST", _data_path(app_id, user, "/presign"), json=body, **kwargs))


__all__ = ["FilesMixin"]
