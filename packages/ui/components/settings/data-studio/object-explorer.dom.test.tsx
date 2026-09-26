import { afterEach, expect, test } from "bun:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Window } from "happy-dom";
import { type ReactNode, act } from "react";
import {
	type IBackendState,
	useBackendStore,
} from "../../../state/backend-state";
import type {
	GraphOverlay,
	GraphSchema,
	RemoteOntologyImport,
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
			style: { color: "", icon: "", size: { mode: "fixed" } },
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
	const previous = useBackendStore.getState().backend;
	useBackendStore.setState({
		backend: {
			...previous,
			graphState: {
				getSchema: async (appId: string, overlayId: string) => {
					schemaReads.push([appId, overlayId]);
					return SCHEMA;
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
	return {
		container: container as unknown as HTMLElement,
		schemaReads,
		render: async (children: ReactNode) => {
			await act(async () => {
				root.render(
					<QueryClientProvider client={client}>{children}</QueryClientProvider>,
				);
			});
			await act(async () => {
				await new Promise((resolve) => setTimeout(resolve, 20));
			});
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

const noop = async () => {
	throw new Error("not used");
};

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
