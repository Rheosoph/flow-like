import { afterEach, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import { act } from "react";
import type { InstalledProject } from "../../../lib/device-management/deployment";
import type { ManagementCall } from "../../../lib/device-management/telemetry";

const window = new Window({ url: "https://app.test" });
Object.assign(window, { SyntaxError, TypeError, Error });
Object.assign(globalThis, {
	window,
	document: window.document,
	navigator: window.navigator,
	localStorage: window.localStorage,
	HTMLElement: window.HTMLElement,
	Element: window.Element,
	Node: window.Node,
	MutationObserver: window.MutationObserver,
	Event: window.Event,
	IS_REACT_ACT_ENVIRONMENT: true,
});
let visibility = "Private";
let lookups: string[] = [];
const bundle = {
	version: 1,
	project_id: "project",
	documents: {
		app: { id: "project", visibility: "Private", bits: [], packages: {} },
		"events/api/versions/1/0/0": {
			id: "api",
			name: "API",
			event_type: "http",
			event_version: [1, 0, 0],
			board_id: "board",
			board_version: [2, 0, 0],
			active: true,
		},
		"boards/board/versions/2/0/0": {
			id: "board",
			version: [2, 0, 0],
			layers: {},
			variables: {},
		},
	},
};
const backend = {
	appState: {
		getApps: async () => [[{ id: "project" }, { name: "My project" }]],
		getAppAuthoritative: async (id: string) => {
			lookups.push(id);
			return { id, visibility, bits: [], packages: {} };
		},
	},
	userState: { getProfile: async () => ({ id: "profile" }) },
	apiState: {
		get: async (_profile: unknown, route: string) => {
			expect(route).toBe("apps/project/device-metadata");
			return bundle;
		},
	},
};
mock.module("../../../state/backend-state", () => ({
	useBackendReady: () => true,
	useBackend: () => backend,
}));
const { createRoot } = await import("react-dom/client");
const { DeviceProjectUpload } = await import("./device-project-upload");
const { pendingArtifactTransfers, rememberArtifactTransfer } = await import(
	"../../../lib/device-management/artifacts"
);
let cleanup: (() => Promise<void>) | undefined;
afterEach(async () => {
	await cleanup?.();
	cleanup = undefined;
	lookups = [];
	visibility = "Private";
	window.localStorage.clear();
});

/**
 * Accepts every chunk in order and derives file sizes from the uploaded manifest.
 * Like the agent, it refuses a transfer it never began with a retryable `failed`.
 */
function device(known: string[] = []) {
	const requests: Record<string, unknown>[] = [];
	const transfers = new Set(known);
	const begins: string[] = [];
	let transfer = "";
	let descriptor: Record<string, unknown> = {};
	let manifest = new Uint8Array();
	let sizes: number[] = [];
	const offsets = new Map<number, number>();
	let committed = false;
	const status = (index: number | null) => {
		const size =
			index === null ? Number(descriptor.manifest_size) : (sizes[index] ?? 0);
		const offset = index === null ? manifest.length : (offsets.get(index) ?? 0);
		return {
			transfer_id: transfer,
			descriptor,
			state: committed ? "committed" : "receiving",
			expires_at: Math.floor(Date.now() / 1000) + 3600,
			manifest_ready: manifest.length === Number(descriptor.manifest_size),
			file_index: index,
			offset,
			complete: offset === size && (index === null || offsets.has(index)),
			project_path: committed
				? `/device/projects/project/revisions/${descriptor.manifest_sha256}`
				: null,
		};
	};
	const call: ManagementCall = async (command, id = crypto.randomUUID()) => {
		const request = command.request as Record<string, unknown>;
		requests.push(request);
		if (request.kind === "begin") {
			begins.push(id);
			transfers.add(id);
			transfer = id;
			descriptor = request.descriptor as Record<string, unknown>;
		} else if (!transfers.has(String(request.transfer_id)))
			return {
				operation_id: id,
				state: "rejected",
				result: {
					error: "Unknown artifact transfer",
					code: "failed",
					retryable: true,
				},
			};
		if (request.kind === "abort") {
			transfers.delete(String(request.transfer_id));
			return {
				operation_id: id,
				state: "completed",
				result: { state: "aborted" },
			};
		}
		const index = (request.file_index ?? null) as number | null;
		if (request.kind === "chunk") {
			const bytes = new Uint8Array(
				Buffer.from(String(request.data), "base64url"),
			);
			if (index === null) {
				const next = new Uint8Array(manifest.length + bytes.length);
				next.set(manifest);
				next.set(bytes, manifest.length);
				manifest = next;
				if (manifest.length === Number(descriptor.manifest_size))
					sizes = JSON.parse(new TextDecoder().decode(manifest)).files.map(
						(file: { size: number }) => file.size,
					);
			} else offsets.set(index, (offsets.get(index) ?? 0) + bytes.length);
		}
		if (request.kind === "commit") committed = true;
		return { operation_id: id, state: "completed", result: status(index) };
	};
	return { call, requests, begins };
}
/** Loses the first begin before it reaches the device, as a session closing mid-send would. */
function droppingFirstBegin(call: ManagementCall) {
	let dropped: string | undefined;
	const wrapped: ManagementCall = async (command, id) => {
		if (
			!dropped &&
			(command.request as Record<string, unknown>).kind === "begin"
		) {
			dropped = id;
			throw new Error("Management session closed before sending");
		}
		return call(command, id);
	};
	return { call: wrapped, dropped: () => dropped };
}

async function fixture(call: ManagementCall = device().call) {
	const container = document.createElement("div");
	document.body.append(container);
	const root = createRoot(container);
	const installed: InstalledProject[] = [];
	await act(async () => {
		root.render(
			<DeviceProjectUpload
				connected
				projectId="project"
				deviceId="device"
				run={(operation) => operation(call)}
				onInstalled={(value) => installed.push(value)}
			/>,
		);
	});
	cleanup = async () => {
		await act(async () => root.unmount());
		container.remove();
	};
	return { container, installed };
}
async function click(container: HTMLElement, text: string) {
	const button = [...container.querySelectorAll("button")].find(
		(value) => value.textContent === text,
	);
	if (!button) throw new Error(`Button ${text} missing`);
	await act(async () => {
		button.click();
		await new Promise((resolve) => setTimeout(resolve, 0));
	});
}
async function until(predicate: () => boolean) {
	for (let attempt = 0; attempt < 200; attempt++) {
		if (predicate()) return;
		await act(async () => new Promise((resolve) => setTimeout(resolve, 10)));
	}
	throw new Error("Timed out waiting for the upload UI");
}

test("online preparation installs the approved metadata catalog with its revision", async () => {
	const fake = device();
	const f = await fixture(fake.call);
	await click(f.container, "Prepare deployment from project");
	await until(() => f.container.textContent?.includes("revision ") ?? false);
	expect(lookups).toEqual(["project"]);
	expect(fake.requests).toHaveLength(0);
	await click(f.container, "Upload project revision");
	await until(() => f.installed.length === 1);
	const [installed] = f.installed;
	expect(installed).toMatchObject({
		project_id: "project",
		source: "online",
		project_path: `/device/projects/project/revisions/${installed.revision}`,
	});
	expect(installed.online_metadata_sha256).toMatch(/^[a-f0-9]{64}$/);
	expect(
		installed.online_catalog?.events.map((event) => [
			event.id,
			event.event_version,
			event.board_version,
		]),
	).toEqual([["api", [1, 0, 0], [2, 0, 0]]]);
	expect(fake.requests.at(-1)?.kind).toBe("commit");
	expect(pendingArtifactTransfers("device")).toEqual([]);
});

test("unfinished uploads from an earlier session can be aborted after reopening", async () => {
	const transferId = crypto.randomUUID();
	rememberArtifactTransfer("device", {
		transfer_id: transferId,
		project_id: "project",
		manifest_sha256: "a".repeat(64),
		confirmed: true,
	});
	const fake = device([transferId]);
	const f = await fixture(fake.call);
	expect(f.container.textContent).toContain(
		"1 unfinished upload of this project",
	);
	await click(f.container, "Abort unfinished uploads");
	await until(() => !f.container.textContent?.includes("unfinished upload"));
	expect(fake.requests).toEqual([
		{ kind: "abort", project_id: "project", transfer_id: transferId },
	]);
	expect(pendingArtifactTransfers("device")).toEqual([]);
});

test("a begin whose reply is lost stays recorded so a later session can abort it", async () => {
	const fake = device();
	let begun: string | undefined;
	const f = await fixture(async (command, id) => {
		if ((command.request as Record<string, unknown>).kind === "begin") {
			begun = id;
			await fake.call(command, id);
			throw new Error("Management session closed before the reply arrived");
		}
		return fake.call(command, id);
	});
	await click(f.container, "Prepare deployment from project");
	await until(() => f.container.textContent?.includes("revision ") ?? false);
	await click(f.container, "Upload project revision");
	await until(
		() => f.container.textContent?.includes("no confirmed completion") ?? false,
	);
	expect(begun).toBeString();
	expect(
		pendingArtifactTransfers("device", "project").map(
			(transfer) => transfer.transfer_id,
		),
	).toEqual([begun ?? ""]);
	expect(f.container.textContent).toContain("Resume encrypted upload");
	await click(f.container, "Resume encrypted upload");
	await until(() => f.installed.length === 1);
	expect(fake.begins).toEqual([begun ?? ""]);
	expect(pendingArtifactTransfers("device")).toEqual([]);
});

test("a begin that never reached the device is begun under the same id when resumed after reopening", async () => {
	const fake = device();
	const lossy = droppingFirstBegin(fake.call);
	const first = await fixture(lossy.call);
	await click(first.container, "Prepare deployment from project");
	await until(
		() => first.container.textContent?.includes("revision ") ?? false,
	);
	await click(first.container, "Upload project revision");
	await until(
		() =>
			first.container.textContent?.includes("no confirmed completion") ?? false,
	);
	const dropped = lossy.dropped() ?? "";
	expect(fake.requests).toEqual([]);
	expect(pendingArtifactTransfers("device", "project")).toEqual([
		expect.objectContaining({ transfer_id: dropped, confirmed: false }),
	]);
	await cleanup?.();
	cleanup = undefined;
	const reopened = await fixture(fake.call);
	await click(reopened.container, "Prepare deployment from project");
	await until(
		() => reopened.container.textContent?.includes("revision ") ?? false,
	);
	await click(reopened.container, "Resume encrypted upload");
	await until(() => reopened.installed.length === 1);
	expect(fake.requests.slice(0, 2).map((request) => request.kind)).toEqual([
		"status",
		"begin",
	]);
	expect(fake.begins).toEqual([dropped]);
	expect(pendingArtifactTransfers("device")).toEqual([]);
});

test("transfers the device does not know are forgotten when aborted instead of blocking the dialog", async () => {
	for (const confirmed of [false, true])
		rememberArtifactTransfer("device", {
			transfer_id: crypto.randomUUID(),
			project_id: "project",
			manifest_sha256: "a".repeat(64),
			confirmed,
		});
	const fake = device();
	const lossy = droppingFirstBegin(fake.call);
	const f = await fixture(lossy.call);
	expect(f.container.textContent).toContain(
		"2 unfinished uploads of this project",
	);
	await click(f.container, "Abort unfinished uploads");
	await until(() => !f.container.textContent?.includes("unfinished upload"));
	expect(f.container.querySelector('[role="alert"]')).toBeNull();
	await click(f.container, "Prepare deployment from project");
	await until(() => f.container.textContent?.includes("revision ") ?? false);
	await click(f.container, "Upload project revision");
	await until(
		() => f.container.textContent?.includes("Abort device transfer") ?? false,
	);
	await click(f.container, "Abort device transfer");
	await until(() => !f.container.textContent?.includes("Transfer: "));
	expect(f.container.querySelector('[role="alert"]')).toBeNull();
	expect(f.container.textContent).toContain("Upload project revision");
	expect(pendingArtifactTransfers("device")).toEqual([]);
	expect(fake.requests.map((request) => request.kind)).toEqual([
		"abort",
		"abort",
		"abort",
	]);
});

test("a browser cannot silently deploy a local-only project as an online source", async () => {
	visibility = "Offline";
	const fake = device();
	const f = await fixture(fake.call);
	await click(f.container, "Prepare deployment from project");
	expect(fake.requests).toHaveLength(0);
	expect(f.installed).toHaveLength(0);
	expect(f.container.textContent).toContain("desktop app");
});
