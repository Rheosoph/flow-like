import { afterEach, describe, expect, spyOn, test } from "bun:test";
import { FlowLikeClient, FlowLikeError } from "../src/index.js";
import { createHttpClient } from "../src/client.js";

const mocks: ReturnType<typeof spyOn>[] = [];
afterEach(() => {
	for (const mock of mocks.splice(0)) mock.mockRestore();
});
function transport(result: unknown = {}) {
	const mock = spyOn(globalThis, "fetch").mockImplementation(async () =>
		Response.json(result),
	);
	mocks.push(mock);
	return mock;
}
const client = () =>
	new FlowLikeClient({
		baseUrl: "https://flow.test/tenant/api/v1/",
		apiKey: "flk_platform_secret",
	});
function sent(mock: ReturnType<typeof transport>, index = 0) {
	const [url, options] = mock.mock.calls[index];
	return {
		url: String(url),
		method: options?.method,
		headers: new Headers(options?.headers),
		body:
			typeof options?.body === "string"
				? JSON.parse(options.body)
				: options?.body,
		options,
	};
}

describe("platform wire contracts", () => {
	test("new apps use PUT and a metadata envelope; no inherited models", async () => {
		const mock = transport({ id: "created" });
		await client().createApp("Invoices", "Parse PDFs");
		const request = sent(mock);
		expect(request.url).toBe("https://flow.test/tenant/api/v1/apps/new");
		expect(request.method).toBe("PUT");
		expect(request.body).toMatchObject({
			bits: [],
			meta: {
				name: "Invoices",
				description: "Parse PDFs",
				tags: [],
				preview_media: [],
			},
		});
		expect(request.body.meta.created_at.secs_since_epoch).toBeGreaterThan(0);
	});
	test("event invocation wraps object and scalar payloads, preserves options and uses /invoke/async", async () => {
		const mock = transport({
			run_id: "run",
			poll_token: "poll",
			status: "Pending",
		});
		const sdk = client();
		await sdk.triggerEventAsync(
			"app /?",
			"event #",
			{ input: 42 },
			{
				version: "1.2.3",
				variant: "candidate",
				profile_id: "profile",
				correlation: { invoice: "42" },
			},
		);
		expect(sent(mock)).toMatchObject({
			method: "POST",
			body: {
				payload: { input: 42 },
				version: "1.2.3",
				profile_id: "profile",
				correlation: { invoice: "42" },
			},
		});
		expect(sent(mock).url).toBe(
			"https://flow.test/tenant/api/v1/apps/app%20%2F%3F/events/event%20%23/invoke/async?__variant=candidate",
		);
		await sdk.triggerEventAsync("a", "e", 7);
		expect(sent(mock, 1).body).toEqual({ payload: 7 });
	});
	test("board invocation includes payload, pinned version and execution options", async () => {
		const mock = transport({ run_id: "r", poll_token: "p", status: "PENDING" });
		await client().triggerWorkflowAsync("a", "b", "start", "input", {
			version: [1, 2, 3],
			stream_state: false,
			profile_id: "profile",
			runtime_variables: { region: { value: "eu" } },
		});
		expect(sent(mock)).toMatchObject({
			method: "POST",
			url: "https://flow.test/tenant/api/v1/apps/a/board/b/invoke/async",
			body: {
				node_id: "start",
				payload: "input",
				version: [1, 2, 3],
				stream_state: false,
				profile_id: "profile",
				runtime_variables: { region: { value: "eu" } },
			},
		});
	});
	test("polling replaces platform credentials and advances the explicit cursor from returned events", async () => {
		const mock = transport({
			run_id: "r",
			status: "Running",
			events: [
				{ sequence: 0, event_type: "output", payload: 1, created_at: "now" },
				{ sequence: 4, event_type: "output", payload: 2, created_at: "now" },
			],
		});
		const sdk = client();
		const page = await sdk.pollExecution("private_poll_token", { timeout: 20 });
		expect(page.lastSequence).toBe(4);
		expect(page.events[0].payload).toBe(1);
		const request = sent(mock);
		expect(request.url).toBe(
			"https://flow.test/tenant/api/v1/execution/poll?after_sequence=-1&timeout=20",
		);
		expect(request.headers.get("authorization")).toBe(
			"Bearer private_poll_token",
		);
		expect(request.headers.has("x-api-key")).toBe(false);
		mock.mockResolvedValueOnce(
			Response.json({ run_id: "r", status: "Completed", events: [] }),
		);
		expect(
			(await sdk.pollExecution("private_poll_token", { afterSequence: 4 }))
				.lastSequence,
		).toBe(4);
	});
	test("database query pagination and reference selectors stay in query parameters", async () => {
		const mock = transport([]);
		await client().queryTable("a", "documents", {
			filter: "status = 'open'",
			limit: 10,
			offset: 20,
			scope: "user",
			branch: "release/1",
			version: 3,
			read_only: true,
			sql_params: { name: "O'Reilly" },
		});
		expect(sent(mock)).toMatchObject({
			method: "POST",
			body: { filter: "status = 'open'", sql_params: { name: "O'Reilly" } },
		});
		expect(sent(mock).url).toBe(
			"https://flow.test/tenant/api/v1/apps/a/db/documents/query?limit=10&offset=20&scope=user&branch=release%2F1&version=3&read_only=true",
		);
	});
	test("user-scoped Azure databases use the user SAS from mixed-provider content credentials", async () => {
		const azure = {
			Azure: {
				account_name: "account",
				content_container: "content",
				content_sas_token: "project-sas",
				user_content_sas_token: "user-sas",
			},
		};
		const mock = transport({
			shared_credentials: {
				Mixed: { meta: { Aws: {} }, content: azure, logs: { Gcp: {} } },
			},
			db_path: "users/sub/apps/a/db",
		});
		expect(await client().getDbCredentials("a")).toEqual({
			uri: "az://content/users/sub/apps/a/db",
			storageOptions: {
				azure_storage_account_name: "account",
				azure_storage_sas_token: "user-sas",
			},
		});
		expect(sent(mock).url).toBe(
			"https://flow.test/tenant/api/v1/apps/a/db/presign",
		);
		mock.mockResolvedValueOnce(
			Response.json({ shared_credentials: azure, db_path: "apps/a/db" }),
		);
		expect(
			(await client().getDbCredentials("a", "_default", "read", "project"))
				.storageOptions.azure_storage_sas_token,
		).toBe("project-sas");
		expect(sent(mock, 1).url).toBe(
			"https://flow.test/tenant/api/v1/apps/a/db/presign/project",
		);
	});
	test("database writes use root table routes and actual body keys; count is numeric", async () => {
		const mock = transport(null);
		const sdk = client();
		await sdk.addToTable("a", "t", [{ id: 1 }]);
		await sdk.deleteFromTable("a", "t", "id=1");
		expect(sent(mock)).toMatchObject({
			method: "PUT",
			url: "https://flow.test/tenant/api/v1/apps/a/db/t",
			body: { items: [{ id: 1 }] },
		});
		expect(sent(mock, 1)).toMatchObject({
			method: "DELETE",
			url: "https://flow.test/tenant/api/v1/apps/a/db/t",
			body: { query: "id=1" },
		});
		mock.mockResolvedValueOnce(Response.json(17));
		expect(await sdk.countItems("a", "t")).toBe(17);
	});
	test("board methods advertise supported format and carry sync, publication, and FlowScript payloads", async () => {
		const mock = transport({ version: [1, 2, 3], created: true });
		const sdk = client();
		expect(await sdk.publishBoardIfChanged("a", "b")).toEqual({
			version: [1, 2, 3],
			created: true,
		});
		expect(sent(mock).headers.get("x-flow-like-board-format")).toBe("2");
		await sdk.applyFlowScript("a", "b", "flow Test", {
			allow_deletions: true,
			scope_anchors: ["selected"],
			module: "main",
		});
		expect(sent(mock, 1).body).toEqual({
			flowscript: "flow Test",
			allow_deletions: true,
			scope_anchors: ["selected"],
			module: "main",
		});
		await sdk.executeCommandsWithSync("a", "b", [{ type: "AddNode" }], {
			revision: "v2",
		});
		expect(sent(mock, 2).body).toEqual({
			commands: [{ type: "AddNode" }],
			sync: { revision: "v2" },
		});
	});
});

describe("signed file transfers", () => {
	test("list uses POST and returns backend StorageItem rows", async () => {
		const rows = [
			{
				location: "folder/file.txt",
				size: 3,
				is_dir: false,
				last_modified: "now",
				e_tag: null,
				version: null,
			},
		];
		const mock = transport(rows);
		expect(
			await client().listFiles("a", {
				scope: "user",
				prefix: "folder",
				refresh: true,
			}),
		).toEqual(rows);
		expect(sent(mock)).toMatchObject({
			method: "POST",
			url: "https://flow.test/tenant/api/v1/apps/a/data/user/list?refresh=true",
			body: { prefix: "folder" },
		});
	});
	test("PUT upload reserves exact bytes and transfers without platform headers", async () => {
		const mock = transport();
		mock
			.mockResolvedValueOnce(
				Response.json([
					{
						prefix: "invoice.txt",
						url: "https://storage.test/object?sig=secret",
					},
				]),
			)
			.mockResolvedValueOnce(new Response(null, { status: 200 }));
		const signal = new AbortController().signal;
		await client().uploadFile("a", Buffer.from("hello"), {
			key: "invoice.txt",
			contentType: "text/plain",
			signal,
		});
		expect(sent(mock)).toMatchObject({
			method: "PUT",
			body: { prefixes: ["invoice.txt"], sizes: [5] },
		});
		const transfer = sent(mock, 1);
		expect(transfer.url).toBe("https://storage.test/object?sig=secret");
		expect(transfer.method).toBe("PUT");
		expect(transfer.headers.has("authorization")).toBe(false);
		expect(transfer.headers.has("x-api-key")).toBe(false);
		expect(transfer.options?.redirect).toBe("error");
		expect(transfer.options?.credentials).toBe("omit");
		expect(transfer.options?.signal).toBe(signal);
		expect(await (transfer.body as Blob).text()).toBe("hello");
	});
	test("S3 POST grants use supplied form fields with the file last", async () => {
		const mock = transport();
		mock
			.mockResolvedValueOnce(
				Response.json([
					{
						prefix: "file",
						url: "https://bucket.test",
						method: "POST",
						fields: {
							key: "scoped/file",
							policy: "signed",
							"x-amz-signature": "signature",
						},
					},
				]),
			)
			.mockResolvedValueOnce(new Response(null, { status: 204 }));
		await client().uploadFile("a", new Blob(["data"]), {
			key: "file",
			scope: "user",
		});
		expect(sent(mock).url).toBe(
			"https://flow.test/tenant/api/v1/apps/a/data/user",
		);
		const transfer = sent(mock, 1);
		expect(transfer.method).toBe("POST");
		expect(transfer.body).toBeInstanceOf(FormData);
		expect(Array.from((transfer.body as FormData).keys())).toEqual([
			"key",
			"policy",
			"x-amz-signature",
			"file",
		]);
		expect(transfer.headers.has("content-type")).toBe(false);
		expect(transfer.headers.has("x-api-key")).toBe(false);
	});
	test("download signs a prefix then fetches the signed URL, preserving binary response", async () => {
		const mock = transport();
		mock
			.mockResolvedValueOnce(
				Response.json([{ prefix: "f", url: "https://storage.test/f" }]),
			)
			.mockResolvedValueOnce(new Response(new Uint8Array([0, 255, 1])));
		const result = await client().downloadFile("a", "f");
		expect(new Uint8Array(await result.arrayBuffer())).toEqual(
			new Uint8Array([0, 255, 1]),
		);
		expect(sent(mock).body).toEqual({ prefixes: ["f"] });
		expect(sent(mock, 1).headers.has("x-api-key")).toBe(false);
	});
	test("signing errors are surfaced without attempting transfer", async () => {
		const mock = transport([{ prefix: "f", error: "Quota exceeded" }]);
		await expect(client().downloadFile("a", "f")).rejects.toThrow(
			"Quota exceeded",
		);
		expect(mock).toHaveBeenCalledTimes(1);
	});
	test("unsupported signed grants fail before sending file content", async () => {
		const mock = transport();
		for (const extra of [
			{ method: "PATCH" },
			{ method: "PUT", fields: { key: "f" } },
		]) {
			mock.mockResolvedValueOnce(
				Response.json([
					{ prefix: "f", url: "https://storage.test/f", ...extra },
				]),
			);
			await expect(
				client().uploadFile("a", new Blob(["private"]), { key: "f" }),
			).rejects.toThrow("Unsupported signed upload");
		}
		expect(mock).toHaveBeenCalledTimes(2);
	});
	test("data presign returns scoped credentials rather than a URL", async () => {
		const result = {
			shared_credentials: { Aws: {} },
			path: "apps/a/upload",
			access_mode: "read",
		};
		const mock = transport(result);
		expect(
			await client().presignData("a", {
				prefix: "folder",
				access_mode: "read",
			}),
		).toEqual(result);
		expect(sent(mock).body).toEqual({ prefix: "folder", access_mode: "read" });
	});
});

// Representative route contracts from packages/api/src/routes/app and devices.
const routeCases: {
	name: string;
	invoke: (sdk: FlowLikeClient) => Promise<unknown>;
	method: string;
	path: string;
	body?: unknown;
}[] = [
	{
		name: "app metadata",
		invoke: (s) => s.updateAppMeta("a", { name: "N" }, "de"),
		method: "PUT",
		path: "/apps/a/meta?language=de",
		body: { name: "N" },
	},
	{
		name: "visibility",
		invoke: (s) => s.setAppVisibility("a", "Private"),
		method: "PATCH",
		path: "/apps/a/visibility",
		body: { visibility: "Private" },
	},
	{
		name: "publication request",
		invoke: (s) => s.requestAppPublication("a", "Public", "Ready"),
		method: "POST",
		path: "/apps/a/publication/request",
		body: { target_visibility: "Public", message: "Ready" },
	},
	{
		name: "event save",
		invoke: (s) =>
			s.upsertEvent("a", "e", { id: "e" }, { register_source: false }),
		method: "PUT",
		path: "/apps/a/events/e",
		body: { event: { id: "e" }, register_source: false },
	},
	{
		name: "event validation",
		invoke: (s) => s.validateEvent("a", "e", "1.2.3"),
		method: "POST",
		path: "/apps/a/events/e/validate?version=1.2.3",
	},
	{
		name: "event setup",
		invoke: (s) => s.setupEvent("a", "e", { force: true }),
		method: "POST",
		path: "/apps/a/events/e/setup",
		body: { force: true },
	},
	{
		name: "event restore defaults to backend plan",
		invoke: (s) => s.restoreEvent("a", "e", [1, 2, 3]),
		method: "POST",
		path: "/apps/a/events/e/restore",
		body: { version: [1, 2, 3] },
	},
	{
		name: "canary shares",
		invoke: (s) => s.updateEventCanary("a", "e", { name: "v", weight: 0.1 }),
		method: "PATCH",
		path: "/apps/a/events/e/canary",
		body: { name: "v", weight: 0.1 },
	},
	{
		name: "schedules",
		invoke: (s) => s.listSchedules({ app_id: "a", limit: 25 }),
		method: "GET",
		path: "/user/schedules?app_id=a&limit=25",
	},
	{
		name: "run history",
		invoke: (s) => s.listRuns("a", "b", { offset: 20, include_nodes: true }),
		method: "GET",
		path: "/apps/a/board/b/runs?offset=20&include_nodes=true",
	},
	{
		name: "structured logs",
		invoke: (s) =>
			s.getRunLogs("a", "b", "r", {
				query: { nodes: ["n"], levels: [2, 3] },
				offset: 5,
				limit: 10,
			}),
		method: "POST",
		path: "/apps/a/board/b/logs/query",
		body: {
			run_id: "r",
			query: { nodes: ["n"], levels: [2, 3] },
			offset: 5,
			limit: 10,
		},
	},
	{
		name: "log summary",
		invoke: (s) => s.getRunSummary("a", "b", "r"),
		method: "GET",
		path: "/apps/a/board/b/logs/summary?run_id=r",
	},
	{
		name: "run payload",
		invoke: (s) => s.getRunPayload("a", "b", "r"),
		method: "GET",
		path: "/apps/a/board/b/runs/r/payload",
	},
	{
		name: "run cancel",
		invoke: (s) => s.cancelRun("r/?"),
		method: "DELETE",
		path: "/execution/run/r%2F%3F",
	},
	{
		name: "execution elements",
		invoke: (s) => s.getExecutionElements("a", "b", "page", { wildcard: true }),
		method: "GET",
		path: "/apps/a/board/b/elements?page_id=page&wildcard=true",
	},
	{
		name: "publish if changed",
		invoke: (s) => s.publishBoardIfChanged("a", "b"),
		method: "POST",
		path: "/apps/a/board/b/version/current",
	},
	{
		name: "board bump",
		invoke: (s) => s.versionBoard("a", "b", "Minor"),
		method: "PATCH",
		path: "/apps/a/board/b?version_type=Minor",
	},
	{
		name: "board undo",
		invoke: (s) => s.undoBoard("a", "b", [{ id: 1 }]),
		method: "PATCH",
		path: "/apps/a/board/b/undo",
		body: { commands: [{ id: 1 }] },
	},
	{
		name: "table listing",
		invoke: (s) => s.listTables("a"),
		method: "GET",
		path: "/apps/a/db",
	},
	{
		name: "user tables",
		invoke: (s) => s.listTables("a", { scope: "user" }),
		method: "GET",
		path: "/apps/a/db/user",
	},
	{
		name: "table creation",
		invoke: (s) =>
			s.createTable("a", "t", {
				fields: [{ name: "id", data_type: "string", primary_key: true }],
			}),
		method: "POST",
		path: "/apps/a/db/t",
		body: { fields: [{ name: "id", data_type: "string", primary_key: true }] },
	},
	{
		name: "drop table distinct from rows",
		invoke: (s) => s.dropTable("a", "t"),
		method: "DELETE",
		path: "/apps/a/db/t/table",
	},
	{
		name: "primary key",
		invoke: (s) => s.setTablePrimaryKey("a", "t", "id"),
		method: "PUT",
		path: "/apps/a/db/t/primary-key",
		body: { column: "id" },
	},
	{
		name: "column removal",
		invoke: (s) => s.dropTableColumns("a", "t", ["old"]),
		method: "DELETE",
		path: "/apps/a/db/t/columns",
		body: { columns: ["old"] },
	},
	{
		name: "index removal",
		invoke: (s) => s.dropTableIndex("a", "t", "index/name"),
		method: "DELETE",
		path: "/apps/a/db/t/index/index%2Fname",
	},
	{
		name: "safe optimize default",
		invoke: (s) => s.optimizeTable("a", "t"),
		method: "POST",
		path: "/apps/a/db/t/optimize",
		body: { keep_versions: true },
	},
	{
		name: "history",
		invoke: (s) => s.getTableHistory("a", "t", { branch: "release" }),
		method: "GET",
		path: "/apps/a/db/t/references?branch=release",
	},
	{
		name: "version tag",
		invoke: (s) => s.createTableTag("a", "t", "v1", { version: 5 }),
		method: "POST",
		path: "/apps/a/db/t/references?version=5",
		body: { action: "create_tag", name: "v1" },
	},
	{
		name: "saved query execution",
		invoke: (s) =>
			s.executeQuery("a", { sql: "SELECT $id", params: { id: 4 } }),
		method: "POST",
		path: "/apps/a/db/queries/execute",
		body: { sql: "SELECT $id", params: { id: 4 } },
	},
	{
		name: "page upsert envelope",
		invoke: (s) => s.upsertPage("a", "p", { id: "p" }),
		method: "PUT",
		path: "/apps/a/pages/p",
		body: { page: { id: "p" } },
	},
	{
		name: "page bootstrap casing",
		invoke: (s) => s.bootstrapPage("a", { eventId: "e", __variant: "v" }),
		method: "GET",
		path: "/apps/a/pages/bootstrap?eventId=e&__variant=v",
	},
	{
		name: "widget upsert envelope",
		invoke: (s) => s.upsertWidget("a", "w", { id: "w" }),
		method: "PUT",
		path: "/apps/a/widgets/w",
		body: { widget: { id: "w" } },
	},
	{
		name: "widget version",
		invoke: (s) => s.versionWidget("a", "w"),
		method: "POST",
		path: "/apps/a/widgets/w/versions",
		body: { version_type: "Patch" },
	},
	{
		name: "route body casing",
		invoke: (s) =>
			s.createRoute("a", { path: "/start", eventId: "e", isDefault: true }),
		method: "POST",
		path: "/apps/a/routes",
		body: { path: "/start", eventId: "e", isDefault: true },
	},
	{
		name: "connection request",
		invoke: (s) => s.requestConnection("a", "target", "Access"),
		method: "PUT",
		path: "/apps/a/connections/request",
		body: { target_app_id: "target", comment: "Access" },
	},
	{
		name: "package body casing",
		invoke: (s) =>
			s.addPackage("a", {
				packageId: "p",
				version: "1.2.3",
				autoUpdate: false,
			}),
		method: "POST",
		path: "/apps/a/packages",
		body: { packageId: "p", version: "1.2.3", autoUpdate: false },
	},
	{
		name: "role assignment",
		invoke: (s) => s.assignRole("a", "role", "user/sub"),
		method: "POST",
		path: "/apps/a/roles/role/assign/user%2Fsub",
	},
	{
		name: "team invite",
		invoke: (s) => s.inviteUser("a", "user", "Join"),
		method: "PUT",
		path: "/apps/a/team/invite",
		body: { sub: "user", message: "Join" },
	},
	{
		name: "API key creation",
		invoke: (s) => s.createApiKey("a", { name: "CI", role_id: "r" }),
		method: "PUT",
		path: "/apps/a/api",
		body: { name: "CI", role_id: "r" },
	},
	{
		name: "graph query",
		invoke: (s) =>
			s.queryGraphCypher(
				"a",
				"g",
				{ query: "MATCH (n) RETURN n" },
				{ scope: "user" },
			),
		method: "POST",
		path: "/apps/a/graph/g/cypher?scope=user",
		body: { query: "MATCH (n) RETURN n" },
	},
	{
		name: "device rename",
		invoke: (s) => s.renameDevice("d", null),
		method: "PATCH",
		path: "/devices/d",
		body: { display_name: null },
	},
	{
		name: "device signed policy",
		invoke: (s) => s.putDeviceManagementPolicy("d", "signed.policy"),
		method: "PUT",
		path: "/devices/d/management/policy",
		body: { policy_jws: "signed.policy" },
	},
	{
		name: "controller signaling",
		invoke: (s) => s.createControllerSignaling("d", "controller"),
		method: "POST",
		path: "/devices/d/signaling/controller",
		body: { participant_id: "controller" },
	},
	{
		name: "device resource grant",
		invoke: (s) => s.createDeviceResourceGrant("d", { placement_id: "p" }),
		method: "POST",
		path: "/devices/d/resource-grants",
		body: { placement_id: "p" },
	},
	{
		name: "schedule release",
		invoke: (s) => s.releaseDeviceSchedule("a", "e", "d", "p"),
		method: "PUT",
		path: "/apps/a/device-schedules/e",
		body: { device_id: "d", placement_id: "p" },
	},
];
for (const spec of routeCases)
	test(`route contract: ${spec.name}`, async () => {
		const mock = transport();
		await spec.invoke(client());
		const request = sent(mock);
		expect(request.url).toBe(`https://flow.test/tenant/api/v1${spec.path}`);
		expect(request.method).toBe(spec.method);
		expect(request.body).toEqual(spec.body);
	});

describe("transport behavior", () => {
	test("success with no content is accepted; plain text and structured errors survive", async () => {
		const mock = transport();
		mock
			.mockResolvedValueOnce(new Response(null, { status: 200 }))
			.mockResolvedValueOnce(
				new Response("Service unavailable", { status: 503 }),
			)
			.mockResolvedValueOnce(
				Response.json(
					{ error: { code: "DENIED", message: "Permission revoked" } },
					{ status: 403 },
				),
			);
		const sdk = client();
		expect(await sdk.deleteApp("a")).toBeUndefined();
		await expect(sdk.health()).rejects.toThrow("Service unavailable");
		const err = await sdk.health().catch((e) => e);
		expect(err).toBeInstanceOf(FlowLikeError);
		expect(err.statusCode).toBe(403);
		expect(err.body.error.code).toBe("DENIED");
	});
	test("base origins and /api/v1 prefixes normalize once; unsafe base URLs are rejected", async () => {
		const mock = transport();
		await new FlowLikeClient({
			baseUrl: "https://flow.test/prefix/",
			pat: "pat_test",
		}).health();
		expect(sent(mock).url).toBe("https://flow.test/prefix/api/v1/health");
		expect(sent(mock).options?.credentials).toBe("omit");
		for (const baseUrl of [
			"https://user:pass@flow.test",
			"https://flow.test?token=secret",
			"https://flow.test/#fragment",
			"file:///tmp/server",
		])
			expect(() => new FlowLikeClient({ baseUrl, pat: "pat_test" })).toThrow();
		expect(() => client().getApp("..")).toThrow();
		for (const path of [
			"/../elsewhere",
			"/%2e%2e/elsewhere",
			"/\\other.test/path",
		])
			await expect(client().request("GET", path)).rejects.toThrow(
				"API paths must be relative",
			);
	});
	test("a successful JSON invocation cannot silently become an empty stream", async () => {
		const mock = transport({ run_id: "r", status: "RUNNING" });
		const iterator = client()
			.triggerEvent("a", "e", { input: 1 })
			[Symbol.asyncIterator]();
		await expect(iterator.next()).rejects.toThrow(
			"Expected text/event-stream response",
		);
		expect(sent(mock).body).toEqual({ payload: { input: 1 } });
	});
	test("SSE preserves multiline data and whitespace across CRLF, UTF-8 fragments, and EOF", async () => {
		const mock = transport();
		const bytes = new TextEncoder().encode(
			"event: output\r\ndata: héllo\r\ndata:  spaced \r\n\r\ndata: final",
		);
		let index = 0;
		mock.mockResolvedValueOnce(
			new Response(
				new ReadableStream({
					pull(controller) {
						if (index === bytes.length) controller.close();
						else controller.enqueue(bytes.slice(index, (index += 1)));
					},
				}),
				{ headers: { "Content-Type": "text/event-stream; charset=utf-8" } },
			),
		);
		const http = createHttpClient("https://flow.test", {
			type: "pat",
			token: "pat_test",
		});
		const received = [];
		for await (const event of http.streamSSE("POST", "/stream"))
			received.push(event);
		expect(received).toEqual([
			{ event: "output", data: "héllo\n spaced ", id: undefined },
			{ event: undefined, data: "final", id: undefined },
		]);
	});
	test("leaving a stream cancels the underlying body reader", async () => {
		const mock = transport();
		let cancelled = false;
		mock.mockResolvedValueOnce(
			new Response(
				new ReadableStream({
					start(c) {
						c.enqueue(new TextEncoder().encode("data: one\n\n"));
					},
					cancel() {
						cancelled = true;
					},
				}),
				{ headers: { "Content-Type": "text/event-stream" } },
			),
		);
		const http = createHttpClient("https://flow.test", {
			type: "pat",
			token: "pat_test",
		});
		for await (const _event of http.streamSSE("POST", "/stream")) break;
		expect(cancelled).toBe(true);
	});
});
