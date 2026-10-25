// @vitest-environment happy-dom
import { createHash, webcrypto } from "node:crypto";
import type { PackageInspection } from "@flow-like/flow-like-ui/lib/schema/developer";
import {
	MemoryTier,
	type PackageManifest,
	TimeoutTier,
} from "@flow-like/flow-like-ui/lib/schema/wasm";
import { type ReactNode, act, createElement } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({
	invoke: vi.fn(),
	post: vi.fn(),
	readFile: vi.fn(),
	fetch: vi.fn(),
	toastError: vi.fn(),
	toastSuccess: vi.fn(),
	onPublished: vi.fn(),
	profile: { hub_profile: { hub: "registry.test", secure: true } },
	auth: {
		isAuthenticated: true,
		user: { profile: { name: "Widget Author" } },
	},
}));

vi.mock("@flow-like/flow-like-ui", async () => {
	const { createElement } = await import("react");
	const container = ({ children }: { children?: ReactNode }) =>
		createElement("div", null, children);
	return {
		...Object.fromEntries(
			[
				"Badge",
				"Card",
				"CardContent",
				"CardDescription",
				"CardHeader",
				"CardTitle",
				"Select",
				"SelectContent",
				"SelectItem",
				"SelectTrigger",
				"SelectValue",
				"Separator",
			].map((name) => [name, container]),
		),
		Button: ({
			variant: _variant,
			size: _size,
			...props
		}: Record<string, unknown>) => createElement("button", props),
		Input: "input",
		Label: "label",
		Textarea: "textarea",
		MemoryTier: { Standard: "standard" },
		TimeoutTier: { Standard: "standard" },
		useBackend: () => ({ userState: { getSettingsProfile: vi.fn() } }),
		useInvoke: () => ({ data: mocks.profile }),
	};
});
vi.mock("@flow-like/locales", () => {
	const t = (
		key: string,
		fallback: string | Record<string, unknown>,
		options?: Record<string, unknown>,
	) => {
		const values = typeof fallback === "object" ? fallback : options;
		const template =
			typeof fallback === "string"
				? fallback
				: String(fallback.defaultValue_other ?? key);
		return template.replace(/\{\{(\w+)\}\}/g, (_, name) =>
			String(values?.[name] ?? ""),
		);
	};
	return {
		i18n: { t },
		useTranslation: () => ({ t }),
		Trans: ({ children }: { children?: ReactNode }) => children,
	};
});
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/plugin-fs", () => ({ readFile: mocks.readFile }));
vi.mock("@tauri-apps/plugin-http", () => ({ fetch: mocks.fetch }));
vi.mock("react-oidc-context", () => ({ useAuth: () => mocks.auth }));
vi.mock("@tanstack/react-query", () => ({
	useQueryClient: () => ({ invalidateQueries: vi.fn() }),
}));
vi.mock("next/navigation", () => ({ useRouter: () => ({ push: vi.fn() }) }));
vi.mock("next/link", () => ({ default: "a" }));
vi.mock("sonner", () => ({
	toast: {
		error: mocks.toastError,
		success: mocks.toastSuccess,
		info: vi.fn(),
	},
}));
vi.mock("../../../../lib/api", () => ({ post: mocks.post }));
vi.mock(
	"@flow-like/flow-like-ui/components/store/package-workspace/use-workspace-data",
	() => ({ invalidatePackageLists: vi.fn() }),
);
vi.mock("../../local-projects", () => ({
	projectRoutes: { workspace: vi.fn() },
}));
vi.mock("../../use-mine-packages", () => ({
	mineQueryKeys: { manifest: vi.fn() },
}));
vi.mock("../node-debugger", () => ({ nodeInspectionKey: vi.fn() }));

import { PublishWizard } from "../publish-wizard";

const projectPath = "/projects/mailbox-elements";
const bundlePath = `${projectPath}/dist/widgets.flwb`;
const wasmPath = `${projectPath}/node.wasm`;
const bundleBytes = new TextEncoder().encode("built widget bundle");
const wasmBytes = new Uint8Array([0, 97, 115, 109]);
const manifest: PackageManifest = {
	manifestVersion: 2,
	id: "com.acme.mailbox-elements",
	name: "Mailbox Elements",
	version: "0.1.0",
	description: "Mailbox widgets",
	authors: [],
	keywords: [],
	wasmPath: "node.wasm",
	widgetBundlePath: "dist/widgets.flwb",
	metadata: {},
	permissions: {
		memory: MemoryTier.Standard,
		timeout: TimeoutTier.Standard,
		network: {
			httpEnabled: false,
			allowedHosts: [],
			websocketEnabled: false,
			tcpEnabled: false,
			udpEnabled: false,
			dnsEnabled: false,
		},
		filesystem: {
			nodeStorage: false,
			userStorage: false,
			uploadDir: false,
			cacheDir: false,
		},
		oauthScopes: [],
		variables: false,
		cache: false,
		streaming: false,
		a2ui: false,
		models: false,
	},
};
const widget: PackageInspection["widgets"][number] = {
	id: "mailbox",
	name: "Mailbox",
	description: "Read mail",
	inputCount: 1,
	eventCount: 1,
	queryCount: 0,
	contract: {
		contractVersion: 1,
		id: "mailbox",
		inputs: { selectedMessage: { type: "string" } },
		events: { messageSelected: { payloadSchema: { type: "string" } } },
	},
};

let root: Root;
let container: HTMLDivElement;
let inspection: PackageInspection;
let artifactError: unknown;

beforeEach(() => {
	vi.clearAllMocks();
	vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
	vi.stubGlobal("crypto", webcrypto);
	inspection = {
		manifest,
		nodes: [],
		widgets: [widget],
		isPackage: true,
		wasmPath: "",
		widgetBundlePath: bundlePath,
	};
	artifactError = undefined;
	mocks.invoke.mockImplementation(async (command, args) => {
		switch (command) {
			case "developer_read_manifest":
				return manifest;
			case "developer_inspect_package":
				return inspection;
			case "developer_get_manifest":
				return { ...manifest, wasm_path: "node.wasm" };
			case "developer_find_publish_artifacts":
				if (artifactError) throw artifactError;
				if (!inspection.wasmPath && args.includeWasm !== false) {
					throw { error: "Declared WASM path does not exist: node.wasm" };
				}
				return {
					wasm: args.includeWasm ? wasmPath : null,
					widgetBundle: bundlePath,
				};
			default:
				throw new Error(`Unexpected command: ${command}`);
		}
	});
	mocks.post.mockImplementation(async (_profile, path) => {
		switch (path) {
			case "registry/check-id":
				return { available: true, owned_by_caller: false };
			case "registry/check-version":
				return { available: true };
			case "registry/upload-url":
				return {
					upload_url: "https://uploads.test/node.wasm",
					widget_bundle_upload_url: "https://uploads.test/widgets.flwb",
				};
			case "registry/publish":
				return {
					success: true,
					package_id: manifest.id,
					version: manifest.version,
				};
			default:
				throw new Error(`Unexpected endpoint: ${path}`);
		}
	});
	mocks.readFile.mockImplementation(async (path) =>
		path === bundlePath ? bundleBytes : wasmBytes,
	);
	mocks.fetch.mockResolvedValue({ ok: true });
	container = document.createElement("div");
	document.body.appendChild(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
	vi.unstubAllGlobals();
});

function button(label: string): HTMLButtonElement {
	const result = Array.from(container.querySelectorAll("button")).find(
		(element) => element.textContent?.trim() === label,
	);
	if (!result) throw new Error(`Missing button: ${label}`);
	return result;
}

async function click(label: string) {
	const target = button(label);
	expect(target.disabled).toBe(false);
	await act(async () => target.click());
}

async function reviewPackage() {
	await act(async () =>
		root.render(
			createElement(PublishWizard, {
				projectPath,
				onPublished: mocks.onPublished,
			}),
		),
	);
	await click("Check");
	await click("Check");
	await click("Continue");
	await click("Continue");
}

describe("PublishWizard artifacts", () => {
	test.each([
		{ name: "widgets only", includeWasm: false },
		{ name: "widgets and nodes", includeWasm: true },
	])(
		"publishes $name from the reviewed inspection",
		async ({ includeWasm }) => {
			if (includeWasm) {
				inspection.wasmPath = wasmPath;
				inspection.nodes = [
					{
						name: "read_mail",
						friendly_name: "Read Mail",
						description: "Read a message",
						category: "Mail",
						pins: [],
						permissions: [],
					},
				];
			}
			await reviewPackage();
			if (!includeWasm) {
				expect(container.textContent).toContain(
					"Only widgets will be published.",
				);
			}
			await click("Publish as Private");
			await act(async () => {
				await vi.waitFor(() =>
					expect(mocks.onPublished).toHaveBeenCalledOnce(),
				);
			});

			expect(mocks.invoke).toHaveBeenCalledWith(
				"developer_find_publish_artifacts",
				{ projectPath, includeWasm },
			);
			expect(mocks.readFile.mock.calls.map(([path]) => path)).toEqual(
				includeWasm ? [wasmPath, bundlePath] : [bundlePath],
			);
			expect(mocks.fetch).toHaveBeenCalledTimes(includeWasm ? 2 : 1);
			expect(mocks.fetch).toHaveBeenCalledWith(
				"https://uploads.test/widgets.flwb",
				{
					method: "PUT",
					headers: { "Content-Type": "application/octet-stream" },
					body: bundleBytes,
				},
			);
			if (includeWasm) {
				expect(mocks.fetch).toHaveBeenCalledWith(
					"https://uploads.test/node.wasm",
					expect.objectContaining({ method: "PUT", body: wasmBytes }),
				);
			}
			expect(mocks.post).toHaveBeenCalledWith(
				mocks.profile.hub_profile,
				"registry/publish",
				{
					manifest: expect.objectContaining({
						id: manifest.id,
						version: manifest.version,
						widgets: [
							{
								id: widget.id,
								name: widget.name,
								description: widget.description,
								contract: widget.contract,
								keywords: [],
							},
						],
						widget_bundle_hash: createHash("sha256")
							.update(bundleBytes)
							.digest("hex"),
					}),
				},
				mocks.auth,
			);
			expect(mocks.onPublished).toHaveBeenCalledWith(
				manifest.id,
				manifest.version,
			);
			expect(mocks.toastError).not.toHaveBeenCalled();
		},
	);

	test("keeps WASM discovery failures readable after reviewing a node artifact", async () => {
		inspection.wasmPath = wasmPath;
		artifactError = { error: "Declared WASM path does not exist: node.wasm" };
		await reviewPackage();
		await click("Publish as Private");

		expect(mocks.invoke).toHaveBeenCalledWith(
			"developer_find_publish_artifacts",
			{ projectPath, includeWasm: true },
		);
		expect(mocks.toastError).toHaveBeenCalledWith(
			"Publish failed: Declared WASM path does not exist: node.wasm",
		);
		expect(mocks.readFile).not.toHaveBeenCalled();
		expect(mocks.fetch).not.toHaveBeenCalled();
		expect(mocks.post.mock.calls.map(([, path]) => path)).toEqual([
			"registry/check-id",
			"registry/check-version",
		]);
		expect(mocks.onPublished).not.toHaveBeenCalled();
		expect(button("Publish as Private").disabled).toBe(false);
	});
});
