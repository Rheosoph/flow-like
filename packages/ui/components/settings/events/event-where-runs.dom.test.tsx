import { afterAll, afterEach, describe, expect, test } from "bun:test";
import { withDeviceEventSource } from "../../../lib/event-source";
import type { IEvent } from "../../../lib/schema/flow/event";
import type { IEventUpsertOptions } from "../../../state/backend-state/event-state";
import {
	byRole,
	click,
	inPortal,
	installDom,
} from "../devices/testing/dom-harness";

const dom = installDom();
const { toast } = await import("sonner");
const { cleanupDevices, mountDevices, preloadDevices } = await import(
	"../devices/testing/mount-devices"
);
await preloadDevices();
const { EventWhereRuns } = await import("./event-where-runs");

afterEach(async () => {
	await cleanupDevices();
	await dom.cleanup();
});
afterAll(dom.restore);

const deviceEvent = withDeviceEventSource({
	id: "evt_nightly",
	name: "Nightly digest",
	active: true,
	event_type: "quick_action",
	execution_mode: "Local",
	config: [],
} as unknown as IEvent);

interface Upsert {
	event: IEvent;
	pat?: string;
	options?: IEventUpsertOptions;
}

interface Options {
	event?: IEvent;
	offline?: boolean | null;
	canExecuteLocally?: boolean;
	dirty?: boolean;
	canWrite?: boolean;
	boardExecutionMode?: string;
	mapping?: { withSink: string[]; sinkAvailability?: never };
	hub?: never;
}

async function mount(options: Options = {}) {
	const upserts: Upsert[] = [];
	const taken: IEvent[] = [];
	const opened: string[] = [];
	const event = options.event ?? deviceEvent;
	const view = await mountDevices(
		() => (
			<EventWhereRuns
				appId="app_invoice_ai"
				event={event}
				mapping={options.mapping}
				boardExecutionMode={options.boardExecutionMode}
				isOffline={options.offline === undefined ? false : options.offline}
				hub={options.hub}
				canWrite={options.canWrite ?? true}
				writeDeniedMessage="Your role cannot change events."
				dirty={options.dirty}
				onOpenRunsOn={() => opened.push(event.id)}
				onTakenBack={(saved) => {
					taken.push(saved);
				}}
			/>
		),
		{
			providers: false,
			backend: {
				capabilities: () => ({
					needsSignIn: false,
					canHostLlamaCPP: false,
					canHostMLX: false,
					canHostEmbeddings: false,
					canExecuteLocally: options.canExecuteLocally ?? true,
				}),
				isOffline: async () => options.offline ?? false,
				userState: { getPATs: async () => [] },
				eventState: {
					getEvents: async () => [],
					upsertEvent: async (
						_appId: string,
						saved: IEvent,
						_version: unknown,
						pat?: string,
						_tokens?: unknown,
						upsertOptions?: IEventUpsertOptions,
					) => {
						upserts.push({ event: saved, pat, options: upsertOptions });
						return saved;
					},
				},
			} as never,
		},
	);
	return { ...view, upserts, taken, opened };
}

const button = (name: string) => byRole("button", name) as HTMLButtonElement;
const toasts = () => toast.getHistory().map((entry) => entry.title);

describe("a device-only event's way back", () => {
	test("says where it runs and opens Runs on", async () => {
		const view = await mount();
		expect(view.container.textContent).toContain(
			"Runs only on the devices you deploy it to. The hub and this computer don't start it.",
		);
		await click(button("Open Runs on"));
		expect(view.opened).toEqual(["evt_nightly"]);
	});

	test("Run on the hub saves it as a Remote event and clears the device-only marker", async () => {
		const before = toasts().length;
		const view = await mount();
		await click(button("Run on the hub"));
		await view.settle();
		expect(view.upserts).toHaveLength(1);
		expect(view.upserts[0].event.execution_mode).toBe("Remote");
		expect(view.upserts[0].options).toEqual({ source: "default" });
		expect(view.taken.map((saved) => saved.id)).toEqual(["evt_nightly"]);
		expect(toasts().slice(before)).toEqual([
			"Nightly digest now runs on the hub.",
		]);
	});

	test("Run on this computer saves it as a Local event", async () => {
		const view = await mount({
			event: { ...deviceEvent, execution_mode: "Remote" } as IEvent,
		});
		await click(button("Run on this computer"));
		await view.settle();
		expect(view.upserts[0].event.execution_mode).toBe("Local");
		expect(view.upserts[0].options).toEqual({ source: "default" });
	});

	test("a trigger that registers a sink asks for the same Personal Access Token as Activate", async () => {
		const view = await mount({
			mapping: { withSink: ["quick_action"] },
		});
		await click(button("Run on the hub"));
		await view.settle();
		expect(inPortal("dialog").textContent).toContain("Authorize this change");
		expect(view.upserts).toEqual([]);
	});

	test("an option that is not available is disabled and says why", async () => {
		const offline = await mount({ offline: true });
		expect(button("Run on the hub").disabled).toBe(true);
		expect(button("Run on this computer").disabled).toBe(false);
		expect(offline.container.textContent).toContain(
			"Run on the hub: This app is not synced to a hub, so the hub cannot run it.",
		);
		await click(button("Run on the hub"));
		expect(offline.upserts).toEqual([]);
		await cleanupDevices();
		await dom.cleanup();

		const web = await mount({ canExecuteLocally: false });
		expect(button("Run on this computer").disabled).toBe(true);
		expect(web.container.textContent).toContain(
			"Run on this computer: This app cannot run flows on this device.",
		);
		await cleanupDevices();
		await dom.cleanup();

		const pinned = await mount({ boardExecutionMode: "Local" });
		expect(button("Run on the hub").disabled).toBe(true);
		expect(pinned.container.textContent).toContain(
			"This flow is set to run locally only.",
		);
	});

	test("unsaved edits and a read-only role block both options", async () => {
		const dirty = await mount({ dirty: true });
		expect(button("Run on the hub").disabled).toBe(true);
		expect(button("Run on this computer").disabled).toBe(true);
		expect(dirty.container.textContent).toContain(
			"Save or discard your changes before changing where it runs.",
		);
		await cleanupDevices();
		await dom.cleanup();

		const readOnly = await mount({ canWrite: false });
		expect(button("Run on the hub").disabled).toBe(true);
		expect(readOnly.container.textContent).toContain(
			"Your role cannot change events.",
		);
	});
});
