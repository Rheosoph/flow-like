import { expect, test } from "bun:test";
import type { IFrontendHosting } from "../../../lib/schema/flow/event-payload";

// Bun module mocks survive mock.restore(). Keep DOM globals and backend mocks
// inside a child process so this fixture cannot change other runtime tests.
if (process.env.FLOW_EVENT_HOSTING_TEST_CHILD !== "1") {
	test("hosted frontend settings require a separate confirmed anonymous opt-in", () => {
		const result = Bun.spawnSync([process.execPath, "test", import.meta.path], {
			env: { ...process.env, FLOW_EVENT_HOSTING_TEST_CHILD: "1" },
			stdout: "pipe",
			stderr: "pipe",
		});
		if (result.exitCode !== 0) {
			throw new Error(result.stderr.toString("utf8"));
		}
		expect(result.exitCode).toBe(0);
	}, 15_000);
} else {
	const { afterAll, afterEach, mock } = await import("bun:test");
	const { Window } = await import("happy-dom");
	const window = new Window({ url: "https://editor.example.test" });
	Object.assign(window, { SyntaxError, TypeError, Error });
	window.document.write("<!doctype html><html><body></body></html>");
	Object.assign(globalThis, {
		window,
		document: window.document,
		navigator: window.navigator,
		Element: window.Element,
		HTMLElement: window.HTMLElement,
		HTMLInputElement: window.HTMLInputElement,
		HTMLButtonElement: window.HTMLButtonElement,
		Node: window.Node,
		NodeFilter: window.NodeFilter,
		Text: window.Text,
		Event: window.Event,
		CustomEvent: window.CustomEvent,
		MouseEvent: window.MouseEvent,
		MutationObserver: window.MutationObserver,
		Document: window.Document,
		DocumentFragment: window.DocumentFragment,
		getComputedStyle: window.getComputedStyle.bind(window),
		requestAnimationFrame: window.requestAnimationFrame.bind(window),
		cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
		IS_REACT_ACT_ENVIRONMENT: true,
	});
	const locales = await import("@flow-like/locales");
	mock.module("@flow-like/locales", () => ({
		...locales,
		useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
	}));
	const backendState = await import("../../../state/backend-state");
	mock.module("../../../state/backend-state", () => ({
		...backendState,
		useBackend: () => ({
			userState: { getProfile: async () => ({}) },
			eventState: {},
		}),
	}));
	const invokeHooks = await import("../../../hooks/use-invoke");
	mock.module("../../../hooks/use-invoke", () => ({
		...invokeHooks,
		useInvoke: () => ({
			data: undefined,
			isLoading: false,
			error: null,
			refetch: async () => ({ data: [] }),
		}),
	}));
	const { act, useState } = await import("react");
	const { createRoot } = await import("react-dom/client");
	const { EventHosting } = await import("./event-hosting");
	const { IEventExecutionMode, IEventExposure } = await import(
		"../../../lib/schema/flow/event"
	);
	const event = {
		id: "published-event",
		name: "Hosted form",
		description: "",
		active: true,
		board_id: "board",
		node_id: "entry",
		event_type: "generic_form",
		event_version: [1, 0, 0],
		execution_mode: IEventExecutionMode.Remote,
		exposure: IEventExposure.Public,
		config: [],
		variables: {},
		priority: 0,
		created_at: { secs_since_epoch: 1, nanos_since_epoch: 0 },
		updated_at: { secs_since_epoch: 1, nanos_since_epoch: 0 },
	};
	const roots: ReturnType<typeof createRoot>[] = [];
	afterEach(async () => {
		await act(async () => {
			for (const root of roots.splice(0)) root.unmount();
		});
		window.document.body.innerHTML = "";
	});
	afterAll(async () => window.happyDOM.abort());

	async function mount(
		initialConfig: Record<string, unknown> = {},
		canWrite = true,
	) {
		const updates: IFrontendHosting[] = [];
		function Harness() {
			const [config, setConfig] = useState(initialConfig);
			return (
				<EventHosting
					appId="app"
					event={event}
					config={config}
					canWrite={canWrite}
					hasUnsavedChanges={false}
					onUpdate={(hosting) => {
						updates.push(hosting);
						setConfig({ ...config, frontend_hosting: hosting });
					}}
				/>
			);
		}
		const container = document.createElement("div");
		document.body.append(container);
		const root = createRoot(container);
		roots.push(root);
		await act(async () => root.render(<Harness />));
		return updates;
	}
	function toggle(name: "enabled" | "anonymous") {
		const element = document.querySelector<HTMLButtonElement>(
			`#frontend-hosting-${name}`,
		);
		if (!element) throw new Error(`Missing ${name} switch`);
		return element;
	}
	async function click(element: HTMLElement) {
		await act(async () => element.click());
	}
	function button(text: string) {
		const element = Array.from(document.querySelectorAll("button")).find(
			(candidate) => candidate.textContent?.trim() === text,
		);
		if (!element) throw new Error(`Missing ${text} button`);
		return element;
	}
	const authenticated = {
		enabled: true,
		allow_anonymous: false,
		auth_proxy: true,
	};
	const anonymous = { enabled: true, allow_anonymous: true, auth_proxy: false };

	test("hosting defaults off and anonymous access requires warning confirmation", async () => {
		const updates = await mount();
		expect(toggle("enabled").getAttribute("aria-checked")).toBe("false");
		expect(toggle("anonymous").disabled).toBe(true);
		await click(toggle("anonymous"));
		expect(updates).toEqual([]);
		await click(toggle("enabled"));
		expect(updates).toEqual([authenticated]);
		expect(toggle("anonymous").getAttribute("aria-checked")).toBe("false");
		await click(toggle("anonymous"));
		expect(updates).toEqual([authenticated]);
		const dialog = document.querySelector('[role="alertdialog"]');
		expect(dialog?.textContent).toContain("Allow anonymous access?");
		expect(dialog?.textContent).toMatch(/anyone with (?:the|this) link/i);
		expect(dialog?.textContent).toMatch(/owner/i);
		expect(dialog?.textContent).toMatch(/all anonymous usage/i);
		expect(dialog?.textContent).toMatch(/compute/i);
		expect(dialog?.textContent).toMatch(/model/i);
		await click(button("Cancel"));
		expect(document.querySelector('[role="alertdialog"]')).toBeNull();
		expect(updates).toEqual([authenticated]);
		expect(toggle("anonymous").getAttribute("aria-checked")).toBe("false");
		await click(toggle("anonymous"));
		await click(button("Enable anonymous access"));
		expect(document.querySelector('[role="alertdialog"]')).toBeNull();
		expect(updates).toEqual([authenticated, anonymous]);
		expect(toggle("anonymous").getAttribute("aria-checked")).toBe("true");
	});

	test("revoking anonymous access is immediate and disabling hosting clears its opt-in", async () => {
		const updates = await mount({ frontend_hosting: anonymous });
		await click(toggle("anonymous"));
		expect(updates).toEqual([authenticated]);
		expect(document.querySelector('[role="alertdialog"]')).toBeNull();
		await click(toggle("anonymous"));
		await click(button("Enable anonymous access"));
		await click(toggle("enabled"));
		expect(updates.at(-1)).toEqual({ ...authenticated, enabled: false });
		expect(toggle("anonymous").disabled).toBe(true);
		expect(toggle("anonymous").getAttribute("aria-checked")).toBe("false");
		await click(toggle("enabled"));
		expect(updates.at(-1)).toEqual(authenticated);
		expect(toggle("anonymous").getAttribute("aria-checked")).toBe("false");
	});

	test("read-only settings cannot change hosting or anonymous access", async () => {
		const updates = await mount({ frontend_hosting: authenticated }, false);
		expect(toggle("enabled").disabled).toBe(true);
		expect(toggle("anonymous").disabled).toBe(true);
		await click(toggle("enabled"));
		await click(toggle("anonymous"));
		expect(updates).toEqual([]);
		expect(document.querySelector('[role="alertdialog"]')).toBeNull();
	});
}
