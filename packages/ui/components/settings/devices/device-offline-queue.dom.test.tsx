import { afterAll, afterEach, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { StrictMode, act } from "react";
import type { ManagementCall } from "../../../lib/device-management/telemetry";

const window = new Window({ url: "https://app.test" });
Object.assign(window, { SyntaxError, TypeError, Error });
Object.assign(globalThis, {
	window,
	document: window.document,
	navigator: window.navigator,
	HTMLElement: window.HTMLElement,
	Element: window.Element,
	Node: window.Node,
	HTMLInputElement: window.HTMLInputElement,
	MutationObserver: window.MutationObserver,
	Event: window.Event,
	IS_REACT_ACT_ENVIRONMENT: true,
});
const { createRoot } = await import("react-dom/client");
const { DeviceOfflineQueue } = await import("./device-offline-queue");
const container = document.createElement("div");
document.body.append(container);
const root = createRoot(container);
let commands: Record<string, unknown>[] = [];
let quarantined = false;
const operation = "00000000-0000-4000-8000-000000000001";
const call: ManagementCall = async (command) => {
	commands.push(command);
	return {
		operation_id: "management",
		state: "completed",
		result:
			command.type === "offline_queue"
				? {
						placement_id: "placement",
						next: null,
						queues: [
							{
								scope: "a".repeat(64),
								quarantined,
								pending_count: 2,
								pending_bytes: 1048576,
								oldest_at: 100,
								mirror_error: "Local mirror budget reached",
								head: {
									sequence: 1,
									operation_id: operation,
									resource: "file",
									payload: null,
									state: "conflict",
									attempts: 1,
									created_at: 100,
									error: "Cloud file changed",
									local_version: 1,
								},
							},
						],
					}
				: { queued_operation_id: operation },
	};
};
async function render() {
	await act(async () =>
		root.render(
			<StrictMode>
				<DeviceOfflineQueue
					placement="placement"
					busy={false}
					run={(run) => run(call)}
				/>
			</StrictMode>,
		),
	);
}
function button(label: string) {
	const button = [...container.querySelectorAll("button")].find(
		(button) => button.textContent === label,
	);
	expect(button).toBeDefined();
	if (!button) throw new Error(`Missing button: ${label}`);
	return button;
}
async function click(label: string) {
	await act(async () => button(label).click());
}
afterEach(async () => {
	await act(async () => root.render(null));
	commands = [];
	quarantined = false;
});
afterAll(async () => {
	await act(async () => root.unmount());
	window.happyDOM.abort();
});

test("queue view reports retained counts and warnings and retries the exact head", async () => {
	await render();
	expect(commands).toHaveLength(0);
	await click("Refresh offline queues");
	expect(container.textContent).toContain("2 pending · 1.00 MiB");
	expect(container.textContent).toContain("Cloud file changed");
	expect(container.textContent).toContain(
		"Local table refresh: Local mirror budget reached",
	);
	await click("Retry queued write");
	expect(commands[1]).toEqual({
		type: "offline_queue_retry",
		placement_id: "placement",
		scope: "a".repeat(64),
		queued_operation_id: operation,
	});
	expect(commands[2].type).toBe("offline_queue");
});

test("skipping an attempted write requires a reason and explicit uncertainty acknowledgement", async () => {
	await render();
	await click("Refresh offline queues");
	expect(button("Skip queued write").disabled).toBe(true);
	const input = container.querySelector<HTMLInputElement>(
		'input:not([type="checkbox"])',
	);
	expect(input).not.toBeNull();
	await act(async () => {
		Object.getOwnPropertyDescriptor(
			window.HTMLInputElement.prototype,
			"value",
		)?.set?.call(input, "Reviewed conflicting file");
		input?.dispatchEvent(new Event("input", { bubbles: true }));
	});
	expect(button("Skip queued write").disabled).toBe(true);
	await act(async () =>
		container
			.querySelector<HTMLInputElement>('input[type="checkbox"]')
			?.click(),
	);
	expect(button("Skip queued write").disabled).toBe(false);
	await click("Skip queued write");
	expect(commands[1]).toEqual({
		type: "offline_queue_skip",
		placement_id: "placement",
		scope: "a".repeat(64),
		queued_operation_id: operation,
		reason: "Reviewed conflicting file",
		acknowledge_uncertain: true,
	});
	expect(container.textContent).toContain("Skip requested");
});

test("quarantined scopes remain visible without replay controls", async () => {
	quarantined = true;
	await render();
	await click("Refresh offline queues");
	expect(container.textContent).toContain("Authorization blocked");
	expect(container.textContent).toContain("Queued data remains on the device");
	expect(container.textContent).not.toContain("Retry queued write");
	expect(container.textContent).not.toContain("Skip queued write");
});
