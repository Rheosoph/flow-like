// @vitest-environment happy-dom
import React, {
	act,
	type ComponentProps,
	type ReactElement,
	type ReactNode,
} from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
	GraphOverlay,
	RemoteOntologyImport,
} from "../../../../state/backend-state/graph-state";
import type {
	ExecuteSqlResult,
	SavedQuery,
} from "../../../../state/backend-state/query-state";
import type { SqlEditorProps } from "../../../ui/sql-editor";
import { TooltipProvider } from "../../../ui/tooltip";
import type { QueryResultView } from "./query-result-view";
import { QueryWorkbench } from "./query-workbench";
import type { SavedQuerySidebar } from "./saved-query-sidebar";

type ResultProps = ComponentProps<typeof QueryResultView>;
const fixture = vi.hoisted(() => ({
	query: { executeSql: vi.fn(), listSavedQueries: vi.fn() },
	graph: { queryRemoteImport: vi.fn() },
	db: { getSchema: vi.fn() },
	saved: [] as SavedQuery[],
	result: undefined as ResultProps | undefined,
	editor: undefined as SqlEditorProps | undefined,
	validParams: false,
	width: 1200,
}));
vi.mock("../../../../state/backend-state", () => ({
	useBackend: () => ({
		queryState: fixture.query,
		graphState: fixture.graph,
		dbState: fixture.db,
	}),
}));
vi.mock("../../../../hooks/use-app-permissions", () => ({
	useAppPermissions: () => ({ can: () => true, isLoading: false }),
}));
vi.mock("../../../../hooks/use-invoke", () => ({
	useInvoke: () => ({
		data: fixture.saved,
		isLoading: false,
		refetch: vi.fn(),
	}),
}));
vi.mock("../../permission/permission-gate", () => ({
	SectionLockedPanel: () => null,
}));
vi.mock("../../permission/permission-notice", () => ({
	PermissionNotice: () => null,
}));
vi.mock("../data-studio-panels", () => ({
	humanizeIdentifier: (value: string) => value,
	OntologyActionParameterForm: ({
		onValidityChange,
	}: { onValidityChange: (valid: boolean) => void }) => {
		React.useEffect(
			() => onValidityChange(fixture.validParams),
			[onValidityChange],
		);
		return <div>Query parameters</div>;
	},
}));
vi.mock("../../../ui/sql-editor", async () => {
	const language = await import("../../../ui/sql-editor/sql-language");
	return {
		extractParams: language.extractParams,
		extractReferencedTables: language.extractReferencedTables,
		SqlEditor: (props: SqlEditorProps) => {
			fixture.editor = props;
			return (
				<textarea
					aria-label="SQL"
					value={props.value}
					onChange={(event) => props.onChange(event.target.value)}
				/>
			);
		},
	};
});
vi.mock("./query-result-view", () => ({
	QueryResultView: (props: ResultProps) => {
		fixture.result = props;
		return (
			<output data-testid="results">
				{JSON.stringify({
					result: props.result,
					loading: props.loading,
					error: props.error,
					stale: props.stale,
				})}
			</output>
		);
	},
}));
vi.mock("./save-query-dialog", () => ({ SaveQueryDialog: () => null }));
vi.mock("./saved-query-sidebar", () => ({
	SavedQuerySidebar: ({
		queries,
		onSelect,
	}: ComponentProps<typeof SavedQuerySidebar>) => (
		<div>
			{queries.map((query) => (
				<button type="button" key={query.id} onClick={() => onSelect(query)}>
					{query.name}
				</button>
			))}
		</div>
	),
}));
vi.mock("../../../ui/select", () => ({
	Select: ({
		value,
		onValueChange,
		children,
	}: {
		value: string;
		onValueChange: (value: string) => void;
		children: ReactNode;
	}) => {
		const [trigger, content] = React.Children.toArray(
			children,
		) as ReactElement<{ "aria-label"?: string; children?: ReactNode }>[];
		return (
			<select
				aria-label={trigger.props["aria-label"]}
				value={value}
				onChange={(event) => onValueChange(event.target.value)}
			>
				{content.props.children}
			</select>
		);
	},
	SelectTrigger: () => null,
	SelectValue: () => null,
	SelectContent: () => null,
	SelectItem: ({ value, disabled }: ComponentProps<"option">) => (
		<option value={value} disabled={disabled}>
			{value}
		</option>
	),
}));
// Panel sizing needs a browser. These handles retain the real open/close callbacks
// while the tests exercise query execution and navigation between drafts.
vi.mock("react-resizable-panels", () => ({
	Panel: React.forwardRef(
		(
			{
				children,
				defaultSize,
				onCollapse,
				onExpand,
			}: {
				children: ReactNode;
				defaultSize?: number;
				onCollapse?: () => void;
				onExpand?: () => void;
			},
			ref,
		) => {
			const collapsed = React.useRef(defaultSize === 0);
			React.useImperativeHandle(ref, () => ({
				isCollapsed: () => collapsed.current,
				collapse: () => {
					collapsed.current = true;
					onCollapse?.();
				},
				expand: () => {
					collapsed.current = false;
					onExpand?.();
				},
			}));
			return <div>{children}</div>;
		},
	),
}));
vi.mock("../../../ui/resizable", () => ({
	ResizablePanelGroup: ({ children }: { children: ReactNode }) => (
		<div>{children}</div>
	),
	ResizableHandle: () => null,
}));

const savedQuery: SavedQuery = {
	id: "saved",
	app_id: "project-a",
	name: "Saved query",
	sql: "SELECT 2",
	kind: "query",
	surface: "native",
	created_at: "2026-01-01T00:00:00Z",
	updated_at: "2026-01-01T00:00:00Z",
};
const ontology: GraphOverlay = {
	id: "ontology-a",
	name: "Customers",
	nodes: [],
	edges: [],
	object_views: [],
	actions: [],
	exposed: false,
	bindings_enabled: false,
	default_limit: 1000,
	created_at: "2026-01-01T00:00:00Z",
	updated_at: "2026-01-01T00:00:00Z",
};
const remote: RemoteOntologyImport = {
	id: "remote-a",
	target_app_id: "remote-project",
	remote_ontology_id: ontology.id,
	contract: ontology,
	source_updated_at: ontology.updated_at,
	bindings_enabled: false,
	installed_at: ontology.created_at,
	updated_at: ontology.updated_at,
};
const response = (value: string): ExecuteSqlResult => ({
	columns: [{ name: "value", type_name: "Utf8", position: 0 }],
	rows: [{ value }],
	row_count: 1,
	truncated: false,
});
const defaults: ComponentProps<typeof QueryWorkbench> = {
	appId: "project-a",
	ontologies: [],
	projectTables: [],
	userTables: [],
	userScoped: false,
	onScopeChange: vi.fn(),
};
let root: Root;
let container: HTMLDivElement;
function required<T>(value: T | null | undefined): T {
	if (value == null) throw new Error("Expected value");
	return value;
}
function button(label: string) {
	return required(
		[...document.querySelectorAll<HTMLButtonElement>("button")].find(
			(node) =>
				node.getAttribute("aria-label") === label ||
				node.textContent?.trim() === label,
		),
	);
}
async function render(
	props: Partial<ComponentProps<typeof QueryWorkbench>> = {},
) {
	await act(async () =>
		root.render(
			<TooltipProvider>
				<QueryWorkbench {...defaults} {...props} />
			</TooltipProvider>,
		),
	);
}
async function sql(value: string) {
	await act(async () => fixture.editor?.onChange(value));
}
async function select(label: string, value: string) {
	const element = required(
		document.querySelector<HTMLSelectElement>(`select[aria-label="${label}"]`),
	);
	await act(async () => {
		element.value = value;
		element.dispatchEvent(new Event("change", { bubbles: true }));
	});
}
async function editorCommand() {
	await act(async () => fixture.editor?.onRun?.());
}
async function shortcut() {
	await act(async () =>
		required(container.querySelector("textarea")).dispatchEvent(
			new KeyboardEvent("keydown", {
				key: "Enter",
				ctrlKey: true,
				bubbles: true,
			}),
		),
	);
}
function deferred<T>() {
	let resolve!: (value: T) => void;
	let reject!: (reason: Error) => void;
	const promise = new Promise<T>((res, rej) => {
		resolve = res;
		reject = rej;
	});
	return { promise, resolve, reject };
}

beforeEach(() => {
	Object.assign(globalThis, { React, IS_REACT_ACT_ENVIRONMENT: true });
	fixture.query.executeSql.mockReset();
	fixture.graph.queryRemoteImport.mockReset();
	fixture.db.getSchema.mockReset();
	fixture.saved = [savedQuery];
	fixture.result = undefined;
	fixture.editor = undefined;
	fixture.validParams = false;
	fixture.width = 1200;
	vi.stubGlobal(
		"ResizeObserver",
		class {
			constructor(private callback: ResizeObserverCallback) {}
			observe(target: Element) {
				this.callback(
					[
						{
							target,
							contentRect: { width: fixture.width },
						} as ResizeObserverEntry,
					],
					this as unknown as ResizeObserver,
				);
			}
			unobserve() {}
			disconnect() {}
		},
	);
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});
afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
	vi.unstubAllGlobals();
});

describe("query workbench execution", () => {
	it("blocks both editor and bubbling shortcuts for empty SQL or invalid parameters", async () => {
		await render();
		await sql("   ");
		await editorCommand();
		await shortcut();
		expect(fixture.query.executeSql).not.toHaveBeenCalled();
		await sql("SELECT $name");
		expect(fixture.result?.runDisabled).toBe(true);
		await editorCommand();
		await shortcut();
		expect(fixture.query.executeSql).not.toHaveBeenCalled();
	});

	it("accepts a valid shortcut once while its request is pending", async () => {
		const request = deferred<ExecuteSqlResult>();
		fixture.query.executeSql.mockReturnValue(request.promise);
		await render();
		await act(async () => {
			fixture.editor?.onRun?.();
			fixture.editor?.onRun?.();
		});
		await shortcut();
		expect(fixture.query.executeSql).toHaveBeenCalledOnce();
		expect(fixture.result?.loading).toBe(true);
		await act(async () => request.resolve(response("current")));
		expect(fixture.result?.loading).toBe(false);
		expect(fixture.result?.result?.rows).toEqual([{ value: "current" }]);
	});

	it("ignores a late response after opening a saved query while a new run is pending", async () => {
		const previous = deferred<ExecuteSqlResult>();
		const current = deferred<ExecuteSqlResult>();
		fixture.query.executeSql
			.mockReturnValueOnce(previous.promise)
			.mockReturnValueOnce(current.promise);
		await render();
		await editorCommand();
		await act(async () => button("Toggle saved queries").click());
		await act(async () => button("Saved query").click());
		expect(fixture.editor?.value).toBe("SELECT 2");
		await editorCommand();
		await act(async () => previous.resolve(response("previous")));
		expect(fixture.result?.loading).toBe(true);
		expect(fixture.result?.result).toBeNull();
		await act(async () => current.resolve(response("current")));
		expect(fixture.result?.result?.rows).toEqual([{ value: "current" }]);
		expect(fixture.result?.stale).toBe(false);
	});

	it("ignores a late failure after starting a new query", async () => {
		const request = deferred<ExecuteSqlResult>();
		fixture.query.executeSql.mockReturnValue(request.promise);
		await render();
		await editorCommand();
		await act(async () => button("New query").click());
		await act(async () => request.reject(new Error("Previous query failed")));
		expect(fixture.result?.loading).toBe(false);
		expect(fixture.result?.error).toBeNull();
		expect(fixture.result?.result).toBeNull();
	});

	it("discards a pending result when database scope changes", async () => {
		const previous = deferred<ExecuteSqlResult>();
		fixture.query.executeSql
			.mockReturnValueOnce(previous.promise)
			.mockResolvedValueOnce(response("personal"));
		await render();
		await editorCommand();
		await render({ userScoped: true });
		await act(async () => previous.resolve(response("project")));
		expect(fixture.result?.result).toBeNull();
		expect(fixture.result?.loading).toBe(false);
		await editorCommand();
		expect(fixture.query.executeSql.mock.calls[1][2]).toBe(true);
		expect(fixture.result?.result?.rows).toEqual([{ value: "personal" }]);
	});

	it("marks completed results stale when SQL changes during execution", async () => {
		const request = deferred<ExecuteSqlResult>();
		fixture.query.executeSql.mockReturnValue(request.promise);
		await render();
		await editorCommand();
		await sql("SELECT 2");
		await act(async () => request.resolve(response("earlier SQL")));
		expect(fixture.result?.result?.rows).toEqual([{ value: "earlier SQL" }]);
		expect(fixture.result?.stale).toBe(true);
		await sql("SELECT 1");
		expect(fixture.result?.stale).toBe(false);
	});

	it("discards the saved draft and pending response when switching projects", async () => {
		const previous = deferred<ExecuteSqlResult>();
		fixture.query.executeSql
			.mockReturnValueOnce(previous.promise)
			.mockResolvedValueOnce(response("new project"));
		await render();
		await act(async () => button("Toggle saved queries").click());
		await act(async () => button("Saved query").click());
		await editorCommand();
		await render({ appId: "project-b" });
		expect(fixture.editor?.value).toBe("SELECT 1");
		expect(container.querySelector("header h3")?.textContent).toBe(
			"Untitled query",
		);
		await act(async () => previous.resolve(response("old project")));
		expect(fixture.result?.result).toBeNull();
		expect(fixture.result?.loading).toBe(false);
		await editorCommand();
		expect(fixture.query.executeSql.mock.calls[1][0]).toBe("project-b");
		expect(fixture.result?.result?.rows).toEqual([{ value: "new project" }]);
	});

	it("treats remembered ontology selections as inactive after returning to native SQL", async () => {
		fixture.query.executeSql.mockResolvedValue(response("native"));
		await render({ ontologies: [ontology], remoteImports: [remote] });
		await editorCommand();
		expect(fixture.result?.stale).toBe(false);
		await select("Query source", "overlay");
		expect(fixture.result?.stale).toBe(true);
		await select("Query source", "native");
		expect(fixture.result?.stale).toBe(false);
		await select("Query source", "remote");
		expect(fixture.result?.stale).toBe(true);
		await select("Query source", "native");
		expect(fixture.result?.stale).toBe(false);
	});

	it("blocks missing parameters before the narrow parameter drawer is opened", async () => {
		fixture.width = 500;
		await render();
		await sql("SELECT $name");
		expect(container.textContent).not.toContain("Query parameters");
		expect(fixture.result?.runDisabled).toBe(true);
		await editorCommand();
		await shortcut();
		expect(fixture.query.executeSql).not.toHaveBeenCalled();
	});

	it("refreshes a same-name table schema when switching projects", async () => {
		fixture.db.getSchema.mockImplementation(async (appId: string) => ({
			fields: [
				{
					name: appId === "project-a" ? "old_column" : "new_column",
					type: "Utf8",
				},
			],
		}));
		await render({ projectTables: ["data"] });
		await sql("SELECT * FROM data");
		expect(fixture.editor?.catalog.tables[0].columns?.[0].name).toBe(
			"old_column",
		);
		await render({ appId: "project-b", projectTables: ["data"] });
		await sql("SELECT * FROM data");
		expect(fixture.db.getSchema).toHaveBeenCalledWith(
			"project-b",
			"data",
			false,
		);
		expect(fixture.editor?.catalog.tables[0].columns?.[0].name).toBe(
			"new_column",
		);
	});
});
