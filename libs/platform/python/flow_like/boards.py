"""Mixin for board CRUD, versioning, and execution endpoints."""

from __future__ import annotations

from typing import Any

from ._http import HTTPClient, segment
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
        resp = self._request("GET", f"/apps/{segment(app_id)}/board")
        data = resp.json()
        items = data if isinstance(data, list) else []
        return [Board(id=b.get("id", ""), raw=b) for b in items]

    async def alist_boards(self, app_id: str) -> list[Board]:
        """Async version of list_boards."""
        resp = await self._arequest("GET", f"/apps/{segment(app_id)}/board")
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
        resp = self._request("GET", f"/apps/{segment(app_id)}/board/{segment(board_id)}", params=params)
        data = resp.json()
        return Board(id=data.get("id", board_id), raw=data)

    async def aget_board(
        self, app_id: str, board_id: str, version: str | None = None
    ) -> Board:
        """Async version of get_board."""
        params = {"version": version} if version else None
        resp = await self._arequest(
            "GET", f"/apps/{segment(app_id)}/board/{segment(board_id)}", params=params
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
        resp = self._request("PUT", f"/apps/{segment(app_id)}/board/{segment(board_id)}", json=body)
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
            "PUT", f"/apps/{segment(app_id)}/board/{segment(board_id)}", json=body
        )
        data = resp.json()
        return UpsertBoardResponse(id=data.get("id", board_id), raw=data)

    def delete_board(self, app_id: str, board_id: str) -> None:
        """Delete a board.

        Args:
            app_id: Parent app identifier.
            board_id: Board to delete.
        """
        self._request("DELETE", f"/apps/{segment(app_id)}/board/{segment(board_id)}")

    async def adelete_board(self, app_id: str, board_id: str) -> None:
        """Async version of delete_board."""
        await self._arequest("DELETE", f"/apps/{segment(app_id)}/board/{segment(board_id)}")

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
            "GET", f"/apps/{segment(app_id)}/board/{segment(board_id)}/prerun", params=params
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
            "GET", f"/apps/{segment(app_id)}/board/{segment(board_id)}/prerun", params=params
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
        resp = self._request("GET", f"/apps/{segment(app_id)}/board/{segment(board_id)}/version")
        data = resp.json()
        return data if isinstance(data, list) else []

    async def aget_board_versions(
        self, app_id: str, board_id: str
    ) -> list[dict[str, Any]]:
        """Async version of get_board_versions."""
        resp = await self._arequest(
            "GET", f"/apps/{segment(app_id)}/board/{segment(board_id)}/version"
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
            f"/apps/{segment(app_id)}/board/{segment(board_id)}",
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
            f"/apps/{segment(app_id)}/board/{segment(board_id)}",
            json={"commands": commands},
        )
        data = resp.json()
        return data if isinstance(data, list) else []

    def version_board(self, app_id: str, board_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """PATCH /apps/{app_id}/board/{board_id}. Bodies and query keys follow the REST API."""
        return self._json("PATCH", f"/apps/{segment(app_id)}/board/{segment(board_id)}", params=params)

    async def aversion_board(self, app_id: str, board_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """PATCH /apps/{app_id}/board/{board_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("PATCH", f"/apps/{segment(app_id)}/board/{segment(board_id)}", params=params)

    def get_board_version_info(self, app_id: str, board_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/board/{board_id}/version/info. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/board/{segment(board_id)}/version/info", params=params)

    async def aget_board_version_info(self, app_id: str, board_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/board/{board_id}/version/info. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/board/{segment(board_id)}/version/info", params=params)

    def get_board_version_current(self, app_id: str, board_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/board/{board_id}/version/current. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/board/{segment(board_id)}/version/current", params=params)

    async def aget_board_version_current(self, app_id: str, board_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/board/{board_id}/version/current. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/board/{segment(board_id)}/version/current", params=params)

    def publish_board_if_changed(self, app_id: str, board_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/board/{board_id}/version/current. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/board/{segment(board_id)}/version/current", params=params)

    async def apublish_board_if_changed(self, app_id: str, board_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/board/{board_id}/version/current. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/board/{segment(board_id)}/version/current", params=params)

    def get_flowscript(self, app_id: str, board_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/board/{board_id}/flowscript. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/board/{segment(board_id)}/flowscript", params=params)

    async def aget_flowscript(self, app_id: str, board_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/board/{board_id}/flowscript. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/board/{segment(board_id)}/flowscript", params=params)

    def render_flowscript(self, app_id: str, board_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/board/{board_id}/flowscript/render. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/board/{segment(board_id)}/flowscript/render", json=body, params=params)

    async def arender_flowscript(self, app_id: str, board_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/board/{board_id}/flowscript/render. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/board/{segment(board_id)}/flowscript/render", json=body, params=params)

    def apply_flowscript(self, app_id: str, board_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/board/{board_id}/flowscript/apply. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/board/{segment(board_id)}/flowscript/apply", json=body, params=params)

    async def aapply_flowscript(self, app_id: str, board_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/board/{board_id}/flowscript/apply. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/board/{segment(board_id)}/flowscript/apply", json=body, params=params)

    def format_flowscript(self, app_id: str, board_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/board/{board_id}/flowscript/format. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/board/{segment(board_id)}/flowscript/format", json=body, params=params)

    async def aformat_flowscript(self, app_id: str, board_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/board/{board_id}/flowscript/format. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/board/{segment(board_id)}/flowscript/format", json=body, params=params)

    def sync_board(self, app_id: str, board_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/board/{board_id}/sync. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/board/{segment(board_id)}/sync", json=body, params=params)

    async def async_board(self, app_id: str, board_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/board/{board_id}/sync. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/board/{segment(board_id)}/sync", json=body, params=params)

    def undo_board(self, app_id: str, board_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PATCH /apps/{app_id}/board/{board_id}/undo. Bodies and query keys follow the REST API."""
        return self._json("PATCH", f"/apps/{segment(app_id)}/board/{segment(board_id)}/undo", json=body, params=params)

    async def aundo_board(self, app_id: str, board_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PATCH /apps/{app_id}/board/{board_id}/undo. Bodies and query keys follow the REST API."""
        return await self._ajson("PATCH", f"/apps/{segment(app_id)}/board/{segment(board_id)}/undo", json=body, params=params)

    def redo_board(self, app_id: str, board_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PATCH /apps/{app_id}/board/{board_id}/redo. Bodies and query keys follow the REST API."""
        return self._json("PATCH", f"/apps/{segment(app_id)}/board/{segment(board_id)}/redo", json=body, params=params)

    async def aredo_board(self, app_id: str, board_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PATCH /apps/{app_id}/board/{board_id}/redo. Bodies and query keys follow the REST API."""
        return await self._ajson("PATCH", f"/apps/{segment(app_id)}/board/{segment(board_id)}/redo", json=body, params=params)

    def get_board_capabilities(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/board/capabilities. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/board/capabilities", params=params)

    async def aget_board_capabilities(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/board/capabilities. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/board/capabilities", params=params)

    def get_board_summaries(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/board/summaries. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/board/summaries", params=params)

    async def aget_board_summaries(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/board/summaries. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/board/summaries", params=params)

    def get_board_variables(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/board/variables. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/board/variables", params=params)

    async def aget_board_variables(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/board/variables. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/board/variables", params=params)

    def get_board_workspace(self, app_id: str, board_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/board/{board_id}/workspace. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/board/{segment(board_id)}/workspace", params=params)

    async def aget_board_workspace(self, app_id: str, board_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/board/{board_id}/workspace. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/board/{segment(board_id)}/workspace", params=params)

    def list_nodes(self, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/nodes. Bodies and query keys follow the REST API."""
        return self._json("GET", "/apps/nodes", params=params)

    async def alist_nodes(self, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/nodes. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", "/apps/nodes", params=params)

    def list_app_nodes(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/nodes. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/nodes", params=params)

    async def alist_app_nodes(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/nodes. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/nodes", params=params)


__all__ = ["BoardsMixin"]
