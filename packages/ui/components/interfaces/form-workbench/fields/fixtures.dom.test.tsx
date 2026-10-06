import {
	afterAll,
	afterEach,
	describe,
	expect,
	setDefaultTimeout,
	test,
} from "bun:test";
import type { WorkbenchField, WorkbenchLayout } from "../contracts";
import { installWorkbenchDom, mountWorkbench } from "../testing/dom";
import { FIXTURE_NAMES, fixture } from "../testing/fixtures";
import { DESKTOP_LAYOUT, PHONE_LAYOUT } from "../testing/layouts";

const dom = installWorkbenchDom();
const { LiveField, liveStore } = await import("./field-testing");

setDefaultTimeout(30_000);
afterEach(dom.cleanup);
afterAll(dom.restore);

const flat = (fields: readonly WorkbenchField[]): WorkbenchField[] =>
	fields.flatMap((field) => [field, ...flat(field.props)]);

/** A control a person can reach with a key: no focus hook for a blocked file field, a group, or a hidden shell. */
const needsFocusHook = (field: WorkbenchField, blocked: boolean) =>
	field.kind !== "group" && !blocked;

function mountAll(
	name: (typeof FIXTURE_NAMES)[number],
	layout: WorkbenchLayout,
) {
	const store = liveStore(name, { layout });
	const { fields } = store.state.form;
	const view = (
		<div>
			{fields.map((field) => (
				<LiveField key={field.key} store={store} name={field.name} />
			))}
		</div>
	);
	return { store, fields, view };
}

describe("every field of every fixture renders at both sizes", () => {
	const layouts: [string, WorkbenchLayout][] = [
		["desktop", DESKTOP_LAYOUT],
		["phone", PHONE_LAYOUT],
	];
	const cases = FIXTURE_NAMES.flatMap((name) =>
		layouts.map((layout) => [name, ...layout] as const),
	);

	test.each(cases)("%s on %s", async (name, _size, layout) => {
		const { store, fields, view } = mountAll(name, layout);
		const mounted = await mountWorkbench(view, { layout });
		const frames = mounted.container.querySelectorAll("[data-fw-field]");
		expect(frames).toHaveLength(flat(fields).length);
		const host = store.state.form.host;
		for (const field of flat(fields)) {
			const blocked =
				field.fileMode === "flowpath" &&
				(field.kind === "file" || field.kind === "files") &&
				!host.flowPathFiles;
			if (!needsFocusHook(field, blocked)) continue;
			const hook = `[data-fw-focus="field:${field.key}"]`;
			expect(mounted.container.querySelector(hook)).not.toBeNull();
		}
		if (fields.length > 0)
			expect(mounted.container.querySelector("label,span[id]")).not.toBeNull();
		await mounted.unmount();
	});
});

describe("the fixtures are what the controls were written against", () => {
	test("the names and the forms they use", () => {
		expect(FIXTURE_NAMES.length).toBeGreaterThanOrEqual(32);
		expect(fixture("idle").form.fields.length).toBe(9);
		expect(fixture("large").form.fields.length).toBe(29);
	});
});
