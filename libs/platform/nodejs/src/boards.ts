import type { HttpClient } from "./client.js";
import type {
	Board,
	PrerunBoardResponse,
	UpsertBoardRequest,
	UpsertBoardResponse,
} from "./types.js";

export function createBoardMethods(http: HttpClient) {
	return {
		async listBoards(appId: string): Promise<Board[]> {
			return http.request<Board[]>("GET", `/apps/${appId}/board`);
		},

		async getBoard(
			appId: string,
			boardId: string,
			version?: string,
		): Promise<Board> {
			return http.request<Board>("GET", `/apps/${appId}/board/${boardId}`, {
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
				`/apps/${appId}/board/${boardId}`,
				{ body: data },
			);
		},

		async deleteBoard(appId: string, boardId: string): Promise<void> {
			await http.request("DELETE", `/apps/${appId}/board/${boardId}`);
		},

		async prerunBoard(
			appId: string,
			boardId: string,
			version?: string,
		): Promise<PrerunBoardResponse> {
			return http.request<PrerunBoardResponse>(
				"GET",
				`/apps/${appId}/board/${boardId}/prerun`,
				{ query: { version } },
			);
		},

		async getBoardVersions(appId: string, boardId: string): Promise<unknown[]> {
			return http.request<unknown[]>(
				"GET",
				`/apps/${appId}/board/${boardId}/version`,
			);
		},

		async versionBoard(appId: string, boardId: string): Promise<unknown> {
			return http.request("PATCH", `/apps/${appId}/board/${boardId}`);
		},

		async executeCommands(
			appId: string,
			boardId: string,
			commands: unknown[],
		): Promise<unknown[]> {
			return http.request<unknown[]>(
				"POST",
				`/apps/${appId}/board/${boardId}`,
				{ body: { commands } },
			);
		},
	};
}
