import { afterEach, expect, test } from "bun:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Window } from "happy-dom";
import { type ReactNode, act } from "react";
import { ApiResponseError } from "../../../lib/api-error";
import {
	type IBackendState,
	useBackendStore,
} from "../../../state/backend-state";
import type {
	GraphOverlay,
	GraphSchema,
	NodeLabelMapping,
	RemoteOntologyImport,
	UpdateOntologyObjectPayload,
	UpdateOntologyRowResult,
} from "../../../state/backend-state/graph-state";

const SUB = "42c52474-5081-70d7-2b23-4bd8c38d8fb0";
const OUTLINE = {
	type: "Polygon",
	coordinates: [
		[
			[16.5083, 48.2204],
			[16.5115, 48.222],
			[16.5102, 48.223],
			[16.5083, 48.2204],
		],
	],
};
const METADATA = {
	"ARROW:extension:name": "geoarrow.wkb",
	"ARROW:extension:metadata": '{"crs":"EPSG:4326","edges":"planar"}',
};
const ROW = {
	id: "HOE-87",
	name: "C2",
	outline: OUTLINE,
	owner_sub: SUB,
	report_file: "apps/app-1/upload/sites/hoe-87.pdf",
	tags: { landuse: "industrial" },
};
const STYLE = { color: "", icon: "", size: { mode: "fixed" } };
const OVERLAY = {
	id: "onto-1",
	name: "Sites",
	nodes: [
		{
			id: "site",
			label: "Site",
			table: "sites",
			id_column: "id",
			display_column: "name",
			property_columns: [
				{ name: "outline", data_type: "Binary" },
				{ name: "owner_sub", data_type: "Utf8" },
				{ name: "report_file", data_type: "Utf8" },
				{ name: "tags", data_type: "Utf8" },
			],
			style: STYLE,
		},
	],
	edges: [],
	object_views: [],
	actions: [],
	exposed: false,
} as unknown as GraphOverlay;
const SCHEMA: GraphSchema = {
	node_labels: [
		{
			label: "Site",
			table: "sites",
			properties: [
				{
					name: "outline",
					data_type: "Binary",
					nullable: true,
					metadata: METADATA,
				},
			],
		},
	],
	edge_labels: [],
};

// Sites are linked by `code`, so `code` identifies them rather than `id`.
const EDIT_ROW = {
	id: "HOE-87",
	code: "S-1",
	name: "C2",
	score: 3,
	created_at: 1_700_000_000_000,
};
const EDIT_OVERLAY = {
	id: "onto-2",
	name: "Sites",
	nodes: [
		{
			id: "site",
			label: "Site",
			table: "sites",
			id_column: "id",
			display_column: "name",
			property_columns: [
				{ name: "code", data_type: "Utf8" },
				{ name: "score", data_type: "Int64" },
				{ name: "created_at", data_type: "Timestamp(Millisecond, None)" },
			],
			style: STYLE,
		},
	],
	edges: [
		{
			label: "NEXT_TO",
			table: "site_links",
			src_label: "Site",
			dst_label: "Site",
			src_column: "from_code",
			dst_column: "to_code",
			src_node_column: "code",
			dst_node_column: "code",
			property_columns: [],
			style: STYLE,
		},
	],
	object_views: [],
	actions: [],
	exposed: false,
} as unknown as GraphOverlay;
const ARROW_SCHEMA = {
	fields: [
		{ name: "id", data_type: "Utf8", nullable: false },
		{ name: "code", data_type: "Utf8", nullable: false },
		{ name: "name", data_type: "Utf8", nullable: true },
		{ name: "score", data_type: "Int64", nullable: true },
		{
			name: "created_at",
			data_type: { Timestamp: ["Millisecond", null] },
			nullable: true,
		},
	],
};

let cleanup: (() => Promise<void>) | undefined;
afterEach(async () => {
	await cleanup?.();
	cleanup = undefined;
});

async function setup() {
	const window = new Window();
	Object.assign(window, { SyntaxError, TypeError });
	Object.assign(globalThis, {
		document: window.document,
		Element: window.Element,
		Event: window.Event,
		CustomEvent: window.CustomEvent,
		InputEvent: window.InputEvent,
		KeyboardEvent: window.KeyboardEvent,
		FocusEvent: window.FocusEvent,
		PointerEvent: window.PointerEvent,
		HTMLElement: window.HTMLElement,
		HTMLButtonElement: window.HTMLButtonElement,
		HTMLInputElement: window.HTMLInputElement,
		HTMLTextAreaElement: window.HTMLTextAreaElement,
		MouseEvent: window.MouseEvent,
		Node: window.Node,
		NodeFilter: window.NodeFilter,
		navigator: window.navigator,
		window,
		Document: window.Document,
		DocumentFragment: window.DocumentFragment,
		Text: window.Text,
		MutationObserver: window.MutationObserver,
		ResizeObserver: window.ResizeObserver,
		getComputedStyle: window.getComputedStyle.bind(window),
		requestAnimationFrame: window.requestAnimationFrame.bind(window),
		cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
		IS_REACT_ACT_ENVIRONMENT: true,
	});
	const { createRoot } = await import("react-dom/client");
	const container = window.document.createElement("div");
	window.document.body.append(container);
	const root = createRoot(container as unknown as HTMLElement);
	const client = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	client.setQueryData(["lookupUserBatched", SUB], {
		id: SUB,
		name: "Felix Schultz",
		created_at: "",
	});
	const schemaReads: string[][] = [];
	const tableSchemaReads: string[][] = [];
	const graphUpdates: unknown[][] = [];
	const previous = useBackendStore.getState().backend;
	useBackendStore.setState({
		backend: {
			...previous,
			graphState: {
				getSchema: async (appId: string, overlayId: string) => {
					schemaReads.push([appId, overlayId]);
					return SCHEMA;
				},
				updateObject: async (...args: unknown[]) => {
					graphUpdates.push(args);
					throw new Error("the panel writes through onUpdateObject only");
				},
			},
			dbState: {
				...previous?.dbState,
				getSchema: async (appId: string, table: string) => {
					tableSchemaReads.push([appId, table]);
					return ARROW_SCHEMA;
				},
			},
		} as unknown as IBackendState,
	});
	cleanup = async () => {
		await act(async () => root.unmount());
		client.clear();
		useBackendStore.setState({ backend: previous });
		await window.happyDOM.close();
	};
	const settle = () =>
		act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 20));
		});
	const click = async (element: Element | null | undefined) => {
		if (!element) throw new Error("nothing to click");
		await act(async () => {
			(element as HTMLElement).click();
		});
		await settle();
	};
	const type = async (input: Element | null, value: string) => {
		if (!input) throw new Error("no input to type into");
		await act(async () => {
			Object.getOwnPropertyDescriptor(
				window.HTMLInputElement.prototype,
				"value",
			)?.set?.call(input, value);
			input.dispatchEvent(
				new window.Event("input", { bubbles: true }) as never,
			);
		});
	};
	const press = async (target: Element, key: string) => {
		await act(async () => {
			target.dispatchEvent(
				new window.KeyboardEvent("keydown", { key, bubbles: true }) as never,
			);
		});
	};
	return {
		container: container as unknown as HTMLElement,
		body: window.document.body as unknown as HTMLElement,
		schemaReads,
		tableSchemaReads,
		graphUpdates,
		settle,
		click,
		type,
		press,
		render: async (children: ReactNode) => {
			await act(async () => {
				root.render(
					<QueryClientProvider client={client}>{children}</QueryClientProvider>,
				);
			});
			await settle();
		},
	};
}

function cellsByColumn(container: HTMLElement) {
	const headers = [...container.querySelectorAll("thead th")].map(
		(cell) => cell.textContent,
	);
	const cells = [...container.querySelectorAll("tbody tr td")];
	return new Map(headers.map((header, index) => [header, cells[index]]));
}

function buttonNamed(root: ParentNode, name: string) {
	return [...root.querySelectorAll("button")].find(
		(button) =>
			(button.getAttribute("aria-label") ?? button.textContent?.trim()) ===
			name,
	);
}

function openButton(container: HTMLElement) {
	return [...container.querySelectorAll("button")].find((button) =>
		button.getAttribute("aria-label")?.startsWith("Open Site"),
	);
}

function sheet(body: HTMLElement) {
	const dialog = body.querySelector('[role="dialog"]');
	if (!dialog) throw new Error("the object sheet is not open");
	return dialog;
}

function fieldCard(dialog: Element, title: string) {
	return [...dialog.querySelectorAll("p")].find(
		(paragraph) => paragraph.textContent === title,
	)?.parentElement?.parentElement;
}

function fieldInput(dialog: Element, name: string) {
	return dialog.querySelector<HTMLInputElement>(`input[aria-label="${name}"]`);
}

const noop = async () => {
	throw new Error("not used");
};

type UpdateCall = [string, NodeLabelMapping, UpdateOntologyObjectPayload];

function updateRecorder(
	respond: (
		payload: UpdateOntologyObjectPayload,
		attempt: number,
	) => UpdateOntologyRowResult,
) {
	const calls: UpdateCall[] = [];
	const onUpdateObject = async (
		ontologyId: string,
		objectType: NodeLabelMapping,
		payload: UpdateOntologyObjectPayload,
	) => {
		calls.push([ontologyId, objectType, payload]);
		return respond(payload, calls.length);
	};
	return { calls, onUpdateObject };
}

function sampler() {
	let calls = 0;
	return {
		calls: () => calls,
		onSample: async () => {
			calls += 1;
			return [{ ...EDIT_ROW }];
		},
	};
}

test("object list cells use the smart readings, typed by the live schema", async () => {
	const { container, render, schemaReads } = await setup();
	const { ObjectExplorerPanel } = await import("./data-studio-panels");
	await render(
		<ObjectExplorerPanel
			appId="app-1"
			ontologies={[OVERLAY]}
			onCreateOntology={() => {}}
			onSample={async () => [ROW]}
			onInvokeAction={noop}
		/>,
	);
	const cells = cellsByColumn(container);
	expect(schemaReads).toEqual([["app-1", "onto-1"]]);
	expect(cells.get("Outline")?.querySelector("button")?.textContent).toContain(
		"Polygon",
	);
	expect(cells.get("Owner Sub")?.textContent).toContain("Felix Schultz");
	expect(cells.get("Owner Sub")?.textContent).not.toContain(SUB);
	expect(
		cells.get("Report File")?.querySelector("button")?.textContent,
	).toContain("hoe-87.pdf");
	expect(cells.get("Tags")?.textContent).toContain('"landuse"');
}, 30_000);

test("a remote object's paths stay text and its schema is not read locally", async () => {
	const { container, render, schemaReads } = await setup();
	const { ObjectExplorerPanel } = await import("./data-studio-panels");
	const remote = {
		id: "import-1",
		target_app_id: "app-1",
		contract: OVERLAY,
	} as unknown as RemoteOntologyImport;
	await render(
		<ObjectExplorerPanel
			appId="app-2"
			ontologies={[]}
			remoteImports={[remote]}
			initialSourceValue="import-1"
			onCreateOntology={() => {}}
			onSample={noop}
			onSampleRemote={async () => [{ ...ROW, report_file: "sites/hoe-87.pdf" }]}
			onInvokeAction={noop}
		/>,
	);
	const cells = cellsByColumn(container);
	expect(schemaReads).toEqual([]);
	// A bare path would otherwise resolve into this app's own storage root.
	expect(cells.get("Report File")?.querySelector("button")).toBeNull();
	expect(cells.get("Report File")?.textContent).toContain("sites/hoe-87.pdf");
	expect(cells.get("Outline")?.querySelector("button")?.textContent).toContain(
		"Polygon",
	);
}, 30_000);

test("without an update callback the object sheet has no Edit button", async () => {
	const { container, body, render, click, tableSchemaReads } = await setup();
	const { ObjectExplorerPanel } = await import("./data-studio-panels");
	await render(
		<ObjectExplorerPanel
			appId="app-1"
			ontologies={[EDIT_OVERLAY]}
			onCreateOntology={() => {}}
			onSample={async () => [{ ...EDIT_ROW }]}
			onInvokeAction={noop}
		/>,
	);
	expect(container.textContent).toContain("Read-only");
	await click(openButton(container));
	const dialog = sheet(body);
	expect(dialog.textContent).toContain("C2");
	expect(buttonNamed(dialog, "Edit")).toBeUndefined();
	expect(tableSchemaReads).toEqual([]);
}, 30_000);

test("a remote object is never editable, typed or written", async () => {
	const { container, body, render, click, tableSchemaReads, graphUpdates } =
		await setup();
	const { ObjectExplorerPanel } = await import("./data-studio-panels");
	const remote = {
		id: "import-2",
		target_app_id: "app-1",
		contract: EDIT_OVERLAY,
	} as unknown as RemoteOntologyImport;
	const updates = updateRecorder(() => {
		throw new Error("a remote object must not be written");
	});
	await render(
		<ObjectExplorerPanel
			appId="app-2"
			ontologies={[]}
			remoteImports={[remote]}
			initialSourceValue="import-2"
			onCreateOntology={() => {}}
			onSample={noop}
			onSampleRemote={async () => [{ ...EDIT_ROW }]}
			onInvokeAction={noop}
			onUpdateObject={updates.onUpdateObject}
		/>,
	);
	expect(container.textContent).toContain("Remote object · read-only");
	await click(openButton(container));
	const dialog = sheet(body);
	expect(dialog.textContent).toContain("C2");
	expect(buttonNamed(dialog, "Edit")).toBeUndefined();
	expect(tableSchemaReads).toEqual([]);
	expect(graphUpdates).toEqual([]);
	expect(updates.calls).toEqual([]);
}, 30_000);

test("edits save the changed fields by the effective id and patch the row in place", async () => {
	const { container, body, render, click, type, tableSchemaReads } =
		await setup();
	const { ObjectExplorerPanel } = await import("./data-studio-panels");
	const samples = sampler();
	const updates = updateRecorder((payload) => ({
		outcome: "updated",
		row: { ...EDIT_ROW, ...payload.updates },
	}));
	await render(
		<ObjectExplorerPanel
			appId="app-1"
			ontologies={[EDIT_OVERLAY]}
			onCreateOntology={() => {}}
			onSample={samples.onSample}
			onInvokeAction={noop}
			onUpdateObject={updates.onUpdateObject}
		/>,
	);
	expect(container.textContent).not.toContain("Read-only");
	expect(tableSchemaReads).toEqual([["app-1", "sites"]]);
	await click(openButton(container));
	const dialog = sheet(body);
	await click(buttonNamed(dialog, "Edit"));

	expect(dialog.textContent).toContain("Editing");
	for (const title of ["Id", "Code"]) {
		const card = fieldCard(dialog, title);
		expect(card?.querySelector("input")).toBeNull();
		expect(
			card?.querySelector('button[aria-label*="identifies the object"]'),
		).not.toBeNull();
	}
	expect(fieldInput(dialog, "name")?.value).toBe("C2");

	await type(fieldInput(dialog, "name"), "C3");
	await type(fieldInput(dialog, "score"), "5");
	expect(dialog.textContent).toContain("2 unsaved changes");
	await click(buttonNamed(dialog, "Save Changes"));

	expect(updates.calls).toHaveLength(1);
	const [ontologyId, objectType, payload] = updates.calls[0];
	expect(ontologyId).toBe("onto-2");
	expect(objectType.label).toBe("Site");
	expect(payload).toEqual({
		object_type: "site",
		id: "S-1",
		updates: { name: "C3", score: 5 },
		expected: { name: "C2", score: 3 },
	});
	expect(dialog.querySelector("h2")?.textContent).toBe("C3");
	expect(buttonNamed(dialog, "Save Changes")).toBeUndefined();
	expect(buttonNamed(dialog, "Edit")).toBeDefined();
	const cells = cellsByColumn(container);
	expect(cells.get("Name")?.textContent).toContain("C3");
	expect(cells.get("Score")?.textContent).toContain("5");
	expect(samples.calls()).toBe(1);
}, 30_000);

test("a rejected save keeps the drafts and says why", async () => {
	const { container, body, render, click, type } = await setup();
	const { ObjectExplorerPanel } = await import("./data-studio-panels");
	const samples = sampler();
	const updates = updateRecorder((_, attempt) => {
		if (attempt === 1) throw new Error("Server said no");
		throw new Error("'Site' S-1 was not found; it may have been deleted");
	});
	await render(
		<ObjectExplorerPanel
			appId="app-1"
			ontologies={[EDIT_OVERLAY]}
			onCreateOntology={() => {}}
			onSample={samples.onSample}
			onInvokeAction={noop}
			onUpdateObject={updates.onUpdateObject}
		/>,
	);
	await click(openButton(container));
	const dialog = sheet(body);
	await click(buttonNamed(dialog, "Edit"));
	await type(fieldInput(dialog, "name"), "C3");
	await click(buttonNamed(dialog, "Save Changes"));

	expect(dialog.querySelector('[role="alert"]')?.textContent).toContain(
		"Server said no",
	);
	expect(fieldInput(dialog, "name")?.value).toBe("C3");
	expect(dialog.textContent).toMatch(/1 unsaved change(?!s)/);

	await click(buttonNamed(dialog, "Save Changes"));
	const alert = dialog.querySelector('[role="alert"]');
	expect(alert?.textContent).toContain("This object no longer exists");
	expect(fieldInput(dialog, "name")?.value).toBe("C3");
	await click(alert && buttonNamed(alert, "Refresh"));
	expect(samples.calls()).toBe(2);
}, 30_000);

test("a stale save shows the conflict and the next save sends the rebased values", async () => {
	const { container, body, render, click, type } = await setup();
	const { ObjectExplorerPanel } = await import("./data-studio-panels");
	const updates = updateRecorder((payload, attempt) =>
		attempt === 1
			? {
					outcome: "stale",
					row: { ...EDIT_ROW, name: "Theirs", score: 4 },
				}
			: {
					outcome: "updated",
					row: { ...EDIT_ROW, score: 4, ...payload.updates },
				},
	);
	await render(
		<ObjectExplorerPanel
			appId="app-1"
			ontologies={[EDIT_OVERLAY]}
			onCreateOntology={() => {}}
			onSample={async () => [{ ...EDIT_ROW }]}
			onInvokeAction={noop}
			onUpdateObject={updates.onUpdateObject}
		/>,
	);
	await click(openButton(container));
	const dialog = sheet(body);
	await click(buttonNamed(dialog, "Edit"));
	await type(fieldInput(dialog, "name"), "C3");
	await click(buttonNamed(dialog, "Save Changes"));

	expect(dialog.querySelector('[role="alert"]')?.textContent).toContain(
		"Someone changed this value",
	);
	expect(fieldInput(dialog, "name")?.value).toBe("C3");
	// An untouched field follows the stored value; an edited one keeps the draft.
	expect(fieldInput(dialog, "score")?.value).toBe("4");
	expect(dialog.querySelector("h2")?.textContent).toBe("Theirs");

	await click(buttonNamed(dialog, "Save Changes"));
	expect(updates.calls.map(([, , payload]) => payload)).toEqual([
		{
			object_type: "site",
			id: "S-1",
			updates: { name: "C3" },
			expected: { name: "C2" },
		},
		{
			object_type: "site",
			id: "S-1",
			updates: { name: "C3" },
			expected: { name: "Theirs" },
		},
	]);
	expect(dialog.querySelector("h2")?.textContent).toBe("C3");
	expect(buttonNamed(dialog, "Save Changes")).toBeUndefined();
}, 30_000);

test("closing with unsaved changes asks first and keeps the sheet open", async () => {
	const { container, body, render, click, type, press } = await setup();
	const { ObjectExplorerPanel } = await import("./data-studio-panels");
	const updates = updateRecorder(() => {
		throw new Error("not saved in this test");
	});
	await render(
		<ObjectExplorerPanel
			appId="app-1"
			ontologies={[EDIT_OVERLAY]}
			onCreateOntology={() => {}}
			onSample={async () => [{ ...EDIT_ROW }]}
			onInvokeAction={noop}
			onUpdateObject={updates.onUpdateObject}
		/>,
	);
	await click(openButton(container));
	const dialog = sheet(body);
	await click(buttonNamed(dialog, "Edit"));
	const name = fieldInput(dialog, "name");
	await type(name, "C3");

	await press(name as Element, "Escape");
	expect(sheet(body)).toBe(dialog);
	expect(dialog.textContent).toContain("Discard your unsaved changes?");
	await click(buttonNamed(dialog, "Keep editing"));
	expect(dialog.textContent).not.toContain("Discard your unsaved changes?");
	expect(fieldInput(dialog, "name")?.value).toBe("C3");

	await click(buttonNamed(dialog, "Close"));
	const banner = [...dialog.querySelectorAll('[role="alert"]')].find((alert) =>
		alert.textContent?.includes("Discard your unsaved changes?"),
	);
	expect(banner).toBeDefined();
	await click(banner && buttonNamed(banner, "Discard"));
	expect(body.querySelector('[role="dialog"]')).toBeNull();
	expect(updates.calls).toEqual([]);
}, 30_000);

test("a re-sample that hands in the same object keeps the edit in progress", async () => {
	const { container, body, render, click, type } = await setup();
	const { ObjectExplorerPanel } = await import("./data-studio-panels");
	const samples = sampler();
	const updates = updateRecorder(() => {
		throw new Error("not saved in this test");
	});
	const panel = (ontology: GraphOverlay) => (
		<ObjectExplorerPanel
			appId="app-1"
			ontologies={[ontology]}
			onCreateOntology={() => {}}
			onSample={samples.onSample}
			onInvokeAction={noop}
			onUpdateObject={updates.onUpdateObject}
		/>
	);
	await render(panel(EDIT_OVERLAY));
	await click(openButton(container));
	const dialog = sheet(body);
	await click(buttonNamed(dialog, "Edit"));
	await type(fieldInput(dialog, "name"), "C3");

	await render(panel({ ...EDIT_OVERLAY }));
	expect(samples.calls()).toBe(2);
	expect(sheet(body)).toBe(dialog);
	expect(fieldInput(dialog, "name")?.value).toBe("C3");
	expect(dialog.textContent).toMatch(/1 unsaved change(?!s)/);
}, 30_000);

test("closing during a save waits for it instead of offering to discard", async () => {
	const { container, body, render, click, type, press, settle } = await setup();
	const { ObjectExplorerPanel } = await import("./data-studio-panels");
	let finish: (() => void) | undefined;
	const calls: UpdateOntologyObjectPayload[] = [];
	const onUpdateObject = async (
		_ontologyId: string,
		_objectType: NodeLabelMapping,
		payload: UpdateOntologyObjectPayload,
	): Promise<UpdateOntologyRowResult> => {
		calls.push(payload);
		await new Promise<void>((resolve) => {
			finish = resolve;
		});
		return { outcome: "updated", row: { ...EDIT_ROW, ...payload.updates } };
	};
	await render(
		<ObjectExplorerPanel
			appId="app-1"
			ontologies={[EDIT_OVERLAY]}
			onCreateOntology={() => {}}
			onSample={async () => [{ ...EDIT_ROW }]}
			onInvokeAction={noop}
			onUpdateObject={onUpdateObject}
		/>,
	);
	await click(openButton(container));
	const dialog = sheet(body);
	await click(buttonNamed(dialog, "Edit"));
	await type(fieldInput(dialog, "name"), "C3");
	await click(buttonNamed(dialog, "Save Changes"));
	expect(calls).toHaveLength(1);

	await press(dialog, "Escape");
	await click(buttonNamed(dialog, "Close"));
	expect(sheet(body)).toBe(dialog);
	expect(dialog.textContent).not.toContain("Discard your unsaved changes?");

	await act(async () => finish?.());
	await settle();
	expect(sheet(body)).toBe(dialog);
	expect(dialog.querySelector("h2")?.textContent).toBe("C3");
	expect(buttonNamed(dialog, "Save Changes")).toBeUndefined();
	expect(dialog.textContent).not.toContain("Discard your unsaved changes?");
}, 30_000);

test("only a hub 404 with a code or the storage message reads as a deleted object", async () => {
	const { container, body, render, click, type } = await setup();
	const { ObjectExplorerPanel } = await import("./data-studio-panels");
	const updates = updateRecorder((_, attempt) => {
		if (attempt === 1)
			throw new ApiResponseError({
				status: 400,
				code: "BAD_REQUEST",
				message: "Object type 'Site' was not found in the ontology",
			});
		if (attempt === 2)
			throw new ApiResponseError({
				status: 404,
				statusText: "Not Found",
				message: "Not Found",
			});
		throw new ApiResponseError({
			status: 404,
			code: "NOT_FOUND",
			message: "'Site' S-1 was not found; it may have been deleted",
		});
	});
	await render(
		<ObjectExplorerPanel
			appId="app-1"
			ontologies={[EDIT_OVERLAY]}
			onCreateOntology={() => {}}
			onSample={async () => [{ ...EDIT_ROW }]}
			onInvokeAction={noop}
			onUpdateObject={updates.onUpdateObject}
		/>,
	);
	await click(openButton(container));
	const dialog = sheet(body);
	await click(buttonNamed(dialog, "Edit"));
	await type(fieldInput(dialog, "name"), "C3");
	const failure = () => dialog.querySelector('[role="alert"]');

	await click(buttonNamed(dialog, "Save Changes"));
	expect(failure()?.textContent).toContain(
		"Object type 'Site' was not found in the ontology",
	);
	expect(failure()?.textContent).not.toContain("This object no longer exists");
	expect(buttonNamed(failure() as Element, "Refresh")).toBeUndefined();

	await click(buttonNamed(dialog, "Save Changes"));
	expect(failure()?.textContent).toContain("Not Found");
	expect(failure()?.textContent).not.toContain("This object no longer exists");

	await click(buttonNamed(dialog, "Save Changes"));
	expect(failure()?.textContent).toContain("This object no longer exists");
	expect(buttonNamed(failure() as Element, "Refresh")).toBeDefined();
	expect(updates.calls).toHaveLength(3);
}, 30_000);
