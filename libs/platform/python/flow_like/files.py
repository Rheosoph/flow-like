"""Mixin for file management operations (list, upload, download, delete, presign)."""

from __future__ import annotations

from typing import IO, Any

from ._http import HTTPClient
from ._types import FileInfo, PresignResult


class FilesMixin(HTTPClient):
    """HTTP mixin that provides file management capabilities."""
    def list_files(self, app_id: str, **kwargs: Any) -> list[FileInfo]:
        """List files associated with an application.

        Args:
            app_id: The application identifier.
            **kwargs: Extra arguments forwarded to the underlying HTTP call.

        Returns:
            A list of ``FileInfo`` objects describing each file.
        """
        resp = self._request("GET", f"/apps/{app_id}/data/list", **kwargs)
        data = resp.json()
        items = data if isinstance(data, list) else data.get("files", [])
        return [
            FileInfo(
                key=f.get("key", ""),
                size=f.get("size"),
                last_modified=f.get("last_modified"),
                raw=f,
            )
            for f in items
        ]

    async def alist_files(self, app_id: str, **kwargs: Any) -> list[FileInfo]:
        """Async version of ``list_files``.

        Args:
            app_id: The application identifier.
            **kwargs: Extra arguments forwarded to the underlying HTTP call.

        Returns:
            A list of ``FileInfo`` objects describing each file.
        """
        resp = await self._arequest("GET", f"/apps/{app_id}/data/list", **kwargs)
        data = resp.json()
        items = data if isinstance(data, list) else data.get("files", [])
        return [
            FileInfo(
                key=f.get("key", ""),
                size=f.get("size"),
                last_modified=f.get("last_modified"),
                raw=f,
            )
            for f in items
        ]

    def upload_file(self, app_id: str, file: IO[bytes], **kwargs: Any) -> dict[str, Any]:
        """Upload a file to an application's data store.

        Args:
            app_id: The application identifier.
            file: A file-like object opened in binary mode.
            **kwargs: Extra arguments forwarded to the underlying HTTP call.

        Returns:
            A dict containing the server's response metadata.
        """
        filename = getattr(file, "name", "upload")
        resp = self._request(
            "POST",
            f"/apps/{app_id}/data/upload",
            files={"file": (filename, file)},
            **kwargs,
        )
        return resp.json()

    async def aupload_file(self, app_id: str, file: IO[bytes], **kwargs: Any) -> dict[str, Any]:
        """Async version of ``upload_file``.

        Args:
            app_id: The application identifier.
            file: A file-like object opened in binary mode.
            **kwargs: Extra arguments forwarded to the underlying HTTP call.

        Returns:
            A dict containing the server's response metadata.
        """
        filename = getattr(file, "name", "upload")
        resp = await self._arequest(
            "POST",
            f"/apps/{app_id}/data/upload",
            files={"file": (filename, file)},
            **kwargs,
        )
        return resp.json()

    def download_file(self, app_id: str, key: str, **kwargs: Any) -> bytes:
        """Download a file's raw content by key.

        Args:
            app_id: The application identifier.
            key: The file key identifying the target file.
            **kwargs: Extra arguments forwarded to the underlying HTTP call.

        Returns:
            The raw bytes of the downloaded file.
        """
        resp = self._request(
            "POST",
            f"/apps/{app_id}/data/download",
            json={"key": key},
            **kwargs,
        )
        return resp.content

    async def adownload_file(self, app_id: str, key: str, **kwargs: Any) -> bytes:
        """Async version of ``download_file``.

        Args:
            app_id: The application identifier.
            key: The file key identifying the target file.
            **kwargs: Extra arguments forwarded to the underlying HTTP call.

        Returns:
            The raw bytes of the downloaded file.
        """
        resp = await self._arequest(
            "POST",
            f"/apps/{app_id}/data/download",
            json={"key": key},
            **kwargs,
        )
        return resp.content

    def delete_file(self, app_id: str, key: str) -> None:
        """Delete a file from an application's data store.

        Args:
            app_id: The application identifier.
            key: The file key identifying the target file.
        """
        self._request("DELETE", f"/apps/{app_id}/data/delete", json={"key": key})

    async def adelete_file(self, app_id: str, key: str) -> None:
        """Async version of ``delete_file``.

        Args:
            app_id: The application identifier.
            key: The file key identifying the target file.
        """
        await self._arequest("DELETE", f"/apps/{app_id}/data/delete", json={"key": key})

    def presign_data(self, app_id: str, **kwargs: Any) -> PresignResult:
        """Generate a presigned URL for direct data access.

        Args:
            app_id: The application identifier.
            **kwargs: Extra arguments forwarded to the underlying HTTP call.

        Returns:
            A ``PresignResult`` containing the presigned URL and headers.
        """
        resp = self._request("POST", f"/apps/{app_id}/data/presign", **kwargs)
        data = resp.json()
        return PresignResult(
            url=data.get("url", ""),
            headers=data.get("headers", {}),
            raw=data,
        )

    async def apresign_data(self, app_id: str, **kwargs: Any) -> PresignResult:
        """Async version of ``presign_data``.

        Args:
            app_id: The application identifier.
            **kwargs: Extra arguments forwarded to the underlying HTTP call.

        Returns:
            A ``PresignResult`` containing the presigned URL and headers.
        """
        resp = await self._arequest("POST", f"/apps/{app_id}/data/presign", **kwargs)
        data = resp.json()
        return PresignResult(
            url=data.get("url", ""),
            headers=data.get("headers", {}),
            raw=data,
        )


__all__ = ["FilesMixin"]
