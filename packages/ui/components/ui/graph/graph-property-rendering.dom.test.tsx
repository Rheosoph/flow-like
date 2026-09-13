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
	SubgraphEdge,
	SubgraphNode,
} from "../../../state/backend-state/graph-state";

const SUB = "42c52474-5081-70d7-2b23-4bd8c38d8fb0";
const POINT = { type: "Point", coordinates: [13, 52] };
const METADATA = {
	"ARROW:extension:name": "geoarrow.wkb",
	"ARROW:extension:metadata": '{"crs":"EPSG:4326","edges":"planar"}',
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
		HTMLElement: window.HTMLElement,
		HTMLButtonElement: window.HTMLButtonElement,
		HTMLInputElement: window.HTMLInputElement,
		HTMLTextAreaElement: window.HTMLTextAreaElement,
		MouseEvent: window.MouseEvent,
		Node: window.Node,
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
	const previous = useBackendStore.getState().backend;
	cleanup = async () => {
		await act(async () => root.unmount());
		client.clear();
		useBackendStore.setState({ backend: previous });
		await window.happyDOM.close();
	};
	return {
		client,
		container,
		render: (children: ReactNode) =>
			act(async () => {
				root.render(
					<QueryClientProvider client={client}>{children}</QueryClientProvider>,
				);
			}),
	};
}

test("ontology properties keep declared Geometry distinct from ordinary JSON", async () => {
	const { container, render } = await setup();
	const { PropertyValue } = await import("./graph-node-inspector");
	await render(
		<>
			<div data-field="sub">
				<PropertyValue propKey="sub" value={SUB} />
			</div>
			<div data-field="geometry">
				<PropertyValue propKey="location" value={POINT} metadata={METADATA} />
			</div>
			<div data-field="json">
				<PropertyValue propKey="payload" value={POINT} />
			</div>
		</>,
	);
	expect(container.querySelector('[data-field="sub"]')?.textContent).toContain(
		"Felix Schultz",
	);
	expect(container.querySelector('[data-field="sub"]')?.textContent).toContain(
		"FS",
	);
	expect(
		container.querySelector('[data-field="sub"]')?.textContent,
	).not.toContain(SUB);
	expect(
		container.querySelector('[data-field="geometry"] button')?.textContent,
	).toContain("Point");
	expect(
		container.querySelector('[data-field="json"] pre')?.textContent,
	).toContain('"coordinates"');
}, 30_000);

test("Cypher columns use account tags and metadata while keeping row alignment", async () => {
	const { container, render } = await setup();
	const { GraphQueryPanel } = await import("./graph-query-panel");
	await render(
		<GraphQueryPanel
			onRunCypher={() => {}}
			results={[
				{ "n.sub": SUB, "n.location": POINT },
				{ "n.location": POINT, "n.sub": SUB, "n.payload": POINT },
			]}
			propertyMetadata={{ "n.location": METADATA }}
		/>,
	);
	expect(
		[...container.querySelectorAll("th")].map((cell) => cell.textContent),
	).toEqual(["n.sub", "n.location", "n.payload"]);
	const rows = container.querySelectorAll("tbody tr");
	for (const row of rows) {
		const cells = row.querySelectorAll("td");
		expect(cells).toHaveLength(3);
		expect(cells[0].textContent).toContain("Felix Schultz");
		expect(cells[1].querySelector("button")?.textContent).toContain("Point");
	}
	expect(
		rows[1].querySelectorAll("td")[2].querySelector("pre")?.textContent,
	).toContain('"coordinates"');
}, 30_000);

test("node titles and edge details use the same resolved account identity", async () => {
	const { container, render } = await setup();
	const { GraphNodeInspector } = await import("./graph-node-inspector");
	const { GraphEdgeInspector } = await import("./graph-edge-inspector");
	const node: SubgraphNode = {
		id: `Person:${SUB}`,
		label: "Person",
		caption: SUB,
		props: { sub: SUB },
	};
	const overlay = {
		nodes: [{ label: "Person", id_column: "sub" }],
		object_views: [],
	} as unknown as GraphOverlay;
	await render(
		<GraphNodeInspector node={node} overlay={overlay} onClose={() => {}} />,
	);
	expect(container.querySelector("h3")?.textContent).toContain("Felix Schultz");
	await render(<GraphEdgeInspector edge={null} onClose={() => {}} />);
	const edge: SubgraphEdge = {
		id: "edge",
		source: node.id,
		target: "Place:1",
		label: "VISITED",
		props: { sub: SUB, location: POINT },
		property_metadata: { location: METADATA },
	};
	await render(
		<GraphEdgeInspector
			edge={edge}
			sourceAccountId={SUB}
			targetCaption="Place"
			onClose={() => {}}
		/>,
	);
	expect(container.textContent?.match(/Felix Schultz/g)).toHaveLength(2);
	expect(container.textContent).toContain("geometry");
	expect(
		[...container.querySelectorAll("button")].some((button) =>
			button.textContent.includes("Point"),
		),
	).toBe(true);
}, 30_000);

test("guided expansion remains available without quick expansion", async () => {
	const { container, render } = await setup();
	const { GraphNodeInspector } = await import("./graph-node-inspector");
	let expansions = 0;
	await render(
		<GraphNodeInspector
			node={{
				id: "RSSFeed:1",
				label: "RSSFeed",
				caption: "News feed",
				props: {},
			}}
			onClose={() => {}}
			onGuidedExpand={() => {
				expansions += 1;
			}}
		/>,
	);
	const expand = [...container.querySelectorAll("button")].find((button) =>
		button.textContent.includes("Expand with"),
	);
	expect(expand).toBeDefined();
	await act(async () => expand?.click());
	expect(expansions).toBe(1);
}, 30_000);

test("canvas labels and React captions share cached identities and batch unresolved accounts", async () => {
	const { container, render } = await setup();
	const { GraphNodeCaption, useGraphAccountLabels } = await import(
		"./graph-node-caption"
	);
	const otherSub = "32a5a414-a001-70d1-7b23-570b1c9d4e2f";
	const batches: string[][] = [];
	const backend = useBackendStore.getState().backend;
	useBackendStore.setState({
		backend: {
			...backend,
			userState: {
				lookupUsers: async (ids: string[]) => {
					batches.push(ids);
					return ids.map((id) => ({
						id,
						name: "Ada Lovelace",
						created_at: "",
					}));
				},
				lookupUser: async (id: string) => ({
					id,
					name: "Ada Lovelace",
					created_at: "",
				}),
			},
		} as unknown as IBackendState,
	});
	const nodes: SubgraphNode[] = [
		{ id: "cached", label: "Person", caption: SUB, props: { sub: SUB } },
		{
			id: "first",
			label: "Person",
			caption: otherSub,
			props: { sub: otherSub },
		},
		{
			id: "repeat",
			label: "Person",
			caption: otherSub,
			props: { sub: otherSub },
		},
		{
			id: "document",
			label: "Document",
			caption: otherSub,
			props: { board_id: otherSub },
		},
	];
	const before = JSON.stringify(nodes);
	const overlay = {
		nodes: [
			{ label: "Person", id_column: "sub" },
			{ label: "Document", id_column: "board_id" },
		],
		object_views: [],
	} as unknown as GraphOverlay;
	function Probe() {
		const labels = useGraphAccountLabels(nodes, overlay);
		return (
			<>
				<output>{JSON.stringify(Object.fromEntries(labels))}</output>
				<div data-caption>
					<GraphNodeCaption node={nodes[1]} overlay={overlay} />
				</div>
			</>
		);
	}
	await render(<Probe />);
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 100));
	});
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 20));
	});
	expect(
		JSON.parse(container.querySelector("output")?.textContent ?? "{}"),
	).toEqual({
		cached: "Felix Schultz",
		first: "Ada Lovelace",
		repeat: "Ada Lovelace",
	});
	expect(container.querySelector("[data-caption]")?.textContent).toContain(
		"Ada Lovelace",
	);
	expect(container.querySelector("[data-caption]")?.textContent).toContain(
		"AL",
	);
	expect(batches).toEqual([[otherSub]]);
	expect(JSON.stringify(nodes)).toBe(before);
}, 30_000);
