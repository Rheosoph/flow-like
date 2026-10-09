import { appPath, boardPath } from "./paths.js";
import type { JsonObject, Version, VersionType } from "./types.js";
import type { HttpClient, QueryParams } from "./client.js";
import type {
	Board,
	PrerunBoardResponse,
	UpsertBoardRequest,
	UpsertBoardResponse,
} from "./types.js";

export function createBoardMethods(http: HttpClient) {
	return {
		async listBoards(appId: string): Promise<Board[]> {
			return http.request<Board[]>("GET", `${appPath(appId)}/board`);
		},

		async getBoard(
			appId: string,
			boardId: string,
			version?: string,
		): Promise<Board> {
			return http.request<Board>("GET", `${boardPath(appId, boardId)}`, {
				query: { version },
			});
		},

		async upsertBoard(
			appId: string,
			boardId: string,
			data: UpsertBoardRequest,
		): Promise<UpsertBoardResponse> {
			return http.request<UpsertBoardResponse>(
				"PUT",
				`${boardPath(appId, boardId)}`,
				{ body: data },
			);
		},

		async deleteBoard(appId: string, boardId: string): Promise<void> {
			await http.request("DELETE", `${boardPath(appId, boardId)}`);
		},

		async prerunBoard(
			appId: string,
			boardId: string,
			version?: string,
		): Promise<PrerunBoardResponse> {
			return http.request<PrerunBoardResponse>(
				"GET",
				`${boardPath(appId, boardId)}/prerun`,
				{ query: { version } },
			);
		},

		async getBoardVersions(appId: string, boardId: string): Promise<unknown[]> {
			return http.request<unknown[]>(
				"GET",
				`${boardPath(appId, boardId)}/version`,
			);
		},

		async versionBoard(
			appId: string,
			boardId: string,
			versionType: VersionType = "Patch",
		): Promise<Version> {
			return http.request("PATCH", `${boardPath(appId, boardId)}`, {
				query: { version_type: versionType },
			});
		},

		async executeCommands(
			appId: string,
			boardId: string,
			commands: unknown[],
		): Promise<unknown[]> {
			return http.request<unknown[]>("POST", `${boardPath(appId, boardId)}`, {
				body: { commands },
			});
		},
		getBoardCapabilities(
			appId: string,
		): Promise<{ board_format_version: number }> {
			return http.request("GET", `${appPath(appId)}/board/capabilities`);
		},
		listBoardSummaries(appId: string): Promise<JsonObject[]> {
			return http.request("GET", `${appPath(appId)}/board/summaries`);
		},
		listBoardVariables(
			appId: string,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request("GET", `${appPath(appId)}/board/variables`, {
				query: options,
			});
		},
		getBoardVersionInfos(
			appId: string,
			boardId: string,
		): Promise<JsonObject[]> {
			return http.request("GET", `${boardPath(appId, boardId)}/version/info`);
		},
		getBoardVersionCurrent(
			appId: string,
			boardId: string,
		): Promise<JsonObject> {
			return http.request(
				"GET",
				`${boardPath(appId, boardId)}/version/current`,
			);
		},
		publishBoardIfChanged(
			appId: string,
			boardId: string,
		): Promise<{ version: Version; created: boolean }> {
			return http.request(
				"POST",
				`${boardPath(appId, boardId)}/version/current`,
			);
		},
		getFlowScript(
			appId: string,
			boardId: string,
			options: {
				version?: string;
				anchors?: boolean;
				node_ids?: string;
				file?: string;
			} = {},
		): Promise<{ flowscript: string; scope_anchors?: string[] }> {
			return http.request("GET", `${boardPath(appId, boardId)}/flowscript`, {
				query: options,
			});
		},
		applyFlowScript(
			appId: string,
			boardId: string,
			flowscript: string,
			options: {
				current_layer?: string;
				allow_deletions?: boolean;
				origin?: string;
				scope_anchors?: string[];
				module?: string;
			} = {},
		): Promise<JsonObject> {
			return http.request(
				"POST",
				`${boardPath(appId, boardId)}/flowscript/apply`,
				{ body: { flowscript, ...options } },
			);
		},
		formatFlowScript(
			appId: string,
			boardId: string,
			flowscript: string,
			anchors?: boolean,
		): Promise<{ flowscript: string }> {
			return http.request(
				"POST",
				`${boardPath(appId, boardId)}/flowscript/format`,
				{ body: { flowscript, anchors } },
			);
		},
		renderFlowScript(
			appId: string,
			boardId: string,
			board: JsonObject,
			anchors?: boolean,
		): Promise<{ flowscript: string }> {
			return http.request(
				"POST",
				`${boardPath(appId, boardId)}/flowscript/render`,
				{ body: { board, anchors } },
			);
		},
		syncBoard(
			appId: string,
			boardId: string,
			request: JsonObject,
			version?: string,
		): Promise<JsonObject> {
			return http.request("POST", `${boardPath(appId, boardId)}/sync`, {
				body: request,
				query: { version },
			});
		},
		executeCommandsWithSync(
			appId: string,
			boardId: string,
			commands: unknown[],
			sync: JsonObject,
		): Promise<JsonObject> {
			return http.request("POST", boardPath(appId, boardId), {
				body: { commands, sync },
			});
		},
		async undoBoard(
			appId: string,
			boardId: string,
			commands: unknown[],
		): Promise<void> {
			await http.request("PATCH", `${boardPath(appId, boardId)}/undo`, {
				body: { commands },
			});
		},
		async redoBoard(
			appId: string,
			boardId: string,
			commands: unknown[],
		): Promise<void> {
			await http.request("PATCH", `${boardPath(appId, boardId)}/redo`, {
				body: { commands },
			});
		},
		listNodes(appId?: string): Promise<JsonObject[]> {
			return http.request(
				"GET",
				appId ? `${appPath(appId)}/nodes` : "/apps/nodes",
			);
		},
		getBoardWorkspace(
			appId: string,
			boardId: string,
			options: QueryParams = {},
		): Promise<JsonObject> {
			return http.request("GET", `${boardPath(appId, boardId)}/workspace`, {
				query: options,
			});
		},
	};
}
