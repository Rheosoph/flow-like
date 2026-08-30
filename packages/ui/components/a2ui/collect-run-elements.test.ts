import {
	afterEach,
	beforeEach,
	describe,
	expect,
	mock,
	spyOn,
	test,
} from "bun:test";
import type { BoardVersion } from "../../lib/schema/flow/board-version";
import type { IElementDemand } from "../../lib/schema/flow/element-demand";
import {
	collectRunElements,
	resetRunElementDemandCache,
} from "./collect-run-elements";
import type { SurfaceComponent } from "./types";
import {
	flattenSurfaceComponentsForElements,
	mergeStoredElementValues,
} from "./workflow-elements";

const SURFACE = "page-1";
const APP = "app-1";
const BOARD = "board-1";

function component(
	id: string,
	data: Record<string, unknown>,
): SurfaceComponent {
	return { id, component: data } as unknown as SurfaceComponent;
}

const components: Record<string, SurfaceComponent> = {
	root: component("root", {
		type: "column",
		children: { explicitList: ["title", "field", "host"] },
	}),
	title: component("title", { type: "text" }),
	field: component("field", {
		type: "textField",
		value: { literalString: "" },
	}),
	host: component("host", {
		type: "widgetInstance",
		instanceId: "inst-1",
		widgetId: "shared-widget",
		inlineWidgetDef: {
			rootComponentId: "child",
			components: [
				{
					id: "child",
					eventRelevant: true,
					component: { type: "textField", value: { literalString: "" } },
				},
			],
		},
	}),
};

const storedValues = {
	[`${SURFACE}/field`]: "typed",
	"inst-1/child": "inner",
};

const demandOf = (selectors: string[]): IElementDemand => ({
	selectors,
	dynamic: false,
	signature: selectors.join("|"),
});

type DemandFetcher = (
	appId: string,
	boardId: string,
	version?: BoardVersion,
) => Promise<IElementDemand>;

function backendWith(fetcher: DemandFetcher) {
	const getElementDemand = mock(fetcher);
	return { backend: { boardState: { getElementDemand } }, getElementDemand };
}

function collect(
	backend: { boardState: Record<string, unknown> },
	overrides: Record<string, unknown> = {},
) {
	return collectRunElements({
		backend,
		appId: APP,
		boardId: BOARD,
		surfaceId: SURFACE,
		components,
		storedValues,
		...overrides,
	});
}

const fullMap = () =>
	mergeStoredElementValues(
		flattenSurfaceComponentsForElements(components, SURFACE),
		storedValues,
		components,
		SURFACE,
	);

const boundValue = (elements: Record<string, unknown>, key: string) =>
	(
		(elements[key] as Record<string, unknown>).component as Record<
			string,
			unknown
		>
	).value;

let warn: ReturnType<typeof spyOn>;

beforeEach(() => {
	resetRunElementDemandCache();
	warn = spyOn(console, "warn").mockImplementation(() => undefined);
});

afterEach(() => {
	warn.mockRestore();
});

describe("collectRunElements with a demand", () => {
	test("sends the demanded elements plus the triggering element", async () => {
		const { backend } = backendWith(async () => demandOf(["title"]));
		const elements = await collect(backend, { triggeringComponentId: "field" });

		expect(Object.keys(elements).sort()).toEqual([
			`${SURFACE}/field`,
			`${SURFACE}/title`,
		]);
		expect(boundValue(elements, `${SURFACE}/field`)).toEqual({
			literalString: "typed",
		});
	});

	test("an empty demand with a trigger sends only the trigger", async () => {
		const { backend } = backendWith(async () => demandOf([]));
		const elements = await collect(backend, { triggeringComponentId: "title" });
		expect(Object.keys(elements)).toEqual([`${SURFACE}/title`]);
	});

	test("an empty demand without a trigger sends nothing", async () => {
		const { backend } = backendWith(async () => demandOf([]));
		expect(await collect(backend)).toEqual({});
	});

	test("addresses the trigger under the widget instance of a scoped run", async () => {
		const { backend } = backendWith(async () => demandOf(["title"]));
		const elements = await collect(backend, {
			widgetScope: { instanceId: "inst-1" },
			triggeringComponentId: "child",
		});

		expect(Object.keys(elements).sort()).toEqual([
			"inst-1/child",
			`${SURFACE}/title`,
		]);
		expect(boundValue(elements, "inst-1/child")).toEqual({
			literalString: "inner",
		});
	});

	test("a scoped run never resolves selectors to the legacy page key of a child", async () => {
		const { backend } = backendWith(async () => demandOf([`${SURFACE}/child`]));
		const elements = await collect(backend, {
			widgetScope: { instanceId: "inst-1" },
		});
		expect(Object.keys(elements)).toEqual([]);
	});
});

describe("collectRunElements fallback", () => {
	test("sends the full surface when the backend cannot answer", async () => {
		expect(await collect({ boardState: {} })).toEqual(fullMap());
	});

	test("sends the full surface when the demand request rejects", async () => {
		const { backend, getElementDemand } = backendWith(async () => {
			throw new Error("offline");
		});
		expect(await collect(backend)).toEqual(fullMap());
		expect(getElementDemand).toHaveBeenCalledTimes(1);
		expect(warn).toHaveBeenCalledTimes(1);
	});

	test("sends the full surface when the demand request throws", async () => {
		const backend = {
			boardState: {
				getElementDemand: () => {
					throw new Error("no ipc");
				},
			},
		};
		expect(await collect(backend)).toEqual(fullMap());
	});

	test("sends the full surface without an app or board and asks nothing", async () => {
		const { backend, getElementDemand } = backendWith(async () =>
			demandOf(["title"]),
		);
		expect(await collect(backend, { boardId: undefined })).toEqual(fullMap());
		expect(await collect(backend, { appId: "" })).toEqual(fullMap());
		expect(getElementDemand).toHaveBeenCalledTimes(0);
	});

	test("retries after a rejected first fetch", async () => {
		let attempts = 0;
		const { backend, getElementDemand } = backendWith(async () => {
			attempts += 1;
			if (attempts === 1) throw new Error("flaky");
			return demandOf(["title"]);
		});

		expect(await collect(backend)).toEqual(fullMap());
		expect(Object.keys(await collect(backend))).toEqual([`${SURFACE}/title`]);
		expect(getElementDemand).toHaveBeenCalledTimes(2);
	});
});

describe("collectRunElements cache", () => {
	test("serves a cached demand without a second request", async () => {
		const { backend, getElementDemand } = backendWith(async () =>
			demandOf(["title"]),
		);
		await collect(backend);
		await collect(backend);
		expect(getElementDemand).toHaveBeenCalledTimes(1);
	});

	test("concurrent first requests share one fetch", async () => {
		const { backend, getElementDemand } = backendWith(async () =>
			demandOf(["title"]),
		);
		const [first, second] = await Promise.all([
			collect(backend),
			collect(backend),
		]);
		expect(first).toEqual(second);
		expect(getElementDemand).toHaveBeenCalledTimes(1);
	});

	test("refresh bypasses the cache and picks up the new demand", async () => {
		let selectors = ["title"];
		const { backend, getElementDemand } = backendWith(async () =>
			demandOf(selectors),
		);
		await collect(backend);
		selectors = ["field"];

		expect(Object.keys(await collect(backend))).toEqual([`${SURFACE}/title`]);
		expect(Object.keys(await collect(backend, { refresh: true }))).toEqual([
			`${SURFACE}/field`,
		]);
		expect(getElementDemand).toHaveBeenCalledTimes(2);
	});

	test("keys the cache by app, board and version", async () => {
		const { backend, getElementDemand } = backendWith(async () =>
			demandOf(["title"]),
		);
		await collect(backend);
		await collect(backend, { boardVersion: [1, 2, 3] });
		await collect(backend, { boardVersion: [1, 2, 3] });
		await collect(backend, { boardId: "board-2" });
		expect(getElementDemand).toHaveBeenCalledTimes(3);
		expect(getElementDemand.mock.calls[1]).toEqual([APP, BOARD, [1, 2, 3]]);
	});

	test("resetRunElementDemandCache forces a new request", async () => {
		const { backend, getElementDemand } = backendWith(async () =>
			demandOf(["title"]),
		);
		await collect(backend);
		resetRunElementDemandCache();
		await collect(backend);
		expect(getElementDemand).toHaveBeenCalledTimes(2);
	});
});
