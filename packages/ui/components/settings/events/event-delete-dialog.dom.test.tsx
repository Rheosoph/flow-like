import { afterAll, afterEach, describe, expect, test } from "bun:test";
import type { IEvent } from "../../../lib/schema/flow/event";
import {
	byRole,
	click,
	inPortal,
	installDom,
	queryByRole,
} from "../devices/testing/dom-harness";

const dom = installDom();
const { EventDeleteDialog } = await import("./event-delete-dialog");
const { SampleEventsDevices, sampleValue } = await import(
	"../devices/events/events-test-kit"
);

afterEach(dom.cleanup);
afterAll(dom.restore);

const APP = "app_invoice_ai";
const event = { id: "evt_nightly", name: "Nightly digest" } as IEvent;

function servedBy(...devices: string[]) {
	return {
		served: devices.map((device) => ({ device, deviceId: device })),
	} as never;
}

function mount(options: {
	devices?: string[];
	opened?: string[];
	onConfirm?: (eventId: string) => void;
	onCancel?: () => void;
}) {
	const opened = options.opened ?? [];
	const base = sampleValue(APP, { opened });
	const value = options.devices
		? {
				...base,
				live: {
					...base.live,
					rows: new Map([[event.id, servedBy(...options.devices)]]),
				} as never,
			}
		: { ...base, live: null };
	return dom.render(
		<SampleEventsDevices value={value}>
			<EventDeleteDialog
				event={event}
				onConfirm={options.onConfirm ?? (() => undefined)}
				onCancel={options.onCancel ?? (() => undefined)}
			/>
		</SampleEventsDevices>,
	);
}

describe("deleting an event", () => {
	test("asks for confirmation and names the event before anything is deleted", async () => {
		const confirmed: string[] = [];
		await mount({ onConfirm: (id) => confirmed.push(id) });
		const dialog = inPortal("alertdialog");
		expect(dialog.textContent).toContain("Delete Nightly digest?");
		expect(dialog.textContent).not.toContain("Still runs on");
		expect(queryByRole("button", "Open Devices")).toBeNull();
		expect(confirmed).toEqual([]);
		await click(byRole("button", "Delete event"));
		expect(confirmed).toEqual([event.id]);
	});

	test("names the devices that still serve it and says they are not stopped", async () => {
		await mount({ devices: ["Alice's laptop", "edge-01", "edge-02"] });
		expect(inPortal("alertdialog").textContent).toContain(
			"Still runs on Alice's laptop and 2 others. Deleting the event does not stop those services.",
		);
	});

	test("a single device is named alone and Open Devices leads to the event's devices", async () => {
		const opened: string[] = [];
		const cancelled: string[] = [];
		await mount({
			devices: ["Alice's laptop"],
			opened,
			onCancel: () => cancelled.push("cancel"),
		});
		expect(inPortal("alertdialog").textContent).toContain(
			"Still runs on Alice's laptop. Deleting the event does not stop those services.",
		);
		await click(byRole("button", "Open Devices"));
		expect(opened).toHaveLength(1);
		expect(opened[0]).toContain("evt_nightly");
		expect(cancelled).toEqual(["cancel"]);
	});

	test("deletion is never blocked: the action stays available next to the warning", async () => {
		const confirmed: string[] = [];
		await mount({
			devices: ["edge-01"],
			onConfirm: (id) => confirmed.push(id),
		});
		await click(byRole("button", "Delete event"));
		expect(confirmed).toEqual([event.id]);
	});
});
