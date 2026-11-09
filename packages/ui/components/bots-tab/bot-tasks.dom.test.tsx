import {
	afterAll,
	beforeAll,
	beforeEach,
	describe,
	expect,
	mock,
	test,
} from "bun:test";
import { act, createElement } from "react";
import type {
	IBotTask,
	ICreateBotTaskRequest,
} from "../../state/backend-state/bot-state";
import {
	fakeI18n,
	installDomGlobals,
	restoreDomGlobals,
	translate,
	domWindow as window,
} from "./testing/dom-harness";

const actualLocales = { ...(await import("@flow-like/locales")) };
mock.module("@flow-like/locales", () => ({
	...actualLocales,
	useTranslation: () => ({ t: translate, i18n: fakeI18n }),
}));

const { createRoot } = await import("react-dom/client");
const { BotTasksView } = await import("./bot-tasks");
const { TriggerPill } = await import("./conversation-note");

function task(patch: Partial<IBotTask>): IBotTask {
	return {
		id: "t1",
		bot_id: "b1",
		kind: "cron",
		cron: "0 9 * * 1-5",
		tz: "Europe/Berlin",
		instruction: "Morning brief",
		title: "Morning brief",
		next_run_at: new Date(Date.now() + 3_600_000).toISOString(),
		enabled: true,
		created_by: "owner",
		created_at: new Date().toISOString(),
		...patch,
	};
}

const created: ICreateBotTaskRequest[] = [];
const toggled: IBotTask[] = [];
const deleted: IBotTask[] = [];

let container: HTMLElement;
let root: ReturnType<typeof createRoot>;

async function show(tasks: IBotTask[]) {
	await act(async () =>
		root.render(
			createElement(BotTasksView, {
				tasks,
				busy: false,
				onCreate: (request: ICreateBotTaskRequest) => created.push(request),
				onToggle: (item: IBotTask) => toggled.push(item),
				onDelete: (item: IBotTask) => deleted.push(item),
			}),
		),
	);
}

function button(label: string): HTMLButtonElement {
	const found = [...container.querySelectorAll("button")].find(
		(candidate) =>
			candidate.getAttribute("aria-label") === label ||
			candidate.textContent?.trim() === label,
	);
	if (!found) throw new Error(`no button "${label}"`);
	return found as unknown as HTMLButtonElement;
}

async function click(element: Element) {
	await act(async () => {
		element.dispatchEvent(
			new window.MouseEvent("click", {
				bubbles: true,
				cancelable: true,
			}) as never,
		);
	});
}

async function type(element: Element, value: string) {
	const field = element as unknown as HTMLInputElement | HTMLTextAreaElement;
	const prototype =
		field.tagName === "TEXTAREA"
			? window.HTMLTextAreaElement.prototype
			: window.HTMLInputElement.prototype;
	await act(async () => {
		Object.getOwnPropertyDescriptor(prototype, "value")?.set?.call(
			field,
			value,
		);
		field.dispatchEvent(new window.Event("input", { bubbles: true }) as never);
	});
}

beforeAll(() => {
	installDomGlobals();
	container = window.document.createElement("div") as unknown as HTMLElement;
	window.document.body.appendChild(container as never);
	root = createRoot(container);
});

afterAll(async () => {
	await act(async () => root.unmount());
	mock.module("@flow-like/locales", () => actualLocales);
	restoreDomGlobals();
});

beforeEach(() => {
	created.length = 0;
	toggled.length = 0;
	deleted.length = 0;
});

describe("BotTasksView", () => {
	test("lists each task with its schedule and next run, and tells paused from done", async () => {
		await show([
			task({}),
			task({
				id: "t2",
				title: "Paused one",
				enabled: false,
				next_run_at: null,
			}),
			task({
				id: "t3",
				kind: "once",
				cron: null,
				tz: null,
				at: "2026-11-04T09:00:00Z",
				title: "Finished",
				enabled: false,
				next_run_at: null,
				last_run_at: "2026-11-04T09:00:01Z",
			}),
		]);
		const rows = [...container.querySelectorAll('[data-slot="bot-task"]')];
		expect(rows.map((row) => row.getAttribute("data-state"))).toEqual([
			"active",
			"paused",
			"done",
		]);
		expect(rows[0]?.textContent).toContain("0 9 * * 1-5 (Europe/Berlin)");
		expect(rows[0]?.textContent).toContain("Next");
		expect(rows[1]?.textContent).toContain("Paused");
		expect(rows[2]?.textContent).toContain("Done");
		expect(() => button("Resume Finished")).toThrow();
		expect(() => button("Pause Morning brief")).not.toThrow();
	});

	test("pause and delete reach the callbacks with the task", async () => {
		await show([task({})]);
		await click(button("Pause Morning brief"));
		await click(button("Delete Morning brief"));
		expect(toggled.map((item) => item.id)).toEqual(["t1"]);
		expect(deleted.map((item) => item.id)).toEqual(["t1"]);
	});

	test("the form stays disabled until a once task has an instruction and a time", async () => {
		await show([]);
		expect(container.textContent).toContain("No tasks yet");
		const add = button("Add task");
		expect(add.disabled).toBe(true);
		await type(
			container.querySelector('textarea[aria-label="Instruction"]') as Element,
			"Check the report",
		);
		expect(button("Add task").disabled).toBe(true);
		await type(
			container.querySelector('input[type="datetime-local"]') as Element,
			"2026-11-04T09:30",
		);
		expect(button("Add task").disabled).toBe(false);
		const monitoring = container.querySelector(
			'[role="switch"]',
		) as HTMLButtonElement;
		expect(monitoring.getAttribute("aria-checked")).toBe("false");
		await click(monitoring);
		await act(async () => {
			(
				container.querySelector("form") as unknown as HTMLFormElement
			).dispatchEvent(
				new window.Event("submit", {
					bubbles: true,
					cancelable: true,
				}) as never,
			);
		});
		expect(created).toHaveLength(1);
		expect(created[0]?.kind).toBe("once");
		expect(created[0]?.at).toBe(new Date("2026-11-04T09:30").toISOString());
		expect(created[0]?.instruction).toBe("Check the report");
		expect(created[0]?.monitoring).toBe(true);
		expect(
			container.querySelector('[role="switch"]')?.getAttribute("aria-checked"),
		).toBe("false");
	});

	test("a repeating task needs five cron fields", async () => {
		await show([]);
		await click(button("Repeat"));
		await type(
			container.querySelector('textarea[aria-label="Instruction"]') as Element,
			"Weekly digest",
		);
		await type(
			container.querySelector('input[aria-label="Cron schedule"]') as Element,
			"0 9 * *",
		);
		expect(button("Add task").disabled).toBe(true);
		await type(
			container.querySelector('input[aria-label="Cron schedule"]') as Element,
			"0 9 * * 1",
		);
		expect(button("Add task").disabled).toBe(false);
	});

	test("a bot at its limit offers no new task", async () => {
		await show(
			Array.from({ length: 50 }, (_, index) => task({ id: `t${index}` })),
		);
		expect(container.querySelector('[data-slot="bot-task-form"]')).toBeNull();
		expect(container.textContent).toContain("the most it can keep");
	});
});

describe("TriggerPill", () => {
	test("a scheduled trigger reads as a pill with its title", async () => {
		await act(async () =>
			root.render(
				createElement(TriggerPill, {
					trigger: {
						type: "trigger",
						id: "tk-1",
						refId: "tk-1-ref",
						text: "Run it",
						createdAt: new Date().toISOString(),
						time: Date.now(),
						source: "schedule",
						title: "Morning brief",
					},
				}),
			),
		);
		const card = container.querySelector('[data-slot="trigger-pill"]');
		expect(card?.textContent).toBe("Scheduled: Morning brief");
	});

	test("other sources are named", async () => {
		await act(async () =>
			root.render(
				createElement(TriggerPill, {
					trigger: {
						type: "trigger",
						id: "tk-2",
						refId: "tk-2-ref",
						text: "Run it",
						createdAt: new Date().toISOString(),
						time: Date.now(),
						source: "webhook",
						title: "New order",
					},
				}),
			),
		);
		expect(
			container.querySelector('[data-slot="trigger-pill"]')?.textContent,
		).toBe("Webhook: New order");
	});

	test("an email names its sender, and a click shows the mail", async () => {
		await act(async () =>
			root.render(
				createElement(TriggerPill, {
					trigger: {
						type: "trigger",
						id: "tk-3",
						refId: "mail-1",
						text: "Answer this mail",
						createdAt: new Date().toISOString(),
						time: Date.now(),
						source: "email",
						title: "Invoice 42",
						payload: {
							from: [{ name: "Anna", email: "anna@example.com" }],
							to: [{ name: "", email: "bot@example.com" }],
							subject: "Invoice 42",
							text: "Please pay by Friday.",
							authenticated: false,
							attachments: [{ name: "invoice.pdf", size: 2048 }],
						},
					},
				}),
			),
		);
		const pill = container.querySelector('[data-slot="trigger-pill"] button');
		expect(pill?.textContent).toBe("Email from Anna: Invoice 42");
		await act(async () => (pill as HTMLButtonElement).click());
		const details = window.document.querySelector(
			'[data-slot="trigger-details"]',
		);
		expect(details?.textContent).toContain("anna@example.com");
		expect(details?.textContent).toContain("Please pay by Friday.");
		expect(details?.textContent).toContain("invoice.pdf");
		expect(details?.textContent).toContain("not verified");
		await act(async () => root.render(createElement("div")));
		await new Promise((resolve) => setTimeout(resolve, 10));
	});
});
