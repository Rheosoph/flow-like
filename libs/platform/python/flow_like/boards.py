"""Mixin for board CRUD, versioning, and execution endpoints."""

from __future__ import annotations

from typing import Any

from ._http import HTTPClient
from ._types import Board, PrerunBoardResponse, UpsertBoardResponse


class BoardsMixin(HTTPClient):
    """HTTP methods for boards within an app."""

    def list_boards(self, app_id: str) -> list[Board]:
        """List all boards belonging to an app.

        Args:
            app_id: Parent app identifier.

        Returns:
            List of Board objects.
        """
        resp = self._request("GET", f"/apps/{app_id}/board")
        data = resp.json()
        items = data if isinstance(data, list) else []
        return [Board(id=b.get("id", ""), raw=b) for b in items]

    async def alist_boards(self, app_id: str) -> list[Board]:
        """Async version of list_boards."""
        resp = await self._arequest("GET", f"/apps/{app_id}/board")
        data = resp.json()
        items = data if isinstance(data, list) else []
        return [Board(id=b.get("id", ""), raw=b) for b in items]

    def get_board(
        self, app_id: str, board_id: str, version: str | None = None
    ) -> Board:
        """Fetch a single board, optionally at a specific version.

        Args:
            app_id: Parent app identifier.
            board_id: Board identifier.
            version: Optional version string.

        Returns:
            The matching Board.
        """
        params = {"version": version} if version else None
        resp = self._request("GET", f"/apps/{app_id}/board/{board_id}", params=params)
        data = resp.json()
        return Board(id=data.get("id", board_id), raw=data)

    async def aget_board(
        self, app_id: str, board_id: str, version: str | None = None
    ) -> Board:
        """Async version of get_board."""
        params = {"version": version} if version else None
        resp = await self._arequest(
            "GET", f"/apps/{app_id}/board/{board_id}", params=params
        )
        data = resp.json()
        return Board(id=data.get("id", board_id), raw=data)

    def upsert_board(
        self,
        app_id: str,
        board_id: str,
        *,
        name: str | None = None,
        description: str | None = None,
        stage: str | None = None,
        log_level: str | None = None,
        execution_mode: str | None = None,
        template: dict[str, Any] | None = None,
    ) -> UpsertBoardResponse:
        """Create or update a board's metadata.

        Args:
            app_id: Parent app identifier.
            board_id: Board identifier.
            name: Display name.
            description: Human-readable description.
            stage: Deployment stage.
            log_level: Logging verbosity.
            execution_mode: How the board should be executed.
            template: Optional board template dict.

        Returns:
            UpsertBoardResponse with the board's ID and raw payload.
        """
        body: dict[str, Any] = {}
        if name is not None:
            body["name"] = name
        if description is not None:
            body["description"] = description
        if stage is not None:
            body["stage"] = stage
        if log_level is not None:
            body["log_level"] = log_level
        if execution_mode is not None:
            body["execution_mode"] = execution_mode
        if template is not None:
            body["template"] = template
        resp = self._request("PUT", f"/apps/{app_id}/board/{board_id}", json=body)
        data = resp.json()
        return UpsertBoardResponse(id=data.get("id", board_id), raw=data)

    async def aupsert_board(
        self,
        app_id: str,
        board_id: str,
        *,
        name: str | None = None,
        description: str | None = None,
        stage: str | None = None,
        log_level: str | None = None,
        execution_mode: str | None = None,
        template: dict[str, Any] | None = None,
    ) -> UpsertBoardResponse:
        """Async version of upsert_board."""
        body: dict[str, Any] = {}
        if name is not None:
            body["name"] = name
        if description is not None:
            body["description"] = description
        if stage is not None:
            body["stage"] = stage
        if log_level is not None:
            body["log_level"] = log_level
        if execution_mode is not None:
            body["execution_mode"] = execution_mode
        if template is not None:
            body["template"] = template
        resp = await self._arequest(
            "PUT", f"/apps/{app_id}/board/{board_id}", json=body
        )
        data = resp.json()
        return UpsertBoardResponse(id=data.get("id", board_id), raw=data)

    def delete_board(self, app_id: str, board_id: str) -> None:
        """Delete a board.

        Args:
            app_id: Parent app identifier.
            board_id: Board to delete.
        """
        self._request("DELETE", f"/apps/{app_id}/board/{board_id}")

    async def adelete_board(self, app_id: str, board_id: str) -> None:
        """Async version of delete_board."""
        await self._arequest("DELETE", f"/apps/{app_id}/board/{board_id}")

    def prerun_board(
        self, app_id: str, board_id: str, version: str | None = None
    ) -> PrerunBoardResponse:
        """Retrieve pre-run information for a board.

        Args:
            app_id: Parent app identifier.
            board_id: Board identifier.
            version: Optional version string.

        Returns:
            PrerunBoardResponse with variables, OAuth needs, and execution info.
        """
        params = {"version": version} if version else None
        resp = self._request(
            "GET", f"/apps/{app_id}/board/{board_id}/prerun", params=params
        )
        data = resp.json()
        return PrerunBoardResponse(
            runtime_variables=data.get("runtime_variables", []),
            oauth_requirements=data.get("oauth_requirements", []),
            requires_local_execution=data.get("requires_local_execution", False),
            execution_mode=data.get("execution_mode", ""),
            can_execute_locally=data.get("can_execute_locally", False),
            raw=data,
        )

    async def aprerun_board(
        self, app_id: str, board_id: str, version: str | None = None
    ) -> PrerunBoardResponse:
        """Async version of prerun_board."""
        params = {"version": version} if version else None
        resp = await self._arequest(
            "GET", f"/apps/{app_id}/board/{board_id}/prerun", params=params
        )
        data = resp.json()
        return PrerunBoardResponse(
            runtime_variables=data.get("runtime_variables", []),
            oauth_requirements=data.get("oauth_requirements", []),
            requires_local_execution=data.get("requires_local_execution", False),
            execution_mode=data.get("execution_mode", ""),
            can_execute_locally=data.get("can_execute_locally", False),
            raw=data,
        )

    def get_board_versions(self, app_id: str, board_id: str) -> list[dict[str, Any]]:
        """List all versions of a board.

        Args:
            app_id: Parent app identifier.
            board_id: Board identifier.

        Returns:
            List of version dicts.
        """
        resp = self._request("GET", f"/apps/{app_id}/board/{board_id}/version")
        data = resp.json()
        return data if isinstance(data, list) else []

    async def aget_board_versions(
        self, app_id: str, board_id: str
    ) -> list[dict[str, Any]]:
        """Async version of get_board_versions."""
        resp = await self._arequest(
            "GET", f"/apps/{app_id}/board/{board_id}/version"
        )
        data = resp.json()
        return data if isinstance(data, list) else []

    def execute_commands(
        self, app_id: str, board_id: str, commands: list[dict[str, Any]]
    ) -> list[dict[str, Any]]:
        """Execute a batch of commands against a board.

        Args:
            app_id: Parent app identifier.
            board_id: Board identifier.
            commands: List of command dicts to execute.

        Returns:
            List of result dicts.
        """
        resp = self._request(
            "POST",
            f"/apps/{app_id}/board/{board_id}",
            json={"commands": commands},
        )
        data = resp.json()
        return data if isinstance(data, list) else []

    async def aexecute_commands(
        self, app_id: str, board_id: str, commands: list[dict[str, Any]]
    ) -> list[dict[str, Any]]:
        """Async version of execute_commands."""
        resp = await self._arequest(
            "POST",
            f"/apps/{app_id}/board/{board_id}",
            json={"commands": commands},
        )
        data = resp.json()
        return data if isinstance(data, list) else []


__all__ = ["BoardsMixin"]
