import { afterEach, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
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
	MutationObserver: window.MutationObserver,
	Event: window.Event,
	IS_REACT_ACT_ENVIRONMENT: true,
});
let visibility = "Private";
let lookups: string[] = [];
const backend = {
	appState: {
		getApps: async () => [[{ id: "project" }, { name: "My project" }]],
		getAppAuthoritative: async (id: string) => {
			lookups.push(id);
			return { id, visibility, bits: [], packages: {} };
		},
	},
};
mock.module("../../../state/backend-state", () => ({
	useBackendReady: () => true,
	useBackend: () => backend,
}));
const { createRoot } = await import("react-dom/client");
const { DeviceProjectUpload } = await import("./device-project-upload");
let cleanup: (() => Promise<void>) | undefined;
afterEach(async () => {
	await cleanup?.();
	cleanup = undefined;
	lookups = [];
	visibility = "Private";
});

async function fixture() {
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	const commands: unknown[] = [];
	const installed: unknown[] = [];
	const run = async <T,>(
		operation: (call: ManagementCall) => Promise<T>,
	): Promise<T> =>
		operation(async (command) => {
			commands.push(command);
			return {
				operation_id: "operation",
				state: "completed",
				result: { project_path: "/device/projects/project/online-cache" },
			};
		});
	await act(async () => {
		root.render(
			<DeviceProjectUpload
				connected
				projectId="project"
				run={run}
				onInstalled={(value) => installed.push(value)}
			/>,
		);
	});
	cleanup = async () => {
		await act(async () => root.unmount());
		container.remove();
	};
	return { container, commands, installed };
}
test("normal online preparation selects the authoritative project without a folder picker", async () => {
	const f = await fixture();
	const button = [...f.container.querySelectorAll("button")].find(
		(button) => button.textContent === "Prepare deployment from project",
	);
	if (!button) throw new Error("Prepare button missing");
	await act(async () => {
		button.click();
		await new Promise((resolve) => setTimeout(resolve, 0));
	});
	expect(lookups).toEqual(["project"]);
	expect(f.commands).toEqual([
		{
			type: "artifact",
			request: { kind: "prepare_online", project_id: "project" },
		},
	]);
	expect(f.installed).toEqual([
		{
			project_id: "project",
			project_path: "/device/projects/project/online-cache",
			source: "online",
		},
	]);
	expect(
		[...f.container.querySelectorAll("summary")].some(
			(item) =>
				item.textContent === "Advanced: import an existing project folder",
		),
	).toBe(true);
});
test("a browser cannot silently deploy a local-only project as an online source", async () => {
	visibility = "Offline";
	const f = await fixture();
	const button = [...f.container.querySelectorAll("button")].find(
		(button) => button.textContent === "Prepare deployment from project",
	);
	if (!button) throw new Error("Prepare button missing");
	await act(async () => {
		button.click();
		await new Promise((resolve) => setTimeout(resolve, 0));
	});
	expect(f.commands).toHaveLength(0);
	expect(f.installed).toHaveLength(0);
	expect(f.container.textContent).toContain("desktop app");
});
