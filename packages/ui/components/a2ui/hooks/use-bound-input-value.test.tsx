import {
	afterAll,
	afterEach,
	beforeEach,
	describe,
	expect,
	mock,
	test,
} from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import type { BoundValue } from "../types";

const pathData: Record<string, unknown> = {};
const setByPath = mock((path: string, value: unknown) => {
	pathData[path] = value;
});

/** Mirrors DataContext.resolve, including its raw-primitive and re-parsing branches. */
const resolveBoundValue = (value: BoundValue) => {
	if (value === null || value === undefined) return value;
	if (typeof value !== "object") return value;
	if ("path" in value) return pathData[value.path];
	if ("literalString" in value) return value.literalString;
	if ("literalNumber" in value) return value.literalNumber;
	if ("literalBool" in value) return value.literalBool;
	if ("literalJson" in value) return JSON.parse(value.literalJson);
	return undefined;
};

mock.module("../DataContext", () => ({
	useData: () => ({ resolve: resolveBoundValue, setByPath }),
}));

afterAll(() => mock.restore());

let browserWindow: Window;
let root: Root | null;

interface HarnessProps {
	bound: BoundValue | undefined;
	revision?: number;
}

const observed = { value: "" as unknown, external: [] as unknown[] };
let commit: (next: unknown) => void = () => {};

const recordExternal = (next: unknown) => {
	observed.external.push(next);
};

function Harness({ bound, revision }: HarnessProps) {
	const { useBoundInputValue } = harnessModule;
	const [value, setValue] = useBoundInputValue<unknown>(bound, "", {
		revision,
		onExternalValue: recordExternal,
	});
	observed.value = value;
	commit = setValue;
	return null;
}

let harnessModule: typeof import("./use-bound-input-value");

async function render(props: HarnessProps) {
	await act(async () => {
		root?.render(<Harness {...props} />);
	});
}

beforeEach(async () => {
	harnessModule = await import("./use-bound-input-value");
	browserWindow = new Window({ url: "https://app.flow-like.test" });
	Object.assign(globalThis, {
		IS_REACT_ACT_ENVIRONMENT: true,
		document: browserWindow.document,
		Element: browserWindow.Element,
		Event: browserWindow.Event,
		HTMLElement: browserWindow.HTMLElement,
		Node: browserWindow.Node,
		navigator: browserWindow.navigator,
		window: browserWindow,
	});
	for (const key of Object.keys(pathData)) delete pathData[key];
	observed.external = [];
	setByPath.mockClear();

	const container = browserWindow.document.createElement("div");
	browserWindow.document.body.append(container);
	root = createRoot(container as unknown as Element);
});

afterEach(async () => {
	const mounted = root;
	root = null;
	await act(async () => mounted?.unmount());
});

describe("useBoundInputValue", () => {
	test("keeps a user edit to a literal binding across re-renders", async () => {
		const bound: BoundValue = { literalString: "" };
		await render({ bound });

		await act(async () => commit("hello"));
		expect(observed.value).toBe("hello");

		// Blur, prop churn, a sibling's data write — none of these are a value write.
		await render({ bound });
		expect(observed.value).toBe("hello");
	});

	test("adopts an external write that moves the value", async () => {
		await render({ bound: { literalString: "" } });
		await act(async () => commit("draft"));

		await render({ bound: { literalString: "from flow" }, revision: 1 });
		expect(observed.value).toBe("from flow");
		expect(observed.external).toEqual(["from flow"]);
	});

	test("adopts an external write back to the declared literal", async () => {
		const bound: BoundValue = { literalString: "" };
		await render({ bound });
		await act(async () => commit("message"));
		expect(observed.value).toBe("message");

		// A composer clearing itself: same string as the declaration, new revision.
		await render({ bound: { literalString: "" }, revision: 1 });
		expect(observed.value).toBe("");
		expect(observed.external).toEqual([""]);
	});

	test("a redundant render at the same revision never clobbers an edit", async () => {
		await render({ bound: { literalString: "seed" }, revision: 3 });
		await act(async () => commit("typed"));

		await render({ bound: { literalString: "seed" }, revision: 3 });
		expect(observed.value).toBe("typed");
		expect(observed.external).toEqual([]);
	});

	test("writes a path binding through to the data context", async () => {
		const bound: BoundValue = { path: "form.name" };
		await render({ bound });

		await act(async () => commit("Ada"));
		expect(setByPath).toHaveBeenCalledWith("form.name", "Ada");

		await render({ bound });
		expect(observed.value).toBe("Ada");
	});

	test("falls back when the binding resolves to nothing", async () => {
		await render({ bound: undefined });
		expect(observed.value).toBe("");
	});

	// `setChecked` and `setProgress` store raw primitives rather than BoundValues, and
	// `resolve` accepts them — so the binding must never be probed with `in`.
	test("renders a raw primitive binding instead of throwing", async () => {
		for (const raw of [true, false, "hello", 0, 42]) {
			await render({ bound: raw as unknown as BoundValue });
			expect(observed.value).toBe(raw);
		}
	});

	test("settles on a literalJson binding rather than re-adopting forever", async () => {
		const bound: BoundValue = { literalJson: '{"a":1}' };
		await render({ bound });
		expect(observed.value).toEqual({ a: 1 });

		// resolve() hands back a fresh object each render; content is what counts.
		await render({ bound: { literalJson: '{"a":1}' } });
		await render({ bound: { literalJson: '{"a":1}' } });
		expect(observed.external).toEqual([]);

		await render({ bound: { literalJson: '{"a":2}' } });
		expect(observed.value).toEqual({ a: 2 });
		expect(observed.external).toEqual([{ a: 2 }]);
	});
});
