import { describe, expect, it, vi } from "vitest";
import type { IBackendState } from "../../state/backend-state";
import type { IPage } from "../../state/backend-state/page-state";
import {
	type WorkspaceScope,
	collectWorkspaceDocuments,
	readWorkspaceDocument,
} from "./workspace-content";
import { loadFlowPilotWorkspaceDocs } from "./workspace-docs";
import {
	type WorkspaceKind,
	type WorkspaceTarget,
	jsonBytes,
	parseWorkspaceResourceId,
	workspaceResourceId,
} from "./workspace-resource";
import {
	WorkspaceSearchSession,
	readWorkspaceSymbol,
} from "./workspace-search";

function fixture() {
	const sources: Record<string, string> = {
		"board-billing": `interface Invoice { id: string; customer_id: string; }
const localConfiguration = "WORKFLOW_GLOBAL_VALUE_SHOULD_NOT_BE_INDEXED"
module billing { //@l:billing-module
    function loadInvoices(customer: string): (invoices: Invoice[]) { //@l:invoice-helper
        const stored = db::open({ name: "customer_receivables", userScoped: false })
        const rows = stored.query({ filter: "customer_account_reference", customer: customer })
        return rows
    }
    eventsSimple refreshInvoices() { //@n:refresh-invoices
        billing::loadInvoices({ customer: "current_customer" })
    }
}`,
		"board-review": `eventsSimple reviewInvoice() { //@n:review-invoice
    remote::callEvent({ eventId: "event-invoices", payload: { customer_id: "review_customer" } })
}`,
	};
	const event = {
		id: "event-invoices",
		name: "Load invoices",
		description: "Find outstanding customer invoices",
		board_id: "board-billing",
		board_version: [2, 1, 0],
		node_id: "refresh-invoices",
		event_type: "generic",
		route: "/invoices",
		default_page_id: "page-invoices",
		config: [81, 71, 91],
		variables: {
			credential: {
				secret: true,
				default_value: "EVENT_VARIABLE_SHOULD_NOT_BE_INDEXED",
			},
		},
		inputs: [
			{
				id: "customer-pin",
				name: "customer_id",
				friendly_name: "Customer account",
				description: "Customer identifier for the invoice query",
				data_type: "String",
				value_type: "Normal",
				default_value: "EVENT_DEFAULT_SHOULD_NOT_BE_INDEXED",
			},
		],
	};
	const page: IPage = {
		id: "page-invoices",
		name: "Invoice review",
		boardId: "board-billing",
		onLoadEventId: "refresh-invoices",
		onUnloadEventId: "close-invoices",
		onIntervalEventId: "poll-invoices",
		onIntervalSeconds: 30,
		route: "/invoices",
		createdAt: "2026-01-01T00:00:00Z",
		updatedAt: "2026-01-02T00:00:00Z",
		layoutType: "stack",
		content: [],
		components: [
			{
				id: "refresh-button",
				component: {
					id: "refresh-button",
					type: "button",
					label: {
						path: "/invoices/refresh_label",
						defaultValue: { path: "BOUND_DEFAULT_SHOULD_NOT_BE_INDEXED" },
					},
					eventHandlers: {
						onClick: [
							{
								name: "run_workflow",
								context: {
									eventId: "event-invoices",
									boardId: "board-billing",
									nodeId: "refresh-invoices",
									parameters: {
										path: "ACTION_CONTEXT_SHOULD_NOT_BE_INDEXED",
									},
								},
								pageAction: {
									actionId: "refresh-action",
									manifestRevision: "manifest-r2",
									capabilityJwt: "PAGE_CAPABILITY_SHOULD_NOT_BE_INDEXED",
								},
							},
						],
					},
				},
			},
		],
	};
	const schemas: Record<string, unknown> = {
		customer_receivables: {
			fields: [
				{
					name: "customer_account_reference",
					data_type: "Utf8",
					nullable: false,
					default: "SCHEMA_DEFAULT_SHOULD_NOT_BE_INDEXED",
					metadata: { path: "SCHEMA_METADATA_SHOULD_NOT_BE_INDEXED" },
				},
				{ name: "amount_due", data_type: "Float64", nullable: false },
			],
			rows: [{ customer_account_reference: "TABLE_ROW_SHOULD_NOT_BE_INDEXED" }],
		},
	};
	const forbidden = Object.fromEntries(
		[
			"getBoard",
			"getBoardAuthoritative",
			"getBoardSummaries",
			"getBoardVariables",
			"getFlowScript",
			"getEvents",
			"getEvent",
			"getPages",
			"getPage",
			"getSchema",
			"listTables",
			"getItems",
		].map((name) => [
			name,
			vi.fn(() => {
				throw new Error(`Unexpected backend method: ${name}`);
			}),
		]),
	);
	const reads = {
		getBoardSummariesAuthoritative: vi.fn(async () => [
			{ id: "board-billing" },
			{ id: "board-review" },
		]),
		getFlowScriptAuthoritative: vi.fn(
			async (_app: string, board: string) => sources[board],
		),
		getEventsAuthoritative: vi.fn(async () => [event]),
		getEventAuthoritative: vi.fn(async () => event),
		getPagesAuthoritative: vi.fn(async () => [
			{ pageId: page.id, boardId: page.boardId },
		]),
		getPageAuthoritative: vi.fn(async () => page),
		listTablesAuthoritative: vi.fn(async () => Object.keys(schemas)),
		getSchemaAuthoritative: vi.fn(
			async (_app: string, table: string) => schemas[table],
		),
	};
	const backend = {
		boardState: { ...forbidden, ...reads },
		eventState: { ...forbidden, ...reads },
		pageState: { ...forbidden, ...reads },
		dbState: { ...forbidden, ...reads },
	} as unknown as IBackendState;
	const scope: WorkspaceScope = {
		getProfileAppIds: vi.fn(async () => new Set(["app-invoices"])),
	};
	return { backend, scope, reads, forbidden, sources, event, page, schemas };
}

describe("FlowPilot workspace content collection", () => {
	it("searches nested table fields without carrying their metadata into exact reads", async () => {
		const { backend, scope, schemas } = fixture();
		schemas.customer_receivables = {
			fields: [
				{
					name: "invoice",
					data_type: {
						Struct: [
							{
								name: "nested_account_reference",
								data_type: "Utf8",
								nullable: false,
								metadata: { source: "PRIVATE_FIELD_METADATA" },
								default: "PRIVATE_FIELD_DEFAULT",
							},
						],
					},
				},
			],
		};
		const result = await new WorkspaceSearchSession().search(
			backend,
			{
				query: "nested_account_reference",
				app_id: "app-invoices",
				kinds: ["table"],
			},
			scope,
		);
		expect(result.coverage).toMatchObject({ complete: true });
		const hits = result.hits as Array<{
			resource_id: string;
			revision: string;
		}>;
		expect(hits).toHaveLength(1);
		const read = await readWorkspaceSymbol(backend, hits[0], scope);
		expect(read.status).toBe("ok");
		expect(read.content).toContain("nested_account_reference");
		expect(read.content).not.toContain("PRIVATE_FIELD");
	});

	it("collects helper bodies and connected app contracts using authoritative reads", async () => {
		const { backend, scope, reads, forbidden } = fixture();
		const result = await collectWorkspaceDocuments(
			backend,
			{ app_id: "app-invoices", kinds: ["workflow", "event", "page", "table"] },
			scope,
		);
		expect(result.coverage.complete).toBe(true);
		expect(result.coverage.app_ids).toEqual(["app-invoices"]);
		expect(result.coverage.indexed_resources).toBe(7);
		const helper = result.documents.find(
			(document) => document.symbol === "billing::loadInvoices",
		);
		expect(helper).toMatchObject({
			target: {
				kind: "workflow",
				app_id: "app-invoices",
				board_id: "board-billing",
				id: "billing::loadInvoices",
			},
			anchor_id: "invoice-helper",
		});
		expect(helper?.content).toContain("customer_account_reference");
		expect(helper?.signature).not.toContain("customer_account_reference");
		const review = result.documents.find(
			(document) => document.symbol === "reviewInvoice",
		);
		expect(review?.content).toContain('eventId: "event-invoices"');
		expect(review?.target.board_id).toBe("board-review");
		const event = result.documents.find(
			(document) => document.target.kind === "event",
		);
		expect(JSON.parse(event?.content ?? "{}")).toMatchObject({
			id: "event-invoices",
			board_id: "board-billing",
			node_id: "refresh-invoices",
			default_page_id: "page-invoices",
			inputs: [{ name: "customer_id", data_type: "String" }],
		});
		const page = result.documents.find(
			(document) => document.target.kind === "page",
		);
		expect(page?.content).toContain("refresh-invoices");
		expect(page?.content).toContain("event-invoices");
		expect(page?.content).toContain("refresh-action");
		expect(page?.content).toContain("/invoices/refresh_label");
		const table = result.documents.find(
			(document) => document.target.kind === "table",
		);
		expect(table?.content).toContain("customer_account_reference");
		expect(table?.content).toContain("Float64");
		const serialized = JSON.stringify(result.documents);
		expect(serialized).not.toContain("SHOULD_NOT_BE_INDEXED");
		expect(event?.content).not.toContain('"config"');
		expect(reads.getFlowScriptAuthoritative).toHaveBeenCalledWith(
			"app-invoices",
			"board-billing",
			undefined,
			true,
		);
		expect(reads.getSchemaAuthoritative).toHaveBeenCalledWith(
			"app-invoices",
			"customer_receivables",
			false,
		);
		for (const method of Object.values(forbidden))
			expect(method).not.toHaveBeenCalled();
	});

	it("keeps the actual component type in the searchable page contract", async () => {
		const { backend, scope } = fixture();
		const document = await readWorkspaceDocument(
			backend,
			{
				kind: "page",
				app_id: "app-invoices",
				board_id: "board-billing",
				id: "page-invoices",
			},
			scope,
		);
		const contract = JSON.parse(document.content);
		expect(JSON.stringify(contract.components)).toContain('"button"');
	});

	it("includes structured Event input fields without schema defaults or examples", async () => {
		const { backend, scope, event } = fixture();
		Object.assign(event.inputs[0], {
			data_type: "Struct",
			schema: JSON.stringify({
				type: "object",
				properties: {
					account_reference: {
						type: "string",
						default: "INPUT_SCHEMA_DEFAULT_SHOULD_NOT_BE_INDEXED",
					},
					invoice_lines: {
						type: "array",
						items: {
							type: "number",
							examples: ["INPUT_SCHEMA_EXAMPLE_SHOULD_NOT_BE_INDEXED"],
						},
					},
				},
				required: ["account_reference"],
			}),
		});
		const result = await collectWorkspaceDocuments(
			backend,
			{ kinds: ["event"] },
			scope,
		);
		expect(result.coverage.complete).toBe(true);
		expect(result.documents[0].content).toContain("account_reference");
		expect(result.documents[0].content).toContain("invoice_lines");
		expect(result.documents[0].content).not.toContain("SHOULD_NOT_BE_INDEXED");
	});

	it("projects the optional flag of Event inputs but never their default values", async () => {
		const { backend, scope, event } = fixture();
		event.inputs.push({
			...event.inputs[0],
			id: "amount-pin",
			name: "amount",
			friendly_name: "Amount",
			description: "Invoice amount",
			data_type: "Float",
			default_value: "EVENT_AMOUNT_DEFAULT_SHOULD_NOT_BE_INDEXED",
		});
		Object.assign(event.inputs[0], { optional: true });
		const result = await collectWorkspaceDocuments(
			backend,
			{ kinds: ["event"] },
			scope,
		);
		expect(result.coverage.complete).toBe(true);
		const contract = JSON.parse(result.documents[0].content);
		expect(contract.inputs).toEqual([
			expect.objectContaining({ name: "customer_id", optional: true }),
			expect.objectContaining({ name: "amount", data_type: "Float" }),
		]);
		expect(contract.inputs[1]).not.toHaveProperty("optional");
		for (const input of contract.inputs)
			expect(input).not.toHaveProperty("default_value");
		expect(result.documents[0].content).not.toContain("SHOULD_NOT_BE_INDEXED");
	});

	it("does not treat object-valued action context as a routing identifier", async () => {
		const { backend, scope, page } = fixture();
		const action = page.components[0].component.eventHandlers?.onClick[0];
		if (!action) throw new Error("Expected an action fixture");
		action.context.eventId = { secret: "NESTED_CONTEXT_SHOULD_NOT_BE_INDEXED" };
		const result = await collectWorkspaceDocuments(
			backend,
			{ kinds: ["page"] },
			scope,
		);
		expect(JSON.stringify(result.documents)).not.toContain(
			"NESTED_CONTEXT_SHOULD_NOT_BE_INDEXED",
		);
		expect(result.coverage.complete).toBe(false);
	});

	it.each<WorkspaceKind>(["workflow", "event", "page", "table"])(
		"rejects inaccessible %s targets before any backend reads",
		async (kind) => {
			const { backend, scope, reads } = fixture();
			const target: WorkspaceTarget = {
				kind,
				app_id: "app-forbidden",
				board_id: "board-billing",
				id: "resource",
			};
			await expect(
				readWorkspaceDocument(backend, target, scope),
			).rejects.toThrow("WORKSPACE_APP_NOT_ACCESSIBLE");
			await expect(
				collectWorkspaceDocuments(
					backend,
					{ app_id: "app-forbidden", kinds: [kind] },
					scope,
				),
			).rejects.toThrow("WORKSPACE_APP_NOT_ACCESSIBLE");
			for (const read of Object.values(reads))
				expect(read).not.toHaveBeenCalled();
		},
	);

	it("accepts the explicitly scoped app and filters workflow inventory by board", async () => {
		const { backend, scope, reads } = fixture();
		scope.scopedAppId = "app-scoped";
		const result = await collectWorkspaceDocuments(
			backend,
			{
				board_id: "board-review",
				kinds: ["workflow"],
			},
			scope,
		);
		expect(result.coverage.complete).toBe(true);
		expect(result.documents.map((document) => document.symbol)).toEqual([
			"reviewInvoice",
		]);
		expect(reads.getFlowScriptAuthoritative).toHaveBeenCalledTimes(1);
		expect(reads.getFlowScriptAuthoritative).toHaveBeenCalledWith(
			"app-scoped",
			"board-review",
			undefined,
			true,
		);
	});

	it("retains reachable resources and reports individual read failures without backend error details", async () => {
		const { backend, scope, reads, sources } = fixture();
		reads.getFlowScriptAuthoritative.mockImplementation(async (_app, board) => {
			if (board === "board-review") throw new Error("PRIVATE_BACKEND_ERROR");
			return sources[board];
		});
		const result = await collectWorkspaceDocuments(
			backend,
			{ kinds: ["workflow", "table"] },
			scope,
		);
		expect(result.coverage.complete).toBe(false);
		expect(result.coverage.issues).toContainEqual({
			code: "RESOURCE_UNREADABLE",
			kind: "workflow",
			app_id: "app-invoices",
			resource: "board-review",
		});
		expect(
			result.documents.some(
				(document) => document.symbol === "billing::loadInvoices",
			),
		).toBe(true);
		expect(
			result.documents.some((document) => document.target.kind === "table"),
		).toBe(true);
		expect(JSON.stringify(result)).not.toContain("PRIVATE_BACKEND_ERROR");
	});

	it("reports inventory failures and malformed schemas instead of complete empty results", async () => {
		const { backend, scope, reads } = fixture();
		reads.getEventsAuthoritative.mockRejectedValue(
			new Error("PRIVATE_INVENTORY_ERROR"),
		);
		reads.getSchemaAuthoritative.mockResolvedValue({
			fields: "not a field array",
		});
		const result = await collectWorkspaceDocuments(
			backend,
			{ kinds: ["event", "table", "page"] },
			scope,
		);
		expect(result.coverage.complete).toBe(false);
		expect(result.coverage.issues.map((issue) => issue.code).sort()).toEqual([
			"INVENTORY_UNREADABLE",
			"RESOURCE_UNREADABLE",
		]);
		expect(result.documents.map((document) => document.target.kind)).toEqual([
			"page",
		]);
		expect(JSON.stringify(result)).not.toContain("PRIVATE_INVENTORY_ERROR");
	});

	it("bounds large app inventories and identifies omitted apps and resources", async () => {
		const { backend, scope, reads } = fixture();
		scope.getProfileAppIds = async () => new Set(["a", "b", "c", "d", "e"]);
		reads.listTablesAuthoritative.mockResolvedValue(
			Array.from(
				{ length: 17 },
				(_, index) => `table-${index.toString().padStart(2, "0")}`,
			),
		);
		reads.getSchemaAuthoritative.mockResolvedValue({
			fields: [{ name: "id", data_type: "Utf8" }],
		});
		const result = await collectWorkspaceDocuments(
			backend,
			{ kinds: ["table"] },
			scope,
		);
		expect(result.coverage.complete).toBe(false);
		expect(result.coverage.app_ids).toEqual(["a", "b", "c", "d"]);
		expect(
			result.coverage.issues.some((issue) => issue.code === "APP_LIMIT"),
		).toBe(true);
		expect(
			result.coverage.issues.filter((issue) => issue.code === "RESOURCE_LIMIT"),
		).toHaveLength(4);
		expect(reads.getSchemaAuthoritative).toHaveBeenCalledTimes(64);
		expect(result.documents).toHaveLength(64);
	});

	it.each(["array", "object", "depth"])(
		"marks silently bounded nested page %s content incomplete",
		async (shape) => {
			const { backend, scope, page } = fixture();
			const component = page.components[0].component as unknown as Record<
				string,
				unknown
			>;
			if (shape === "array")
				component.extensions = Array.from({ length: 257 }, (_, index) => ({
					path: `/binding/${index}`,
				}));
			if (shape === "object")
				component.extensions = Object.fromEntries(
					Array.from({ length: 129 }, (_, index) => [
						`field-${index}`,
						{ path: `/binding/${index}` },
					]),
				);
			if (shape === "depth") {
				let nested: unknown = { path: "/binding/deep" };
				for (let index = 0; index < 18; index++) nested = { nested };
				component.extensions = nested;
			}
			const result = await collectWorkspaceDocuments(
				backend,
				{ kinds: ["page"] },
				scope,
			);
			expect(result.coverage.complete).toBe(false);
			expect(
				result.coverage.issues.some(
					(issue) => issue.code === "CONTRACT_TRUNCATED",
				),
			).toBe(true);
		},
	);

	it("bounds Event pins and table fields with explicit incomplete coverage", async () => {
		const { backend, scope, event, reads } = fixture();
		event.inputs = Array.from({ length: 129 }, (_, index) => ({
			...event.inputs[0],
			name: `pin_${index}`,
		}));
		reads.getSchemaAuthoritative.mockResolvedValue({
			fields: Array.from({ length: 257 }, (_, index) => ({
				name: `field_${index}`,
				type: "string",
			})),
		});
		const result = await collectWorkspaceDocuments(
			backend,
			{ kinds: ["event", "table"] },
			scope,
		);
		expect(result.coverage.complete).toBe(false);
		expect(
			result.coverage.issues.filter(
				(issue) => issue.code === "CONTRACT_TRUNCATED",
			),
		).toHaveLength(2);
		expect(JSON.stringify(result.documents)).not.toContain("pin_128");
		expect(JSON.stringify(result.documents)).not.toContain("field_256");
	});

	it("reads a fresh contract revision and rejects mismatched board ownership", async () => {
		const { backend, scope, event, page } = fixture();
		const eventTarget: WorkspaceTarget = {
			kind: "event",
			app_id: "app-invoices",
			board_id: "board-billing",
			id: event.id,
		};
		const before = await readWorkspaceDocument(backend, eventTarget, scope);
		event.inputs[0].name = "billing_account_id";
		const after = await readWorkspaceDocument(backend, eventTarget, scope);
		expect(after.resource_id).toBe(before.resource_id);
		expect(after.revision).not.toBe(before.revision);
		expect(after.content).toContain("billing_account_id");
		await expect(
			readWorkspaceDocument(
				backend,
				{ ...eventTarget, board_id: "board-review" },
				scope,
			),
		).rejects.toThrow("WORKSPACE_RESOURCE_NOT_FOUND");
		await expect(
			readWorkspaceDocument(
				backend,
				{
					kind: "page",
					app_id: "app-invoices",
					board_id: "board-review",
					id: page.id,
				},
				scope,
			),
		).rejects.toThrow("WORKSPACE_RESOURCE_NOT_FOUND");
	});

	it("joins bundled docs with exact IDs, revisions and limited source coverage", async () => {
		const { backend, scope, reads } = fixture();
		scope.getProfileAppIds = vi.fn(async () => {
			throw new Error("APP_PROFILE_NOT_NEEDED");
		});
		const corpus = await loadFlowPilotWorkspaceDocs();
		const result = await collectWorkspaceDocuments(
			backend,
			{ kinds: ["doc"] },
			scope,
		);
		expect(result.coverage.docs).toEqual(corpus.coverage);
		expect(result.coverage.indexed_resources).toBe(corpus.entries.length);
		expect(result.coverage.app_ids).toEqual([]);
		const document = result.documents.find((entry) =>
			entry.source_path?.endsWith("dev/a2ui/pages.md"),
		);
		expect(document).toBeDefined();
		const target = parseWorkspaceResourceId(document?.resource_id);
		expect(target?.kind).toBe("doc");
		if (!target) throw new Error("Expected an exact docs resource ID");
		expect(await readWorkspaceDocument(backend, target, scope)).toEqual(
			document,
		);
		await expect(
			readWorkspaceDocument(
				backend,
				{ kind: "doc", id: "doc:../../private.md#secret" },
				scope,
			),
		).rejects.toThrow("WORKSPACE_RESOURCE_NOT_FOUND");
		for (const read of Object.values(reads))
			expect(read).not.toHaveBeenCalled();
		expect(scope.getProfileAppIds).not.toHaveBeenCalled();
	});

	it("searches the real bundled documentation and reads a discovered section through the tool service", async () => {
		const { backend, scope, reads } = fixture();
		const session = new WorkspaceSearchSession();
		const response = await session.search(
			backend,
			{ query: "Page Event Route", kinds: ["doc"], limit: 20 },
			scope,
		);
		expect(response.status).toBe("ok");
		const found = (response.hits as Record<string, unknown>[]).find(
			(hit) => hit.title === "Pages > Page, Event, and Route",
		);
		if (!found)
			throw new Error("Expected the bundled Page, Event, and Route section");
		expect(found.revision).toMatch(/^[a-f0-9]{64}$/);
		const read = await readWorkspaceSymbol(backend, found, scope);
		expect(read).toMatchObject({
			status: "ok",
			kind: "doc",
			revision: found.revision,
			content_is_untrusted: true,
			next_offset: null,
		});
		const corpus = await loadFlowPilotWorkspaceDocs();
		const original = corpus.entries.find(
			(entry) => entry.title === found.title,
		);
		expect(read.content).toBe(original?.content);
		for (const method of Object.values(reads))
			expect(method).not.toHaveBeenCalled();
	});

	it("bounds a zero-hit search reply when unreadable Unicode resource diagnostics exceed the response budget", async () => {
		const { backend, scope, reads, event } = fixture();
		scope.getProfileAppIds = async () =>
			new Set(
				Array.from(
					{ length: 4 },
					(_, index) => `app-${index}-${"🧩".repeat(100)}`,
				),
			);
		reads.getEventsAuthoritative.mockResolvedValue(
			Array.from({ length: 16 }, (_, index) => ({
				...event,
				id: `event-${index.toString().padStart(2, "0")}-${"請".repeat(200)}`,
			})),
		);
		reads.getEventAuthoritative.mockRejectedValue(
			new Error("Resource unavailable"),
		);
		const session = new WorkspaceSearchSession();
		const response = await session.search(
			backend,
			{ query: "nonexistent_feature_marker", kinds: ["event"] },
			scope,
		);
		expect(reads.getEventAuthoritative).toHaveBeenCalledTimes(64);
		expect(response).toMatchObject({
			status: "error",
			code: "WORKSPACE_REPLY_TOO_LARGE",
		});
		expect(jsonBytes(response)).toBeLessThanOrEqual(24_000);
	});

	it("round trips exact resource identities and refuses malformed or noncanonical IDs", () => {
		const target: WorkspaceTarget = {
			kind: "workflow",
			app_id: "app / 日本語",
			board_id: "board:#",
			id: "billing::loadInvoices",
		};
		const resourceId = workspaceResourceId(target);
		expect(parseWorkspaceResourceId(resourceId)).toEqual(target);
		for (const value of [
			undefined,
			"",
			"workspace:v1:%broken",
			"workspace:v1:[]",
			workspaceResourceId({
				kind: "workflow",
				app_id: "app",
				id: "missing-board",
			}),
			workspaceResourceId({ kind: "doc", app_id: "app", id: "wrong-scope" }),
			`${resourceId}suffix`,
			resourceId.replace("%5B", "%5b"),
		]) {
			expect(parseWorkspaceResourceId(value)).toBeUndefined();
		}
	});
});
