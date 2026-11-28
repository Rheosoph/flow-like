import { describe, expect, test } from "bun:test";
import {
	CATALOG_WIRE_VERSION,
	CatalogReader,
	type ICatalogSource,
	decodeCatalog,
} from "./catalog-wire";

const builtIn = {
	schemas: ['{"type":"object"}'],
	node_defaults: { category: "Tests", coordinates: null, docs: null },
	pin_defaults: {
		connected_to: [],
		data_type: "Execution",
		default_value: null,
		depends_on: [],
		options: null,
		pin_type: "Input",
		schema: null,
		value_type: "Normal",
	},
	option_defaults: {
		enforce_schema: null,
		sensitive: null,
		valid_values: null,
	},
	nodes: [
		{
			name: "alpha",
			friendly_name: "Alpha",
			description: "First",
			pins: [
				{ name: "exec_in", friendly_name: "In", description: "", index: 1 },
				{
					name: "payload",
					friendly_name: "Payload",
					description: "",
					index: 2,
					data_type: "Struct",
					schema: 0,
					options: { enforce_schema: true },
				},
				{
					name: "result",
					friendly_name: "Result",
					description: "",
					index: 1,
					pin_type: "Output",
					data_type: "Struct",
					schema: 0,
				},
			],
		},
		{
			name: "beta",
			friendly_name: "Beta",
			description: "Second",
			category: "Other",
			pins: [],
		},
	],
};

const presets = {
	schemas: [],
	node_defaults: {},
	pin_defaults: { connected_to: [], depends_on: [] },
	option_defaults: {},
	nodes: [
		{
			id: "preset_one",
			name: "alpha",
			friendly_name: "Alpha for orders",
			description: "",
			category: "Presets",
			pins: [{ name: "exec_in", index: 1 }],
		},
	],
};

const document = (
	sections: unknown[],
	extra: { built_in?: string; etag?: string } = {},
) => ({ v: CATALOG_WIRE_VERSION, ...extra, sections });

const nodesOf = (...sections: unknown[]) =>
	decodeCatalog(document(sections))?.nodes ?? [];

describe("decodeCatalog", () => {
	test("restores every field a node did not state", () => {
		const [alpha, beta] = nodesOf(builtIn);

		expect(alpha.category).toBe("Tests");
		expect(alpha.coordinates).toBeNull();
		expect(beta.category).toBe("Other");

		const pins = Object.values(alpha.pins);
		expect(pins.map((pin) => pin.name)).toEqual([
			"exec_in",
			"payload",
			"result",
		]);
		expect(pins[0]).toMatchObject({
			pin_type: "Input",
			data_type: "Execution",
			value_type: "Normal",
			schema: null,
			options: null,
			default_value: null,
			connected_to: [],
			depends_on: [],
		});
		expect(pins[1].schema).toBe('{"type":"object"}');
		expect(pins[1].options).toEqual({
			enforce_schema: true,
			sensitive: null,
			valid_values: null,
		});
		expect(pins[2]).toMatchObject({
			pin_type: "Output",
			schema: '{"type":"object"}',
		});
	});

	test("gives every node and pin an id of its own, the same on each decode", () => {
		const first = nodesOf(builtIn, presets);
		expect(first).toEqual(nodesOf(builtIn, presets));

		const ids = first.flatMap((node) => [node.id, ...Object.keys(node.pins)]);
		expect(new Set(ids).size).toBe(ids.length);
		for (const node of first) {
			for (const [key, pin] of Object.entries(node.pins)) {
				expect(pin.id).toBe(key);
			}
		}
	});

	test("keeps the id and the position of a node that states one", () => {
		const nodes = nodesOf(builtIn, presets);
		expect(nodes.map((node) => node.id)).toEqual([
			"catalog:alpha",
			"catalog:beta",
			"preset_one",
		]);
		expect(nodes[2].name).toBe("alpha");
	});

	test("never shares a default list between two pins", () => {
		const [alpha] = nodesOf(builtIn);
		const [first, second] = Object.values(alpha.pins);
		first.connected_to.push("somewhere");
		expect(second.connected_to).toEqual([]);
	});

	test("keeps a stated null where the default is a list", () => {
		const [node] = nodesOf({
			...presets,
			nodes: [
				{
					id: "preset_one",
					name: "alpha",
					pins: [{ name: "exec_in", index: 1, depends_on: null }],
				},
			],
		});
		const [pin] = Object.values(node.pins);
		expect(pin.depends_on).toBeNull();
		expect(pin.connected_to).toEqual([]);
	});

	test("keeps pin ids apart when two package nodes state the same id", () => {
		const nodes = nodesOf({
			...presets,
			nodes: [presets.nodes[0], presets.nodes[0]],
		});
		expect(nodes.map((node) => node.id)).toEqual(["preset_one", "preset_one"]);
		const pinIds = nodes.flatMap((node) => Object.keys(node.pins));
		expect(new Set(pinIds).size).toBe(pinIds.length);
	});

	test("reports which built-in catalog a document belongs to and its tag", () => {
		const part = decodeCatalog(
			document([presets], { built_in: "abc", etag: '"tag"' }),
		);
		expect(part?.builtIn).toBe("abc");
		expect(part?.etag).toBe('"tag"');
	});

	test("passes the plain list of an older hub through", () => {
		const plain = [{ id: "a", name: "alpha", pins: {} }];
		const part = decodeCatalog(plain);
		expect(part?.nodes as unknown).toBe(plain);
		expect(part?.builtIn).toBeUndefined();
	});

	test("reports a layout it does not know instead of guessing", () => {
		expect(
			decodeCatalog({ v: CATALOG_WIRE_VERSION + 1, parts: [] }),
		).toBeUndefined();
	});

	test("refuses an answer that is no catalog", () => {
		expect(() => decodeCatalog({ error: "upstream" })).toThrow();
		expect(() =>
			decodeCatalog({ v: CATALOG_WIRE_VERSION, sections: [{}] }),
		).toThrow();
	});
});

/** A hub with one built-in catalog and per-app presets, counting what is asked of it. */
function hub(digest = "digest-1") {
	const calls = { app: 0, builtIn: 0, plain: 0 };
	const tags = new Map<string, string>();
	const source: ICatalogSource = {
		app: async (appId, etag) => {
			calls.app++;
			const current = tags.get(appId) ?? `"${appId}-v1"`;
			if (etag === current) return { notModified: true };
			return {
				notModified: false,
				data: document([presets], { built_in: digest, etag: current }),
			};
		},
		builtIn: async (wanted) => {
			calls.builtIn++;
			return document([builtIn], { built_in: wanted });
		},
		plain: async () => {
			calls.plain++;
			return [{ id: "plain", name: "plain", pins: {} }] as never;
		},
	};
	return { source, calls, tags };
}

describe("CatalogReader", () => {
	test("puts an app's own nodes after the built-in ones", async () => {
		const { source } = hub();
		const nodes = await new CatalogReader(source).read("app-a");
		expect(nodes.map((node) => node.id)).toEqual([
			"catalog:alpha",
			"catalog:beta",
			"preset_one",
		]);
	});

	test("fetches the built-in catalog once for all apps and shares its nodes", async () => {
		const { source, calls } = hub();
		const reader = new CatalogReader(source);
		const [first, second] = await Promise.all([
			reader.read("app-a"),
			reader.read("app-b"),
		]);

		expect(calls.builtIn).toBe(1);
		expect(first[0]).toBe(second[0]);
		expect(first[2]).not.toBe(second[2]);
	});

	test("returns the held list untouched when the source says nothing changed", async () => {
		const { source, calls, tags } = hub();
		const reader = new CatalogReader(source);
		const first = await reader.read("app-a");
		expect(await reader.read("app-a")).toBe(first);
		expect(calls.app).toBe(2);

		tags.set("app-a", '"app-a-v2"');
		const changed = await reader.read("app-a");
		expect(changed).not.toBe(first);
		expect(calls.builtIn).toBe(1);
	});

	test("uses a plain list as it is and asks for no built-in catalog", async () => {
		const { source, calls } = hub();
		const plain = [{ id: "a", name: "alpha", pins: {} }];
		source.app = async () => ({ notModified: false, data: plain });
		expect((await new CatalogReader(source).read("app-a")) as unknown).toBe(
			plain,
		);
		expect(calls.builtIn).toBe(0);
	});

	test("falls back to the plain list when the built-in catalog is another build's", async () => {
		const { source, calls } = hub();
		source.builtIn = async () => document([builtIn], { built_in: "other" });
		const reader = new CatalogReader(source);
		const nodes = await reader.read("app-a");
		expect(nodes.map((node) => node.id)).toEqual(["plain"]);

		// The failed read is not remembered: the next app asks again.
		source.builtIn = async (wanted) => {
			calls.builtIn++;
			return document([builtIn], { built_in: wanted });
		};
		expect((await reader.read("app-b")).length).toBe(3);
		expect(calls.builtIn).toBe(1);
	});

	test("asks for the plain list when the layout is unknown", async () => {
		const { source, calls } = hub();
		source.app = async () => ({
			notModified: false,
			data: { v: CATALOG_WIRE_VERSION + 1 },
		});
		await new CatalogReader(source).read("app-a");
		expect(calls.plain).toBe(1);
	});
});
