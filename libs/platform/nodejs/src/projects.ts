import type { HttpClient, QueryParams } from "./client.js";
import { appPath, segment } from "./paths.js";
import type { JsonObject, Version, VersionType } from "./types.js";

/** Project resources. Complex documents retain their backend JSON field names. */
export function createProjectMethods(http: HttpClient) {
	return {
		listPages(app: string, boardId?: string): Promise<JsonObject[]> {
			return http.request("GET", `${appPath(app)}/pages`, {
				query: { board_id: boardId },
			});
		},
		getPage(
			app: string,
			id: string,
			options: { board_id?: string; version?: string } = {},
		): Promise<JsonObject> {
			return http.request("GET", `${appPath(app)}/pages/${segment(id)}`, {
				query: options,
			});
		},
		upsertPage(app: string, id: string, page: JsonObject): Promise<JsonObject> {
			return http.request("PUT", `${appPath(app)}/pages/${segment(id)}`, {
				body: { page },
			});
		},
		deletePage(app: string, id: string, boardId?: string): Promise<unknown> {
			return http.request("DELETE", `${appPath(app)}/pages/${segment(id)}`, {
				query: { board_id: boardId },
			});
		},
		getPageByRoute(app: string, route: string): Promise<JsonObject | null> {
			return http.request("GET", `${appPath(app)}/pages/by-route`, {
				query: { route },
			});
		},
		bootstrapPage(
			app: string,
			options: { route?: string; eventId?: string; __variant?: string } = {},
		): Promise<JsonObject> {
			return http.request("GET", `${appPath(app)}/pages/bootstrap`, {
				query: options,
			});
		},
		listWidgets(app: string, language?: string): Promise<JsonObject[]> {
			return http.request("GET", `${appPath(app)}/widgets`, {
				query: { language },
			});
		},
		getWidget(app: string, id: string, version?: string): Promise<JsonObject> {
			return http.request("GET", `${appPath(app)}/widgets/${segment(id)}`, {
				query: { version },
			});
		},
		upsertWidget(
			app: string,
			id: string,
			widget: JsonObject,
		): Promise<JsonObject> {
			return http.request("PUT", `${appPath(app)}/widgets/${segment(id)}`, {
				body: { widget },
			});
		},
		deleteWidget(app: string, id: string): Promise<unknown> {
			return http.request("DELETE", `${appPath(app)}/widgets/${segment(id)}`);
		},
		getWidgetVersions(app: string, id: string): Promise<Version[]> {
			return http.request(
				"GET",
				`${appPath(app)}/widgets/${segment(id)}/versions`,
			);
		},
		versionWidget(
			app: string,
			id: string,
			versionType: VersionType = "Patch",
		): Promise<Version> {
			return http.request(
				"POST",
				`${appPath(app)}/widgets/${segment(id)}/versions`,
				{ body: { version_type: versionType } },
			);
		},
		listRoutes(app: string): Promise<JsonObject[]> {
			return http.request("GET", `${appPath(app)}/routes`);
		},
		getRouteByPath(app: string, path: string): Promise<JsonObject | null> {
			return http.request("GET", `${appPath(app)}/routes/by-path`, {
				query: { path },
			});
		},
		getDefaultRoute(app: string): Promise<JsonObject | null> {
			return http.request("GET", `${appPath(app)}/routes/default`);
		},
		createRoute(
			app: string,
			body: { path: string; eventId: string; isDefault?: boolean },
		): Promise<JsonObject> {
			return http.request("POST", `${appPath(app)}/routes`, { body });
		},
		updateRoute(
			app: string,
			id: string,
			body: { eventId?: string; isDefault?: boolean },
		): Promise<JsonObject> {
			return http.request("PUT", `${appPath(app)}/routes/${segment(id)}`, {
				body,
			});
		},
		deleteRoute(app: string, id: string): Promise<unknown> {
			return http.request("DELETE", `${appPath(app)}/routes/${segment(id)}`);
		},
		listConnections(app: string): Promise<JsonObject> {
			return http.request("GET", `${appPath(app)}/connections`);
		},
		addConnection(
			app: string,
			sourceAppId: string,
			roleId: string,
		): Promise<unknown> {
			return http.request("POST", `${appPath(app)}/connections`, {
				body: { source_app_id: sourceAppId, role_id: roleId },
			});
		},
		requestConnection(
			app: string,
			targetAppId: string,
			comment?: string,
		): Promise<unknown> {
			return http.request("PUT", `${appPath(app)}/connections/request`, {
				body: { target_app_id: targetAppId, comment },
			});
		},
		updateConnection(
			app: string,
			id: string,
			roleId: string,
		): Promise<unknown> {
			return http.request("PUT", `${appPath(app)}/connections/${segment(id)}`, {
				body: { role_id: roleId },
			});
		},
		deleteConnection(app: string, id: string): Promise<unknown> {
			return http.request(
				"DELETE",
				`${appPath(app)}/connections/${segment(id)}`,
			);
		},
		acceptConnection(
			app: string,
			id: string,
			roleId: string,
		): Promise<unknown> {
			return http.request(
				"POST",
				`${appPath(app)}/connections/queue/${segment(id)}`,
				{ body: { role_id: roleId } },
			);
		},
		rejectConnection(app: string, id: string): Promise<unknown> {
			return http.request(
				"DELETE",
				`${appPath(app)}/connections/queue/${segment(id)}`,
			);
		},
		listAccessibleApps(
			app: string,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request("GET", `${appPath(app)}/connections/accessible`, {
				query: options,
			});
		},
		getConnectionGraph(
			app: string,
			options: QueryParams = {},
		): Promise<JsonObject> {
			return http.request("GET", `${appPath(app)}/connections/graph`, {
				query: options,
			});
		},
		getConnectedTables(
			app: string,
			target: string,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request(
				"GET",
				`${appPath(app)}/connections/${segment(target)}/tables`,
				{ query: options },
			);
		},
		getConnectedEvents(app: string, target: string): Promise<unknown> {
			return http.request(
				"GET",
				`${appPath(app)}/connections/${segment(target)}/events`,
			);
		},
		getConnectedEventDetail(
			app: string,
			target: string,
			event: string,
		): Promise<unknown> {
			return http.request(
				"GET",
				`${appPath(app)}/connections/${segment(target)}/events/${segment(event)}/detail`,
			);
		},
		createConnectionToken(
			app: string,
			target: string,
			body: { ttl_seconds?: number; run_id?: string } = {},
		): Promise<JsonObject> {
			return http.request(
				"POST",
				`${appPath(app)}/connections/${segment(target)}/token`,
				{ body },
			);
		},
		listPackages(app: string, language?: string): Promise<JsonObject[]> {
			return http.request("GET", `${appPath(app)}/packages`, {
				query: { language },
			});
		},
		addPackage(
			app: string,
			body: { packageId: string; version: string; autoUpdate: boolean },
			language?: string,
		): Promise<JsonObject> {
			return http.request("POST", `${appPath(app)}/packages`, {
				body,
				query: { language },
			});
		},
		updatePackage(
			app: string,
			id: string,
			body: { version?: string; autoUpdate?: boolean },
			language?: string,
		): Promise<JsonObject> {
			return http.request("PATCH", `${appPath(app)}/packages/${segment(id)}`, {
				body,
				query: { language },
			});
		},
		removePackage(app: string, id: string): Promise<unknown> {
			return http.request("DELETE", `${appPath(app)}/packages/${segment(id)}`);
		},
		getPackagePatchInfo(app: string, id: string): Promise<JsonObject> {
			return http.request(
				"GET",
				`${appPath(app)}/packages/${segment(id)}/patch-info`,
			);
		},
		getPackageUpdates(app: string): Promise<JsonObject[]> {
			return http.request("GET", `${appPath(app)}/packages/updates`);
		},
		reactivatePackage(
			app: string,
			id: string,
			language?: string,
		): Promise<JsonObject> {
			return http.request(
				"POST",
				`${appPath(app)}/packages/${segment(id)}/reactivate`,
				{ query: { language } },
			);
		},
		listRoles(app: string): Promise<[string | null, JsonObject[]]> {
			return http.request("GET", `${appPath(app)}/roles`);
		},
		getOwnRole(app: string): Promise<JsonObject> {
			return http.request("GET", `${appPath(app)}/roles/me`);
		},
		upsertRole(app: string, id: string, role: JsonObject): Promise<unknown> {
			return http.request("PUT", `${appPath(app)}/roles/${segment(id)}`, {
				body: role,
			});
		},
		deleteRole(app: string, id: string): Promise<unknown> {
			return http.request("DELETE", `${appPath(app)}/roles/${segment(id)}`);
		},
		setDefaultRole(app: string, id: string): Promise<unknown> {
			return http.request(
				"PUT",
				`${appPath(app)}/roles/${segment(id)}/default`,
			);
		},
		assignRole(app: string, id: string, userId: string): Promise<unknown> {
			return http.request(
				"POST",
				`${appPath(app)}/roles/${segment(id)}/assign/${segment(userId)}`,
			);
		},
		listTeam(app: string, language?: string): Promise<JsonObject[]> {
			return http.request("GET", `${appPath(app)}/team`, {
				query: { language },
			});
		},
		inviteUser(
			app: string,
			userId: string,
			message?: string,
		): Promise<unknown> {
			return http.request("PUT", `${appPath(app)}/team/invite`, {
				body: { sub: userId, message },
			});
		},
		removeTeamMember(app: string, userId: string): Promise<unknown> {
			return http.request("DELETE", `${appPath(app)}/team/${segment(userId)}`);
		},
		listInvites(app: string, language?: string): Promise<JsonObject[]> {
			return http.request("GET", `${appPath(app)}/team/invites`, {
				query: { language },
			});
		},
		revokeInvite(app: string, id: string): Promise<unknown> {
			return http.request(
				"DELETE",
				`${appPath(app)}/team/invites/${segment(id)}`,
			);
		},
		listInviteLinks(app: string): Promise<JsonObject[]> {
			return http.request("GET", `${appPath(app)}/team/link`);
		},
		createInviteLink(
			app: string,
			body: {
				name?: string;
				max_uses?: number;
				expires_in_hours?: number;
			} = {},
		): Promise<unknown> {
			return http.request("PUT", `${appPath(app)}/team/link`, { body });
		},
		deleteInviteLink(app: string, id: string): Promise<unknown> {
			return http.request("DELETE", `${appPath(app)}/team/link/${segment(id)}`);
		},
		joinInviteLink(app: string, token: string): Promise<unknown> {
			return http.request(
				"POST",
				`${appPath(app)}/team/link/join/${segment(token)}`,
			);
		},
		requestJoinApp(app: string, comment?: string): Promise<unknown> {
			return http.request("PUT", `${appPath(app)}/team/queue`, {
				body: { comment },
			});
		},
		listJoinRequests(app: string, language?: string): Promise<JsonObject[]> {
			return http.request("GET", `${appPath(app)}/team/queue`, {
				query: { language },
			});
		},
		acceptJoinRequest(app: string, id: string): Promise<unknown> {
			return http.request("POST", `${appPath(app)}/team/queue/${segment(id)}`);
		},
		rejectJoinRequest(app: string, id: string): Promise<unknown> {
			return http.request(
				"DELETE",
				`${appPath(app)}/team/queue/${segment(id)}`,
			);
		},
		listApiKeys(app: string): Promise<JsonObject[]> {
			return http.request("GET", `${appPath(app)}/api`);
		},
		createApiKey(
			app: string,
			body: {
				name: string;
				description?: string;
				role_id?: string;
				valid_until?: number;
			},
		): Promise<JsonObject> {
			return http.request("PUT", `${appPath(app)}/api`, { body });
		},
		deleteApiKey(app: string, id: string): Promise<unknown> {
			return http.request("DELETE", `${appPath(app)}/api/${segment(id)}`);
		},
		listTemplates(app: string, options: QueryParams = {}): Promise<unknown> {
			return http.request("GET", `${appPath(app)}/templates`, {
				query: options,
			});
		},
		getTemplate(
			app: string,
			id: string,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request("GET", `${appPath(app)}/templates/${segment(id)}`, {
				query: options,
			});
		},
		upsertTemplate(
			app: string,
			id: string,
			body: JsonObject,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request("PUT", `${appPath(app)}/templates/${segment(id)}`, {
				body,
				query: options,
			});
		},
		deleteTemplate(
			app: string,
			id: string,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request(
				"DELETE",
				`${appPath(app)}/templates/${segment(id)}`,
				{ query: options },
			);
		},
		getTemplatePreview(
			app: string,
			id: string,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request(
				"GET",
				`${appPath(app)}/templates/${segment(id)}/preview`,
				{ query: options },
			);
		},
		listGroups(app: string, options: QueryParams = {}): Promise<unknown> {
			return http.request("GET", `${appPath(app)}/groups`, { query: options });
		},
		createGroup(
			app: string,
			body: JsonObject,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request("POST", `${appPath(app)}/groups`, {
				body,
				query: options,
			});
		},
		getGroup(
			app: string,
			id: string,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request("GET", `${appPath(app)}/groups/${segment(id)}`, {
				query: options,
			});
		},
		updateGroup(
			app: string,
			id: string,
			body: JsonObject,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request("PUT", `${appPath(app)}/groups/${segment(id)}`, {
				body,
				query: options,
			});
		},
		deleteGroup(
			app: string,
			id: string,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request("DELETE", `${appPath(app)}/groups/${segment(id)}`, {
				query: options,
			});
		},
		setGroupVisibility(
			app: string,
			id: string,
			body: JsonObject,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request(
				"PATCH",
				`${appPath(app)}/groups/${segment(id)}/visibility`,
				{ body, query: options },
			);
		},
		getGroupPublication(
			app: string,
			id: string,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request(
				"GET",
				`${appPath(app)}/groups/${segment(id)}/publication`,
				{ query: options },
			);
		},
		addGroupMember(
			app: string,
			id: string,
			body: JsonObject,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request(
				"POST",
				`${appPath(app)}/groups/${segment(id)}/members`,
				{ body, query: options },
			);
		},
		removeGroupMember(
			app: string,
			id: string,
			member: string,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request(
				"DELETE",
				`${appPath(app)}/groups/${segment(id)}/members/${segment(member)}`,
				{ query: options },
			);
		},
		leaveGroup(
			app: string,
			id: string,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request(
				"DELETE",
				`${appPath(app)}/groups/${segment(id)}/membership`,
				{ query: options },
			);
		},
		listGroupRequests(
			app: string,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request("GET", `${appPath(app)}/groups/requests`, {
				query: options,
			});
		},
		acceptGroupRequest(
			app: string,
			id: string,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request(
				"POST",
				`${appPath(app)}/groups/requests/${segment(id)}`,
				{ query: options },
			);
		},
		declineGroupRequest(
			app: string,
			id: string,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request(
				"DELETE",
				`${appPath(app)}/groups/requests/${segment(id)}`,
				{ query: options },
			);
		},
		listGraphImports(app: string, options: QueryParams = {}): Promise<unknown> {
			return http.request("GET", `${appPath(app)}/graph/imports`, {
				query: options,
			});
		},
		sampleGraphImport(
			app: string,
			id: string,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request(
				"GET",
				`${appPath(app)}/graph/imports/${segment(id)}/sample`,
				{ query: options },
			);
		},
		queryGraphImport(
			app: string,
			id: string,
			body: JsonObject,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request(
				"POST",
				`${appPath(app)}/graph/imports/${segment(id)}/query`,
				{ body, query: options },
			);
		},
		listGraphs(app: string, options: QueryParams = {}): Promise<unknown> {
			return http.request("GET", `${appPath(app)}/graph`, { query: options });
		},
		createGraph(
			app: string,
			body: JsonObject,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request("POST", `${appPath(app)}/graph`, {
				body,
				query: options,
			});
		},
		getGraph(
			app: string,
			id: string,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request("GET", `${appPath(app)}/graph/${segment(id)}`, {
				query: options,
			});
		},
		updateGraph(
			app: string,
			id: string,
			body: JsonObject,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request("PUT", `${appPath(app)}/graph/${segment(id)}`, {
				body,
				query: options,
			});
		},
		deleteGraph(
			app: string,
			id: string,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request("DELETE", `${appPath(app)}/graph/${segment(id)}`, {
				query: options,
			});
		},
		getGraphSchema(
			app: string,
			id: string,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request(
				"GET",
				`${appPath(app)}/graph/${segment(id)}/schema`,
				{ query: options },
			);
		},
		validateGraph(
			app: string,
			id: string,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request(
				"POST",
				`${appPath(app)}/graph/${segment(id)}/validate`,
				{ query: options },
			);
		},
		queryGraphCypher(
			app: string,
			id: string,
			body: JsonObject,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request(
				"POST",
				`${appPath(app)}/graph/${segment(id)}/cypher`,
				{ body, query: options },
			);
		},
		queryGraphSql(
			app: string,
			id: string,
			body: JsonObject,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request("POST", `${appPath(app)}/graph/${segment(id)}/sql`, {
				body,
				query: options,
			});
		},
		getGraphNeighbors(
			app: string,
			id: string,
			body: JsonObject,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request(
				"POST",
				`${appPath(app)}/graph/${segment(id)}/neighbors`,
				{ body, query: options },
			);
		},
		getGraphChildren(
			app: string,
			id: string,
			body: JsonObject,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request(
				"POST",
				`${appPath(app)}/graph/${segment(id)}/children`,
				{ body, query: options },
			);
		},
		getSubgraph(
			app: string,
			id: string,
			body: JsonObject,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request(
				"POST",
				`${appPath(app)}/graph/${segment(id)}/subgraph`,
				{ body, query: options },
			);
		},
		findGraphPaths(
			app: string,
			id: string,
			body: JsonObject,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request(
				"POST",
				`${appPath(app)}/graph/${segment(id)}/paths`,
				{ body, query: options },
			);
		},
		getGraphAnalytics(
			app: string,
			id: string,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request(
				"GET",
				`${appPath(app)}/graph/${segment(id)}/analytics`,
				{ query: options },
			);
		},
		searchGraph(
			app: string,
			id: string,
			body: JsonObject,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request(
				"POST",
				`${appPath(app)}/graph/${segment(id)}/search`,
				{ body, query: options },
			);
		},
		sampleGraph(
			app: string,
			id: string,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request(
				"GET",
				`${appPath(app)}/graph/${segment(id)}/sample`,
				{ query: options },
			);
		},
		upsertGraphNodes(
			app: string,
			id: string,
			body: JsonObject,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request(
				"POST",
				`${appPath(app)}/graph/${segment(id)}/nodes`,
				{ body, query: options },
			);
		},
		upsertGraphEdges(
			app: string,
			id: string,
			body: JsonObject,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request(
				"POST",
				`${appPath(app)}/graph/${segment(id)}/edges`,
				{ body, query: options },
			);
		},
		updateGraphObject(
			app: string,
			id: string,
			body: JsonObject,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request(
				"PATCH",
				`${appPath(app)}/graph/${segment(id)}/objects`,
				{ body, query: options },
			);
		},
		updateGraphRelationship(
			app: string,
			id: string,
			body: JsonObject,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request(
				"PATCH",
				`${appPath(app)}/graph/${segment(id)}/relationships`,
				{ body, query: options },
			);
		},
		invokeOntologyAction(
			app: string,
			id: string,
			actionId: string,
			body: JsonObject,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request(
				"POST",
				`${appPath(app)}/graph/${segment(id)}/actions/${segment(actionId)}/invoke`,
				{ body, query: options },
			);
		},
		prerunOntologyAction(
			app: string,
			id: string,
			actionId: string,
			options: QueryParams = {},
		): Promise<unknown> {
			return http.request(
				"GET",
				`${appPath(app)}/graph/${segment(id)}/actions/${segment(actionId)}/prerun`,
				{ query: options },
			);
		},
	};
}
