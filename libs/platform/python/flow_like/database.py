"""Database operations mixin for the Flow-Like Python SDK."""

from __future__ import annotations

from typing import TYPE_CHECKING, Any

if TYPE_CHECKING:
    from lancedb.db import LanceDBConnection

from ._http import HTTPClient, segment, response_json
from ._types import (
    CountResult,
    LanceConnectionInfo,
    PresignDbAccessResponse,
    QueryResult,
    TableSchema,
)


def _presign_path(app_id: str, scope: str) -> str:
    if scope not in ("user", "project"):
        raise ValueError("Database scope must be 'user' or 'project'")
    return f"/apps/{segment(app_id)}/db/presign" + ("/project" if scope == "project" else "")


def _parse_presign_response(data: dict[str, Any]) -> PresignDbAccessResponse:
    """Convert a raw JSON dict into a ``PresignDbAccessResponse``."""
    return PresignDbAccessResponse(
        shared_credentials=data.get("shared_credentials", {}),
        db_path=data.get("db_path", ""),
        table_name=data.get("table_name", ""),
        access_mode=data.get("access_mode", "read"),
        expiration=data.get("expiration"),
        raw=data,
    )


def _resolve_connection_info(resp: PresignDbAccessResponse) -> LanceConnectionInfo:
    """Derive a ``LanceConnectionInfo`` from a presigned response."""
    creds = resp.shared_credentials
    while "Mixed" in creds:
        creds = creds["Mixed"]["content"]

    if "Aws" in creds:
        raw = creds["Aws"]
        cfg_raw = raw.get("content_config") or {}
        uri = f"s3://{raw['content_bucket']}/{resp.db_path}"
        opts: dict[str, str] = {}
        if raw.get("access_key_id"):
            opts["aws_access_key_id"] = raw["access_key_id"]
        if raw.get("secret_access_key"):
            opts["aws_secret_access_key"] = raw["secret_access_key"]
        if raw.get("session_token"):
            opts["aws_session_token"] = raw["session_token"]
        if raw.get("region"):
            opts["aws_region"] = raw["region"]
        if cfg_raw.get("endpoint"):
            opts["aws_endpoint"] = cfg_raw["endpoint"]
        return LanceConnectionInfo(uri=uri, storage_options=opts)

    if "Azure" in creds:
        raw = creds["Azure"]
        uri = f"az://{raw['content_container']}/{resp.db_path}"
        opts = {"azure_storage_account_name": raw["account_name"]}
        sas = (raw.get("user_content_sas_token") if resp.db_path.startswith("users/") else None) or raw.get("content_sas_token")
        if sas:
            opts["azure_storage_sas_token"] = sas
        if raw.get("account_key"):
            opts["azure_storage_account_key"] = raw["account_key"]
        return LanceConnectionInfo(uri=uri, storage_options=opts)

    if "Gcp" in creds:
        raw = creds["Gcp"]
        uri = f"gs://{raw['content_bucket']}/{resp.db_path}"
        opts = {}
        if raw.get("access_token"):
            opts["google_storage_token"] = raw["access_token"]
        elif raw.get("service_account_key"):
            opts["google_service_account_key"] = raw["service_account_key"]
        return LanceConnectionInfo(uri=uri, storage_options=opts)

    raise ValueError(f"Unknown shared credentials provider: {list(creds.keys())}")


class DatabaseMixin(HTTPClient):
    """Mixin providing database access and LanceDB integration."""

    def get_db_credentials(
        self,
        app_id: str,
        table_name: str = "_default",
        access_mode: str = "read",
        *, scope: str = "user",
    ) -> LanceConnectionInfo:
        """Obtain cloud-storage credentials for a LanceDB database.

        Args:
            app_id: Application identifier that owns the database.
            table_name: Logical table name to access.
            access_mode: ``"read"`` or ``"write"``.

        Returns:
            A ``LanceConnectionInfo`` with the URI and storage options.
        """
        resp = self._request(
            "POST",
            _presign_path(app_id, scope),
            json={"table_name": table_name, "access_mode": access_mode},
        )
        return _resolve_connection_info(_parse_presign_response(resp.json()))

    async def aget_db_credentials(
        self,
        app_id: str,
        table_name: str = "_default",
        access_mode: str = "read",
        *, scope: str = "user",
    ) -> LanceConnectionInfo:
        """Async version of ``get_db_credentials``."""
        resp = await self._arequest(
            "POST",
            _presign_path(app_id, scope),
            json={"table_name": table_name, "access_mode": access_mode},
        )
        return _resolve_connection_info(_parse_presign_response(resp.json()))

    def get_db_credentials_raw(
        self,
        app_id: str,
        table_name: str = "_default",
        access_mode: str = "read",
        *, scope: str = "user",
    ) -> PresignDbAccessResponse:
        """Obtain raw presigned database access credentials.

        Args:
            app_id: Application identifier that owns the database.
            table_name: Logical table name to access.
            access_mode: ``"read"`` or ``"write"``.

        Returns:
            A ``PresignDbAccessResponse`` with raw credential data.
        """
        resp = self._request(
            "POST",
            _presign_path(app_id, scope),
            json={"table_name": table_name, "access_mode": access_mode},
        )
        return _parse_presign_response(resp.json())

    async def aget_db_credentials_raw(
        self,
        app_id: str,
        table_name: str = "_default",
        access_mode: str = "read",
        *, scope: str = "user",
    ) -> PresignDbAccessResponse:
        """Async version of ``get_db_credentials_raw``."""
        resp = await self._arequest(
            "POST",
            _presign_path(app_id, scope),
            json={"table_name": table_name, "access_mode": access_mode},
        )
        return _parse_presign_response(resp.json())

    def create_lance_connection(
        self, app_id: str, access_mode: str = "read", *, scope: str = "user"
    ) -> LanceDBConnection:
        """Create a LanceDB connection for an application database.

        Args:
            app_id: Application identifier that owns the database.
            access_mode: ``"read"`` or ``"write"``.

        Returns:
            A ``LanceDBConnection`` ready for queries.

        Raises:
            ImportError: If the ``lancedb`` package is not installed.
        """
        try:
            import lancedb
        except ImportError as e:
            raise ImportError(
                "lancedb is required for create_lance_connection. "
                "Install it with: uv add flow-like[lance]"
            ) from e

        info = self.get_db_credentials(app_id, access_mode=access_mode, scope=scope)
        return lancedb.connect(info.uri, storage_options=info.storage_options)

    async def acreate_lance_connection(
        self, app_id: str, access_mode: str = "read", *, scope: str = "user"
    ) -> LanceDBConnection:
        """Async version of ``create_lance_connection``."""
        try:
            import lancedb
        except ImportError as e:
            raise ImportError(
                "lancedb is required for acreate_lance_connection. "
                "Install it with: uv add flow-like[lance]"
            ) from e

        info = await self.aget_db_credentials(app_id, access_mode=access_mode, scope=scope)
        return lancedb.connect(info.uri, storage_options=info.storage_options)

    def list_tables(self, app_id: str, *, params: dict[str, Any] | None = None) -> list[str]:
        """List all table names in an application database.

        Args:
            app_id: Application identifier that owns the database.

        Returns:
            A list of table name strings.
        """
        resp = self._request("GET", f"/apps/{segment(app_id)}/db", params=params)
        data = resp.json()
        return data if isinstance(data, list) else data.get("tables", [])

    async def alist_tables(self, app_id: str, *, params: dict[str, Any] | None = None) -> list[str]:
        """Async version of ``list_tables``."""
        resp = await self._arequest("GET", f"/apps/{segment(app_id)}/db", params=params)
        data = resp.json()
        return data if isinstance(data, list) else data.get("tables", [])

    def get_table_schema(self, app_id: str, table: str, *, params: dict[str, Any] | None = None) -> TableSchema:
        """Retrieve the schema of a database table.

        Args:
            app_id: Application identifier that owns the database.
            table: Name of the table.

        Returns:
            A ``TableSchema`` describing the table columns.
        """
        resp = self._request("GET", f"/apps/{segment(app_id)}/db/{segment(table)}/schema", params=params)
        data: dict[str, Any] = resp.json()
        return TableSchema(
            name=data.get("name", table),
            columns=data.get("fields", data.get("columns", [])),
            raw=data,
        )

    async def aget_table_schema(self, app_id: str, table: str, *, params: dict[str, Any] | None = None) -> TableSchema:
        """Async version of ``get_table_schema``."""
        resp = await self._arequest("GET", f"/apps/{segment(app_id)}/db/{segment(table)}/schema", params=params)
        data: dict[str, Any] = resp.json()
        return TableSchema(
            name=data.get("name", table),
            columns=data.get("fields", data.get("columns", [])),
            raw=data,
        )

    def query_table(self, app_id: str, table: str, query: dict[str, Any], *, params: dict[str, Any] | None = None) -> QueryResult:
        """Execute a query against a database table.

        Args:
            app_id: Application identifier that owns the database.
            table: Name of the table to query.
            query: Query parameters as a JSON-serializable dict.

        Returns:
            A ``QueryResult`` containing the matched rows.
        """
        resp = self._request("POST", f"/apps/{segment(app_id)}/db/{segment(table)}/query", json=query, params=params)
        data: Any = resp.json()
        if isinstance(data, list):
            return QueryResult(rows=data, raw={"rows": data})
        rows: list[dict[str, Any]] = data.get("rows", []) if isinstance(data, dict) else []
        return QueryResult(rows=rows, raw=data if isinstance(data, dict) else {"value": data})

    async def aquery_table(self, app_id: str, table: str, query: dict[str, Any], *, params: dict[str, Any] | None = None) -> QueryResult:
        """Async version of ``query_table``."""
        resp = await self._arequest("POST", f"/apps/{segment(app_id)}/db/{segment(table)}/query", json=query, params=params)
        data: Any = resp.json()
        if isinstance(data, list):
            return QueryResult(rows=data, raw={"rows": data})
        rows: list[dict[str, Any]] = data.get("rows", []) if isinstance(data, dict) else []
        return QueryResult(rows=rows, raw=data if isinstance(data, dict) else {"value": data})

    def add_to_table(self, app_id: str, table: str, data: list[dict[str, Any]], *, params: dict[str, Any] | None = None) -> dict[str, Any]:
        """Insert rows into a database table.

        Args:
            app_id: Application identifier that owns the database.
            table: Name of the target table.
            data: List of row dicts to insert.

        Returns:
            A dict with the API response (e.g. inserted count).
        """
        resp = self._request("PUT", f"/apps/{segment(app_id)}/db/{segment(table)}", json={"items": data}, params=params)
        return response_json(resp)

    async def aadd_to_table(
        self, app_id: str, table: str, data: list[dict[str, Any]], *, params: dict[str, Any] | None = None
    ) -> dict[str, Any]:
        """Async version of ``add_to_table``."""
        resp = await self._arequest("PUT", f"/apps/{segment(app_id)}/db/{segment(table)}", json={"items": data}, params=params)
        return response_json(resp)

    def delete_from_table(self, app_id: str, table: str, filter: str | dict[str, Any], *, params: dict[str, Any] | None = None) -> None:
        """Delete rows from a database table matching a filter.

        Args:
            app_id: Application identifier that owns the database.
            table: Name of the target table.
            filter: Filter criteria identifying rows to delete.
        """
        self._request("DELETE", f"/apps/{segment(app_id)}/db/{segment(table)}", json={"query": filter} if isinstance(filter, str) else filter, params=params)

    async def adelete_from_table(self, app_id: str, table: str, filter: str | dict[str, Any], *, params: dict[str, Any] | None = None) -> None:
        """Async version of ``delete_from_table``."""
        await self._arequest("DELETE", f"/apps/{segment(app_id)}/db/{segment(table)}", json={"query": filter} if isinstance(filter, str) else filter, params=params)

    def count_items(self, app_id: str, table: str, *, params: dict[str, Any] | None = None) -> CountResult:
        """Count the number of rows in a database table.

        Args:
            app_id: Application identifier that owns the database.
            table: Name of the table.

        Returns:
            A ``CountResult`` with the row count.
        """
        resp = self._request("GET", f"/apps/{segment(app_id)}/db/{segment(table)}/count", params=params)
        data = resp.json()
        return CountResult(count=data if isinstance(data, int) else data.get("count", 0), raw={"count": data} if isinstance(data, int) else data)

    async def acount_items(self, app_id: str, table: str, *, params: dict[str, Any] | None = None) -> CountResult:
        """Async version of ``count_items``."""
        resp = await self._arequest("GET", f"/apps/{segment(app_id)}/db/{segment(table)}/count", params=params)
        data = resp.json()
        return CountResult(count=data if isinstance(data, int) else data.get("count", 0), raw={"count": data} if isinstance(data, int) else data)

    def create_table(self, app_id: str, table: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/db/{table}. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/db/{segment(table)}", json=body, params=params)

    async def acreate_table(self, app_id: str, table: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/db/{table}. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/db/{segment(table)}", json=body, params=params)

    def drop_table(self, app_id: str, table: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/db/{table}/table. Bodies and query keys follow the REST API."""
        return self._json("DELETE", f"/apps/{segment(app_id)}/db/{segment(table)}/table", params=params)

    async def adrop_table(self, app_id: str, table: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/db/{table}/table. Bodies and query keys follow the REST API."""
        return await self._ajson("DELETE", f"/apps/{segment(app_id)}/db/{segment(table)}/table", params=params)

    def list_table_items(self, app_id: str, table: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/db/{table}. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/db/{segment(table)}", params=params)

    async def alist_table_items(self, app_id: str, table: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/db/{table}. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/db/{segment(table)}", params=params)

    def list_user_tables(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/db/user. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/db/user", params=params)

    async def alist_user_tables(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/db/user. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/db/user", params=params)

    def update_table(self, app_id: str, table: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/db/{table}/update. Bodies and query keys follow the REST API."""
        return self._json("PUT", f"/apps/{segment(app_id)}/db/{segment(table)}/update", json=body, params=params)

    async def aupdate_table(self, app_id: str, table: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/db/{table}/update. Bodies and query keys follow the REST API."""
        return await self._ajson("PUT", f"/apps/{segment(app_id)}/db/{segment(table)}/update", json=body, params=params)

    def optimize_table(self, app_id: str, table: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/db/{table}/optimize. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/db/{segment(table)}/optimize", json=body, params=params)

    async def aoptimize_table(self, app_id: str, table: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/db/{table}/optimize. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/db/{segment(table)}/optimize", json=body, params=params)

    def add_table_column(self, app_id: str, table: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/db/{table}/columns. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/db/{segment(table)}/columns", json=body, params=params)

    async def aadd_table_column(self, app_id: str, table: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/db/{table}/columns. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/db/{segment(table)}/columns", json=body, params=params)

    def alter_table_column(self, app_id: str, table: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/db/{table}/columns. Bodies and query keys follow the REST API."""
        return self._json("PUT", f"/apps/{segment(app_id)}/db/{segment(table)}/columns", json=body, params=params)

    async def aalter_table_column(self, app_id: str, table: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/db/{table}/columns. Bodies and query keys follow the REST API."""
        return await self._ajson("PUT", f"/apps/{segment(app_id)}/db/{segment(table)}/columns", json=body, params=params)

    def drop_table_columns(self, app_id: str, table: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/db/{table}/columns. Bodies and query keys follow the REST API."""
        return self._json("DELETE", f"/apps/{segment(app_id)}/db/{segment(table)}/columns", json=body, params=params)

    async def adrop_table_columns(self, app_id: str, table: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/db/{table}/columns. Bodies and query keys follow the REST API."""
        return await self._ajson("DELETE", f"/apps/{segment(app_id)}/db/{segment(table)}/columns", json=body, params=params)

    def set_table_primary_key(self, app_id: str, table: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/db/{table}/primary-key. Bodies and query keys follow the REST API."""
        return self._json("PUT", f"/apps/{segment(app_id)}/db/{segment(table)}/primary-key", json=body, params=params)

    async def aset_table_primary_key(self, app_id: str, table: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/db/{table}/primary-key. Bodies and query keys follow the REST API."""
        return await self._ajson("PUT", f"/apps/{segment(app_id)}/db/{segment(table)}/primary-key", json=body, params=params)

    def build_table_index(self, app_id: str, table: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/db/{table}/index. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/db/{segment(table)}/index", json=body, params=params)

    async def abuild_table_index(self, app_id: str, table: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/db/{table}/index. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/db/{segment(table)}/index", json=body, params=params)

    def drop_table_index(self, app_id: str, table: str, index_name: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/db/{table}/index/{index_name}. Bodies and query keys follow the REST API."""
        return self._json("DELETE", f"/apps/{segment(app_id)}/db/{segment(table)}/index/{segment(index_name)}", params=params)

    async def adrop_table_index(self, app_id: str, table: str, index_name: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/db/{table}/index/{index_name}. Bodies and query keys follow the REST API."""
        return await self._ajson("DELETE", f"/apps/{segment(app_id)}/db/{segment(table)}/index/{segment(index_name)}", params=params)

    def get_table_indices(self, app_id: str, table: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/db/{table}/indices. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/db/{segment(table)}/indices", params=params)

    async def aget_table_indices(self, app_id: str, table: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/db/{table}/indices. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/db/{segment(table)}/indices", params=params)

    def get_table_view(self, app_id: str, table: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/db/{table}/view. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/db/{segment(table)}/view", params=params)

    async def aget_table_view(self, app_id: str, table: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/db/{table}/view. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/db/{segment(table)}/view", params=params)

    def get_table_history(self, app_id: str, table: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/db/{table}/references. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/db/{segment(table)}/references", params=params)

    async def aget_table_history(self, app_id: str, table: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/db/{table}/references. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/db/{segment(table)}/references", params=params)

    def table_reference_action(self, app_id: str, table: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/db/{table}/references. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/db/{segment(table)}/references", json=body, params=params)

    async def atable_reference_action(self, app_id: str, table: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/db/{table}/references. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/db/{segment(table)}/references", json=body, params=params)

    def compare_table(self, app_id: str, table: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/db/{table}/compare. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/db/{segment(table)}/compare", json=body, params=params)

    async def acompare_table(self, app_id: str, table: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/db/{table}/compare. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/db/{segment(table)}/compare", json=body, params=params)

    def list_saved_queries(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/db/queries. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/db/queries", params=params)

    async def alist_saved_queries(self, app_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/db/queries. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/db/queries", params=params)

    def get_saved_query(self, app_id: str, query_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/db/queries/{query_id}. Bodies and query keys follow the REST API."""
        return self._json("GET", f"/apps/{segment(app_id)}/db/queries/{segment(query_id)}", params=params)

    async def aget_saved_query(self, app_id: str, query_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """GET /apps/{app_id}/db/queries/{query_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("GET", f"/apps/{segment(app_id)}/db/queries/{segment(query_id)}", params=params)

    def create_saved_query(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/db/queries. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/db/queries", json=body, params=params)

    async def acreate_saved_query(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/db/queries. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/db/queries", json=body, params=params)

    def update_saved_query(self, app_id: str, query_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/db/queries/{query_id}. Bodies and query keys follow the REST API."""
        return self._json("PUT", f"/apps/{segment(app_id)}/db/queries/{segment(query_id)}", json=body, params=params)

    async def aupdate_saved_query(self, app_id: str, query_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """PUT /apps/{app_id}/db/queries/{query_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("PUT", f"/apps/{segment(app_id)}/db/queries/{segment(query_id)}", json=body, params=params)

    def delete_saved_query(self, app_id: str, query_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/db/queries/{query_id}. Bodies and query keys follow the REST API."""
        return self._json("DELETE", f"/apps/{segment(app_id)}/db/queries/{segment(query_id)}", params=params)

    async def adelete_saved_query(self, app_id: str, query_id: str, *, params: dict[str, Any] | None = None) -> Any:
        """DELETE /apps/{app_id}/db/queries/{query_id}. Bodies and query keys follow the REST API."""
        return await self._ajson("DELETE", f"/apps/{segment(app_id)}/db/queries/{segment(query_id)}", params=params)

    def execute_query(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/db/queries/execute. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/db/queries/execute", json=body, params=params)

    async def aexecute_query(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/db/queries/execute. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/db/queries/execute", json=body, params=params)

    def presign_project_database(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/db/presign/project. Bodies and query keys follow the REST API."""
        return self._json("POST", f"/apps/{segment(app_id)}/db/presign/project", json=body, params=params)

    async def apresign_project_database(self, app_id: str, body: dict[str, Any], *, params: dict[str, Any] | None = None) -> Any:
        """POST /apps/{app_id}/db/presign/project. Bodies and query keys follow the REST API."""
        return await self._ajson("POST", f"/apps/{segment(app_id)}/db/presign/project", json=body, params=params)


__all__ = ["DatabaseMixin"]
