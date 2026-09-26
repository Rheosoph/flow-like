import { afterEach, expect, test } from "bun:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Window } from "happy-dom";
import { type ReactNode, act } from "react";
import type { ObjectEditField } from "../../../lib/ontology-object-edit";
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
		CustomEvent: window.CustomEvent,
		InputEvent: window.InputEvent,
		KeyboardEvent: window.KeyboardEvent,
		FocusEvent: window.FocusEvent,
		PointerEvent: window.PointerEvent,
		NodeFilter: window.NodeFilter,
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
	const type = async (input: HTMLInputElement, value: string) => {
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
	const click = async (target: Element | null | undefined) => {
		await act(async () => {
			target?.dispatchEvent(
				new window.MouseEvent("click", { bubbles: true }) as never,
			);
		});
	};
	return {
		client,
		container: container as unknown as HTMLElement,
		type,
		press,
		click,
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

test("declared types and the owning app turn bytes, geometries and paths into their smart readings", async () => {
	const { container, render } = await setup();
	const { PropertyRow, PropertyStorageScope } = await import(
		"./graph-node-inspector"
	);
	const kindOf = (field: string) =>
		container.querySelector(`[data-field="${field}"] span.shrink-0`)
			?.textContent;
	await render(
		<>
			<div data-field="bytes">
				<PropertyRow propKey="payload" typeName="Binary" value={[123, 125]} />
			</div>
			<div data-field="numbers">
				<PropertyRow propKey="payload" value={[123, 125]} />
			</div>
			<div data-field="outline">
				<PropertyRow propKey="outline" typeName="Binary" value={POINT} />
			</div>
			<div data-field="unscoped">
				<PropertyRow
					propKey="report"
					value="apps/app-1/upload/reports/q3.pdf"
				/>
			</div>
			<PropertyStorageScope value="app-1">
				<div data-field="file">
					<PropertyRow
						propKey="report"
						value="apps/app-1/upload/reports/q3.pdf"
					/>
				</div>
				<div data-field="foreign">
					<PropertyRow
						propKey="report"
						value="apps/app-2/upload/reports/q3.pdf"
					/>
				</div>
			</PropertyStorageScope>
		</>,
	);
	expect(kindOf("bytes")).toBe("binary");
	expect(container.querySelector('[data-field="bytes"] canvas')).toBeNull();
	expect(kindOf("numbers")).toBe("vector");
	expect(kindOf("outline")).toBe("geometry");
	expect(
		container.querySelector('[data-field="outline"] button')?.textContent,
	).toContain("Point");
	expect(kindOf("unscoped")).toBe("string");
	expect(kindOf("file")).toBe("file");
	expect(
		container.querySelector('[data-field="file"] button')?.textContent,
	).toContain("q3.pdf");
	expect(kindOf("foreign")).toBe("string");
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

type Updates = Record<string, unknown>;

function editField(
	name: string,
	kind: "string" | "number",
	integer = false,
): ObjectEditField {
	return { name, kind, integer, nullable: true, temporal: null };
}

const IDENTITY_LOCK =
	"This column identifies the object, so it can't be edited here.";
const RELATIONSHIP_LOCK =
	"This column links objects together, so it can't be edited here.";

const PERSON_OVERLAY = {
	nodes: [
		{
			label: "Person",
			table: "people",
			id_column: "id",
			display_column: "name",
			property_columns: [],
		},
	],
	edges: [],
	object_views: [],
	actions: [],
} as unknown as GraphOverlay;

const PERSON: SubgraphNode = {
	id: "Person:1",
	label: "Person",
	caption: "Ada",
	props: { id: 1, name: "Ada", role: "Engineer" },
};

const PERSON_FIELDS: ReadonlyMap<string, ObjectEditField> = new Map([
	["id", editField("id", "number", true)],
	["name", editField("name", "string")],
	["role", editField("role", "string")],
]);

function affordances(container: Element) {
	return {
		pencils: container.querySelectorAll("svg.lucide-pencil").length,
		locks: container.querySelectorAll("svg.lucide-lock").length,
	};
}

function buttonLabelled(container: Element, label: string) {
	return container.querySelector<HTMLButtonElement>(
		`button[aria-label="${label}"]`,
	);
}

test("inspectors stay read-only without an update callback or loaded column types", async () => {
	const { container, render } = await setup();
	const { GraphNodeInspector } = await import("./graph-node-inspector");
	await render(
		<GraphNodeInspector
			node={PERSON}
			overlay={PERSON_OVERLAY}
			onClose={() => {}}
		/>,
	);
	const readOnly = container.innerHTML;
	expect(affordances(container)).toEqual({ pencils: 0, locks: 0 });

	await render(
		<GraphNodeInspector
			node={PERSON}
			overlay={PERSON_OVERLAY}
			editFields={PERSON_FIELDS}
			onClose={() => {}}
		/>,
	);
	expect(container.innerHTML).toBe(readOnly);

	await render(
		<GraphNodeInspector
			node={PERSON}
			overlay={PERSON_OVERLAY}
			onUpdateProperties={async () => {}}
			onClose={() => {}}
		/>,
	);
	expect(affordances(container)).toEqual({ pencils: 0, locks: 0 });
	expect(container.textContent).not.toContain("can't be edited here");
}, 30_000);

test("an editable object locks its identity and saves one property inline", async () => {
	const { container, render, type, press, click } = await setup();
	const { GraphNodeInspector } = await import("./graph-node-inspector");
	const { StaleObjectError } = await import(
		"../../../lib/ontology-object-edit"
	);
	const calls: [Updates, Updates][] = [];
	let outcome: "error" | "stale" | "ok" = "error";
	await render(
		<GraphNodeInspector
			node={PERSON}
			overlay={PERSON_OVERLAY}
			editFields={PERSON_FIELDS}
			onUpdateProperties={async (updates, baseline) => {
				calls.push([updates, baseline]);
				if (outcome === "error") throw new Error("Server said no");
				if (outcome === "stale") {
					throw new StaleObjectError({ id: 1, name: "Theirs" });
				}
			}}
			onClose={() => {}}
		/>,
	);

	expect(buttonLabelled(container, "Edit name")).not.toBeNull();
	expect(buttonLabelled(container, "Edit id")).toBeNull();
	expect(buttonLabelled(container, IDENTITY_LOCK)).not.toBeNull();
	expect(affordances(container)).toEqual({ pencils: 2, locks: 1 });

	await click(buttonLabelled(container, "Edit name"));
	const input = container.querySelector("input") as HTMLInputElement;
	expect(input.value).toBe("Ada");
	expect(buttonLabelled(container, "Edit role")?.disabled).toBe(true);

	await type(input, "x");
	await press(input, "Enter");
	expect(calls).toEqual([[{ name: "x" }, PERSON.props]]);
	expect(container.querySelector('[role="alert"]')?.textContent).toContain(
		"Server said no",
	);
	expect(container.querySelector("input")?.value).toBe("x");

	outcome = "stale";
	await press(input, "Enter");
	expect(container.textContent).toContain("Someone changed this value");
	expect(container.textContent).toContain("Theirs");
	expect(container.querySelector("input")?.value).toBe("x");

	outcome = "ok";
	await press(input, "Enter");
	expect(calls).toHaveLength(3);
	expect(container.querySelector("input")).toBeNull();
	expect(buttonLabelled(container, "Edit role")?.disabled).toBe(false);
}, 30_000);

test("selecting another object drops the open editor", async () => {
	const { container, render, click } = await setup();
	const { GraphNodeInspector } = await import("./graph-node-inspector");
	const inspect = (node: SubgraphNode) =>
		render(
			<GraphNodeInspector
				node={node}
				overlay={PERSON_OVERLAY}
				editFields={PERSON_FIELDS}
				onUpdateProperties={async () => {}}
				onClose={() => {}}
			/>,
		);
	await inspect(PERSON);
	await click(buttonLabelled(container, "Edit role"));
	expect(container.querySelector("input")?.value).toBe("Engineer");

	await inspect({
		...PERSON,
		id: "Person:2",
		props: { id: 2, name: "Grace", role: "Admiral" },
	});
	expect(container.querySelector("input")).toBeNull();
	expect(container.textContent).toContain("Admiral");
	expect(buttonLabelled(container, "Edit name")?.disabled).toBe(false);
}, 30_000);

test("an object whose identity is not in view explains that it can't be edited", async () => {
	const { container, render } = await setup();
	const { GraphNodeInspector } = await import("./graph-node-inspector");
	await render(
		<GraphNodeInspector
			node={{ ...PERSON, props: { name: "Ada", role: "Engineer" } }}
			overlay={PERSON_OVERLAY}
			editFields={PERSON_FIELDS}
			onUpdateProperties={async () => {}}
			onClose={() => {}}
		/>,
	);
	expect(container.textContent).toContain("This object can't be edited here.");
	expect(affordances(container)).toEqual({ pencils: 0, locks: 0 });
}, 30_000);

test("a foreign-key relationship points to its owning object", async () => {
	const { container, render, click } = await setup();
	const { GraphEdgeInspector } = await import("./graph-edge-inspector");
	const overlay = {
		nodes: [
			{ label: "Employee", table: "employees", id_column: "id" },
			{ label: "Department", table: "departments", id_column: "id" },
		],
		edges: [
			{
				label: "WORKS_IN",
				table: "employees",
				src_column: "id",
				dst_column: "department_id",
				src_label: "Employee",
				dst_label: "Department",
				property_columns: [],
			},
		],
		object_views: [],
		actions: [],
	} as unknown as GraphOverlay;
	const opened: string[] = [];
	await render(
		<GraphEdgeInspector
			edge={{
				id: "works",
				source: "Employee:1",
				target: "Department:7",
				label: "WORKS_IN",
				props: { since: 2020 },
			}}
			overlay={overlay}
			sourceNode={{
				id: "Employee:1",
				label: "Employee",
				caption: "Ada",
				props: { id: 1 },
			}}
			targetNode={{
				id: "Department:7",
				label: "Department",
				caption: "Research",
				props: { id: 7 },
			}}
			editFields={new Map([["since", editField("since", "number", true)]])}
			onUpdateProperties={async () => {}}
			onOpenNode={(nodeId) => opened.push(nodeId)}
			onClose={() => {}}
		/>,
	);
	expect(container.textContent).toContain(
		"These values are stored on Ada. Open it to edit them.",
	);
	expect(affordances(container)).toEqual({ pencils: 0, locks: 0 });
	await click(
		[...container.querySelectorAll("button")].find(
			(button) => button.textContent === "Open Ada",
		),
	);
	expect(opened).toEqual(["Employee:1"]);
}, 30_000);

test("a join-table relationship edits its own properties but never its endpoints", async () => {
	const { container, render, type, press, click } = await setup();
	const { GraphEdgeInspector } = await import("./graph-edge-inspector");
	const overlay = {
		nodes: [
			{ label: "Person", table: "people", id_column: "id" },
			{ label: "Team", table: "teams", id_column: "id" },
		],
		edges: [
			{
				label: "MEMBER_OF",
				table: "memberships",
				src_column: "person_id",
				dst_column: "team_id",
				src_label: "Person",
				dst_label: "Team",
				property_columns: [],
			},
		],
		object_views: [],
		actions: [],
	} as unknown as GraphOverlay;
	const edge: SubgraphEdge = {
		id: "membership",
		source: "Person:1",
		target: "Team:2",
		label: "MEMBER_OF",
		props: { person_id: 1, team_id: 2, role: "lead" },
	};
	const calls: [Updates, Updates][] = [];
	await render(
		<GraphEdgeInspector
			edge={edge}
			overlay={overlay}
			sourceNode={{ id: "Person:1", label: "Person", props: { id: 1 } }}
			targetNode={{ id: "Team:2", label: "Team", props: { id: 2 } }}
			editFields={
				new Map([
					["person_id", editField("person_id", "number", true)],
					["team_id", editField("team_id", "number", true)],
					["role", editField("role", "string")],
				])
			}
			onUpdateProperties={async (updates, baseline) => {
				calls.push([updates, baseline]);
			}}
			onClose={() => {}}
		/>,
	);
	expect(container.textContent).not.toContain("stored on");
	expect(buttonLabelled(container, "Edit role")).not.toBeNull();
	expect(buttonLabelled(container, "Edit person_id")).toBeNull();
	expect(buttonLabelled(container, "Edit team_id")).toBeNull();
	expect(
		container.querySelectorAll(`button[aria-label="${RELATIONSHIP_LOCK}"]`),
	).toHaveLength(2);

	await click(buttonLabelled(container, "Edit role"));
	const input = container.querySelector("input") as HTMLInputElement;
	await type(input, "member");
	await press(input, "Enter");
	expect(calls).toEqual([[{ role: "member" }, edge.props]]);
	expect(container.querySelector("input")).toBeNull();
}, 30_000);

test("a save that settles after switching objects leaves the newer editor open", async () => {
	const { container, render, type, press, click } = await setup();
	const { GraphNodeInspector } = await import("./graph-node-inspector");
	let settle: (() => void) | undefined;
	const inspect = (node: SubgraphNode) =>
		render(
			<GraphNodeInspector
				node={node}
				overlay={PERSON_OVERLAY}
				editFields={PERSON_FIELDS}
				onUpdateProperties={() =>
					new Promise<void>((resolve) => {
						settle = resolve;
					})
				}
				onClose={() => {}}
			/>,
		);
	await inspect(PERSON);
	await click(buttonLabelled(container, "Edit name"));
	await type(container.querySelector("input") as HTMLInputElement, "x");
	await press(container.querySelector("input") as HTMLInputElement, "Enter");
	expect(settle).toBeDefined();
	const firstSave = settle;

	await inspect({
		...PERSON,
		id: "Person:2",
		props: { id: 2, name: "Grace", role: "Admiral" },
	});
	await click(buttonLabelled(container, "Edit role"));
	const input = container.querySelector("input") as HTMLInputElement;
	await type(input, "Rear Admiral");
	await act(async () => firstSave?.());

	expect(container.querySelector("input")).toBe(input);
	expect(input.value).toBe("Rear Admiral");
	expect(buttonLabelled(container, "Edit name")?.disabled).toBe(true);
}, 30_000);

test("closing an inline editor returns focus to its pencil unless focus moved on", async () => {
	const { container, render, type, press, click } = await setup();
	const { GraphNodeInspector } = await import("./graph-node-inspector");
	let settle: (() => void) | undefined;
	let deferred = false;
	await render(
		<>
			<input aria-label="elsewhere" />
			<GraphNodeInspector
				node={PERSON}
				overlay={PERSON_OVERLAY}
				editFields={PERSON_FIELDS}
				onUpdateProperties={() =>
					deferred
						? new Promise<void>((resolve) => {
								settle = resolve;
							})
						: Promise.resolve()
				}
				onClose={() => {}}
			/>
		</>,
	);
	const focused = () => container.ownerDocument.activeElement;

	await click(buttonLabelled(container, "Edit name"));
	const nameInput = container.querySelector(
		'input[aria-label="name"]',
	) as HTMLInputElement;
	expect(focused()).toBe(nameInput);
	await type(nameInput, "x");
	await press(nameInput, "Enter");
	expect(container.querySelector('input[aria-label="name"]')).toBeNull();
	expect(focused()).toBe(buttonLabelled(container, "Edit name"));

	await click(buttonLabelled(container, "Edit role"));
	await press(
		container.querySelector('input[aria-label="role"]') as Element,
		"Escape",
	);
	expect(focused()).toBe(buttonLabelled(container, "Edit role"));

	deferred = true;
	await click(buttonLabelled(container, "Edit role"));
	const roleInput = container.querySelector(
		'input[aria-label="role"]',
	) as HTMLInputElement;
	await type(roleInput, "Lead");
	await press(roleInput, "Enter");
	const elsewhere = container.querySelector(
		'input[aria-label="elsewhere"]',
	) as HTMLInputElement;
	await act(async () => elsewhere.focus());
	await act(async () => settle?.());
	expect(container.querySelector('input[aria-label="role"]')).toBeNull();
	expect(focused()).toBe(elsewhere);
}, 30_000);
