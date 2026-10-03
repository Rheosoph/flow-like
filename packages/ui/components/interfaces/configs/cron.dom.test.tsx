import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { IEventExecutionMode } from "../../../lib/schema/flow/event";
import type { IHub } from "../../../lib/schema/hub/hub";
import {
	byRole,
	installDom,
	queryByRole,
	settle,
} from "../../settings/devices/testing/dom-harness";
import type { IConfigInterfaceProps } from "../interfaces";

const dom = installDom();
const { CronJobConfig } = await import("./cron");

afterEach(async () => {
	await dom.cleanup();
});
afterAll(dom.restore);

const hub = { domain: "hub.example.test" } as unknown as IHub;

async function render(props: Partial<IConfigInterfaceProps>) {
	const updates: unknown[] = [];
	const view = await dom.render(
		<CronJobConfig
			isEditing
			appId="app"
			boardId="board"
			nodeId="node"
			node={{ id: "node" } as IConfigInterfaceProps["node"]}
			config={{ sink_type: "cron", expression: "0 9 * * *" }}
			onConfigUpdate={(payload) => updates.push(payload)}
			section="runtime"
			{...props}
		/>,
	);
	await settle();
	return { ...view, updates };
}

describe("Cron runtime", () => {
	test("a standalone editor with a hub and a desktop offers the execution target", async () => {
		await render({ hub, canExecuteLocally: true });
		expect(queryByRole("combobox", /execution target/i)).not.toBeNull();
	});

	test.each([
		[IEventExecutionMode.Local, "runs in the desktop app"],
		[IEventExecutionMode.Remote, "runs on the hub"],
	])(
		"an event running %s fixes where the schedule runs",
		async (eventExecutionMode, sentence) => {
			const view = await render({
				hub,
				canExecuteLocally: true,
				eventExecutionMode,
			});
			expect(queryByRole("combobox", /execution target/i)).toBeNull();
			expect(view.container.textContent).toContain(sentence);
		},
	);

	test.each([
		[IEventExecutionMode.Local, false],
		[IEventExecutionMode.Remote, true],
	])(
		"an event running %s warns about hub-only cron seconds: %s",
		async (eventExecutionMode, warns) => {
			const view = await render({
				hub,
				canExecuteLocally: true,
				eventExecutionMode,
				section: "schedule",
				config: { sink_type: "cron", expression: "30 0 9 * * *" },
			});
			expect(
				view.container.textContent?.includes("minute-precision only"),
			).toBe(warns);
		},
	);
});

describe("Cron time zone", () => {
	const original = Intl.DateTimeFormat.prototype.resolvedOptions;

	afterEach(() => {
		Intl.DateTimeFormat.prototype.resolvedOptions = original;
	});

	test("a schedule without a zone shows UTC, the zone it runs in, not the browser's", async () => {
		Intl.DateTimeFormat.prototype.resolvedOptions = function resolvedOptions() {
			return { ...original.call(this), timeZone: "Asia/Tokyo" };
		};
		await render({ section: "schedule" });
		expect(byRole("combobox", /UTC/).textContent).toContain("UTC");
		expect(queryByRole("combobox", /Tokyo/)).toBeNull();
	});

	test("a written zone is the one shown", async () => {
		await render({
			section: "schedule",
			config: {
				sink_type: "cron",
				expression: "0 9 * * *",
				timezone: "Europe/Berlin",
			},
		});
		expect(byRole("combobox", /Europe\/Berlin/)).toBeDefined();
	});
});
