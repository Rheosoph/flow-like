import { describe, expect, test } from "bun:test";
import type { FileSlot, RunEntry, RunStatus } from "../contracts";
import { fixture } from "../testing/fixtures";
import {
	type BarActionInput,
	type MenuItemId,
	barActionsOf,
} from "./bar-actions";

const done = fixture("done").runs[0] as RunEntry;
const NOW = done.endedAt ?? 0;

function actionsFor(
	patch: Omit<Partial<BarActionInput>, "run"> & {
		run?: Partial<RunEntry>;
	} = {},
) {
	const { run, ...rest } = patch;
	return barActionsOf({
		run: { ...done, ...run },
		now: NOW,
		hasFields: true,
		compact: false,
		comparing: false,
		canPin: true,
		pinned: false,
		railDiffers: false,
		copy: "answer",
		...rest,
	});
}

const ids = (list: ReturnType<typeof barActionsOf>) =>
	list.menu.map((item) => item.id);

describe("barActionsOf: working and waiting runs", () => {
	for (const status of [
		"starting",
		"running",
		"streaming",
		"asking",
		"sending",
	] as const) {
		test(`${status}: Stop only`, () => {
			const actions = actionsFor({ run: { status } });
			expect(actions).toMatchObject({
				stop: true,
				removeFromQueue: false,
				repeat: null,
				use: false,
				copy: null,
			});
			expect(actions.menu).toEqual([]);
		});
	}

	test("queued: Remove from queue only", () => {
		const actions = actionsFor({ run: { status: "queued" } });
		expect(actions.removeFromQueue).toBe(true);
		expect(actions.stop).toBe(false);
		expect(actions.menu).toEqual([]);
	});
});

describe("barActionsOf: finished runs", () => {
	test("done with the rail unchanged: Copy and the menu, no Run again (the dock's Run does it)", () => {
		const actions = actionsFor();
		expect(actions.repeat).toBeNull();
		expect(actions.use).toBe(false);
		expect(actions.copy).toBe("answer");
		expect(ids(actions)).toEqual(["pin", "use", "savePreset", "remove"]);
	});

	test("done with the rail edited: Run again and Use these inputs", () => {
		const actions = actionsFor({ railDiffers: true });
		expect(actions.repeat).toBe("runAgain");
		expect(actions.use).toBe(true);
	});

	test("stopped: Run again even when the rail matches", () => {
		expect(actionsFor({ run: { status: "stopped" } }).repeat).toBe("runAgain");
	});

	test("failed: Try again", () => {
		const actions = actionsFor({ run: { status: "failed" } });
		expect(actions.repeat).toBe("tryAgain");
	});

	test("not started because its file was not sent: Try again", () => {
		const actions = actionsFor({
			run: {
				status: "notStarted",
				outcome: { kind: "notStarted", reason: "fileNotSent" },
			},
		});
		expect(actions.repeat).toBe("tryAgain");
	});

	test("not started and taken out of the queue: Run again", () => {
		const actions = actionsFor({
			run: {
				status: "notStarted",
				outcome: { kind: "notStarted", reason: "removedFromQueue" },
			},
		});
		expect(actions.repeat).toBe("runAgain");
	});

	test("a form without inputs has no repeat button and no input items", () => {
		const actions = actionsFor({ hasFields: false, run: { status: "failed" } });
		expect(actions.repeat).toBeNull();
		expect(actions.use).toBe(false);
		expect(ids(actions)).toEqual(["pin", "remove"]);
	});

	test("without room to compare there is no pin item", () => {
		expect(ids(actionsFor({ canPin: false }))).not.toContain("pin");
		expect(ids(actionsFor({ pinned: true }))).toContain("unpin");
	});

	test("Remove sits behind a separator", () => {
		const menu = actionsFor().menu;
		expect(menu.at(-1)).toMatchObject({ id: "remove", separatorBefore: true });
		expect(menu[0]?.separatorBefore).toBe(false);
	});
});

describe("barActionsOf: Pick again (S4)", () => {
	const reminder: FileSlot = {
		id: "reminder:invoice_file:0:old.pdf",
		name: "old.pdf",
		size: 1,
		type: null,
		state: "reminder",
		progress: null,
		ref: null,
		error: null,
		sentAt: null,
		expiresAt: null,
	};
	const expired = (run: RunEntry): Partial<RunEntry> => ({
		status: "stopped" as RunStatus,
		copy: {
			...run.copy,
			values: { ...run.copy.values, invoice_file: reminder },
		},
	});

	test("files that can no longer be sent: Use these inputs takes the repeat button's place", () => {
		const actions = actionsFor({ run: expired(done) });
		expect(actions.repeat).toBeNull();
		expect(actions.use).toBe(true);
	});

	test("the menu keeps Run again, disabled, with its reason", () => {
		const actions = actionsFor({ run: expired(done) });
		const repeat = actions.menu.find((item) => item.id === "repeat");
		expect(repeat).toMatchObject({
			disabled: true,
			blocker: "files",
			repeat: "runAgain",
		});
	});

	test("a done run in the same case shows Use these inputs when the rail differs", () => {
		const actions = actionsFor({
			run: { ...expired(done), status: "done" },
			railDiffers: true,
		});
		expect(actions.repeat).toBeNull();
		expect(actions.use).toBe(true);
	});
});

describe("barActionsOf: inputs a saved run cannot send again (S4)", () => {
	const fields = fixture("done").form.fields;
	const withValues = (
		values: RunEntry["copy"]["values"],
	): Partial<RunEntry> => ({
		status: "failed",
		origin: "history",
		copy: { ...done.copy, values },
	});

	test("the form still takes the run's own copy: Try again", () => {
		expect(actionsFor({ run: { status: "failed" }, fields }).repeat).toBe(
			"tryAgain",
		);
	});

	test("an object property kept out of storage: Use these inputs, the menu names why", () => {
		const terms = fields.find((field) => field.kind === "group");
		if (!terms) throw new Error("the done form has no object field");
		const [first, second] = terms.props;
		const actions = actionsFor({
			fields,
			run: withValues({
				...done.copy.values,
				[terms.name]: { [first.name]: "EUR", [second.name]: { $hidden: true } },
			}),
		});
		expect(actions.repeat).toBeNull();
		expect(actions.use).toBe(true);
		expect(actions.menu.find((item) => item.id === "repeat")).toMatchObject({
			disabled: true,
			blocker: "hidden",
			repeat: "tryAgain",
		});
	});

	test("a field today's form requires that the saved run lacks: Use these inputs", () => {
		const vendor = fields.find((field) => field.name === "vendor_name");
		if (!vendor) throw new Error("the done form has no vendor field");
		const { [vendor.name]: _gone, ...rest } = done.copy.values;
		const required = fields.map((field) =>
			field.name === vendor.name
				? { ...field, required: true, defaultValue: "" }
				: field,
		);
		const actions = actionsFor({ fields: required, run: withValues(rest) });
		expect(actions.repeat).toBeNull();
		expect(actions.use).toBe(true);
		expect(actions.menu.find((item) => item.id === "repeat")?.blocker).toBe(
			"form",
		);
	});
});

describe("barActionsOf: compact bars and two panes", () => {
	test("on a phone Copy moves into the menu", () => {
		const actions = actionsFor({ compact: true });
		expect(actions.copy).toBeNull();
		const copy = actions.menu.find((item) => item.id === "copy");
		expect(copy?.copy).toBe("answer");
	});

	test("with two panes the repeat goes to the menu, enabled", () => {
		const actions = actionsFor({
			compact: true,
			comparing: true,
			railDiffers: true,
			run: { status: "stopped" },
		});
		expect(actions.repeat).toBeNull();
		const repeat = actions.menu.find((item) => item.id === "repeat");
		expect(repeat).toMatchObject({ disabled: false, repeat: "runAgain" });
		expect(actions.use).toBe(true);
	});

	test("a failed run in two panes lists Try again in the menu", () => {
		const actions = actionsFor({
			comparing: true,
			compact: true,
			run: { status: "failed" },
		});
		expect(actions.menu.find((item) => item.id === "repeat")?.repeat).toBe(
			"tryAgain",
		);
	});

	test("nothing to copy: no Copy item", () => {
		const actions = actionsFor({ compact: true, copy: null });
		expect((ids(actions) as MenuItemId[]).includes("copy")).toBe(false);
	});
});
