import { afterEach, expect, test } from "bun:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Window } from "happy-dom";
import { act } from "react";
import {
	IValueType,
	type IVariable,
	IVariableType,
} from "../../../lib/schema/flow/variable";
import { convertJsonToUint8Array } from "../../../lib/uint8";
import {
	type IBackendState,
	useBackendStore,
} from "../../../state/backend-state";
import {
	type RuntimeVariablesContextValue,
	RuntimeVariablesProvider,
} from "../../../state/runtime-variables-context";
import type { DeviceRequirement } from "./use-app-setup";

let cleanup: (() => Promise<void>) | undefined;
afterEach(async () => {
	await cleanup?.();
	cleanup = undefined;
});

async function setup() {
	const window = new Window();
	Object.assign(window, { SyntaxError, TypeError });
	const globals = {
		document: window.document,
		Element: window.Element,
		Event: window.Event,
		CustomEvent: window.CustomEvent,
		InputEvent: window.InputEvent,
		KeyboardEvent: window.KeyboardEvent,
		FocusEvent: window.FocusEvent,
		PointerEvent: window.PointerEvent,
		HTMLElement: window.HTMLElement,
		HTMLButtonElement: window.HTMLButtonElement,
		HTMLInputElement: window.HTMLInputElement,
		HTMLTextAreaElement: window.HTMLTextAreaElement,
		MouseEvent: window.MouseEvent,
		Node: window.Node,
		NodeFilter: window.NodeFilter,
		navigator: window.navigator,
		window,
		Document: window.Document,
		DocumentFragment: window.DocumentFragment,
		Text: window.Text,
		MutationObserver: window.MutationObserver,
		ResizeObserver: window.ResizeObserver,
		getComputedStyle: window.getComputedStyle.bind(window),
		requestAnimationFrame: window.requestAnimationFrame.bind(window),
		cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
		IS_REACT_ACT_ENVIRONMENT: true,
	};
	const descriptors = Object.keys(globals).map(
		(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
	);
	Object.assign(globalThis, globals);
	const previousBackend = useBackendStore.getState().backend;
	const { createRoot } = await import("react-dom/client");
	const container = window.document.createElement("div");
	window.document.body.append(container);
	const root = createRoot(container as unknown as HTMLElement);
	const client = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	cleanup = async () => {
		await act(async () => root.unmount());
		client.clear();
		useBackendStore.setState({ backend: previousBackend });
		await window.happyDOM.close();
		for (const [key, descriptor] of descriptors) {
			if (descriptor) Object.defineProperty(globalThis, key, descriptor);
			else Reflect.deleteProperty(globalThis, key);
		}
	};
	return {
		window,
		root,
		client,
		container: container as unknown as HTMLElement,
		body: window.document.body as unknown as HTMLElement,
	};
}

function secretVariable(): IVariable {
	return {
		id: "certificate",
		name: "Certificate",
		data_type: IVariableType.String,
		value_type: IValueType.Normal,
		default_value: null,
		exposed: true,
		editable: true,
		secret: true,
		runtime_configured: true,
	};
}

function requirement(variable: IVariable): DeviceRequirement {
	return {
		variable,
		seeded: variable,
		boardId: "board-1",
		boardName: "Certificate flow",
		satisfied: false,
		waived: false,
		alsoShared: false,
	};
}

function deviceStore(
	saveValues: RuntimeVariablesContextValue["saveValues"],
): RuntimeVariablesContextValue {
	return {
		getValues: async () => new Map(),
		saveValues,
		hasAllValues: async () => false,
		listValues: async () => [],
		deleteValue: async () => {},
		deleteValues: async () => {},
		subscribe: () => () => {},
	};
}

function input(container: HTMLElement) {
	const field = container.querySelector<HTMLInputElement>(
		'input[type="password"]',
	);
	if (!field) throw new Error("Secret input is missing");
	return field;
}

async function enterSecret(
	container: HTMLElement,
	window: Window,
	value: string,
) {
	const field = input(container);
	await act(async () => {
		Object.getOwnPropertyDescriptor(
			window.HTMLInputElement.prototype,
			"value",
		)?.set?.call(field, value);
		field.dispatchEvent(new Event("input", { bubbles: true }));
	});
}

function button(container: HTMLElement, label: string): HTMLButtonElement {
	const found = Array.from(container.querySelectorAll("button")).find(
		(candidate) =>
			candidate.getAttribute("aria-label") === label ||
			candidate.textContent?.trim() === label,
	);
	if (!found) throw new Error(`Button is missing: ${label}`);
	return found;
}

const draft = `certificate-draft-${"A".repeat(12_000)}`;
const bytes = convertJsonToUint8Array(draft) ?? [];

test("device save failures preserve a long secret and support retry without exposing the error payload", async () => {
	const { root, container, window } = await setup();
	const { RequirementRow } = await import("./setup-rows");
	const saved: number[][] = [];
	const store = deviceStore(async (_appId, _boardId, values) => {
		saved.push(values[0].value);
		if (saved.length === 1) throw `Storage rejected ${draft}`;
	});
	await act(async () =>
		root.render(
			<RuntimeVariablesProvider value={store}>
				<RequirementRow
					appId="app-1"
					requirement={requirement(secretVariable())}
					onWaive={() => {}}
					onJumpToShared={() => {}}
				/>
			</RuntimeVariablesProvider>,
		),
	);
	await enterSecret(container, window, draft);
	await act(async () => button(container, "Save on this device").click());
	expect(container.querySelector('[role="alert"]')?.textContent).toContain(
		"Could not save on this device",
	);
	expect(container.textContent).not.toContain(draft);
	expect(input(container).value).toBe(draft);
	expect(button(container, "Save on this device").disabled).toBe(false);
	await act(async () => button(container, "Save on this device").click());
	expect(saved).toEqual([bytes, bytes]);
	expect(container.querySelector('[role="alert"]')).toBeNull();
});

test("a missing device store gives a visible save failure", async () => {
	const { root, container } = await setup();
	const { RequirementRow } = await import("./setup-rows");
	await act(async () =>
		root.render(
			<RequirementRow
				appId="app-1"
				requirement={requirement({ ...secretVariable(), default_value: bytes })}
				onWaive={() => {}}
				onJumpToShared={() => {}}
			/>,
		),
	);
	await act(async () => button(container, "Save on this device").click());
	expect(container.querySelector('[role="alert"]')?.textContent).toContain(
		"Device storage is unavailable",
	);
	expect(input(container).value).toBe(draft);
});

test("shared save failures retain the draft for a successful retry", async () => {
	const { root, container, client, window } = await setup();
	const { SharedParameterRow } = await import("./setup-rows");
	const backend = useBackendStore.getState().backend;
	const commands: unknown[] = [];
	useBackendStore.setState({
		backend: {
			...backend,
			boardState: {
				...backend?.boardState,
				getBoardVariables: async () => [],
				getBoard: async () => {
					throw new Error("Unused board read");
				},
				executeCommand: async (_appId, _boardId, command) => {
					commands.push(command);
					if (commands.length === 1) throw new Error(`Rejected ${draft}`);
					return command;
				},
			},
		} as IBackendState,
	});
	await act(async () =>
		root.render(
			<QueryClientProvider client={client}>
				<SharedParameterRow
					appId="app-1"
					boardId="board-1"
					variable={secretVariable()}
					onJumpToSetup={() => {}}
				/>
			</QueryClientProvider>,
		),
	);
	await enterSecret(container, window, draft);
	await act(async () => button(container, "Save").click());
	expect(container.querySelector('[role="alert"]')?.textContent).toContain(
		"Could not save to the app",
	);
	expect(container.textContent).not.toContain(draft);
	expect(input(container).value).toBe(draft);
	expect(button(container, "Save").disabled).toBe(false);
	await act(async () => button(container, "Save").click());
	expect(commands).toHaveLength(2);
	expect(commands[0]).toEqual(commands[1]);
	expect(container.querySelector('[role="alert"]')).toBeNull();
});

test("the runtime prompt catches a rejected save and retries the retained secret", async () => {
	const { root, body, window } = await setup();
	const { RuntimeVariablesPrompt } = await import(
		"../../flow/runtime-variables-prompt"
	);
	const saved: number[][] = [];
	await act(async () =>
		root.render(
			<RuntimeVariablesPrompt
				open
				onOpenChange={() => {}}
				variables={[secretVariable()]}
				existingValues={new Map()}
				onSave={async (values) => {
					saved.push(values[0].value);
					if (saved.length === 1) throw new Error(`Rejected ${draft}`);
				}}
				onCancel={() => {}}
			/>,
		),
	);
	await enterSecret(body, window, draft);
	await act(async () => button(body, "Save & Continue").click());
	expect(body.querySelector('[role="alert"]')?.textContent).toContain(
		"Could not save runtime variables",
	);
	expect(body.textContent).not.toContain(draft);
	expect(input(body).value).toBe(draft);
	expect(button(body, "Save & Continue").disabled).toBe(false);
	await act(async () => button(body, "Save & Continue").click());
	expect(saved).toEqual([bytes, bytes]);
	expect(body.querySelector('[role="alert"]')).toBeNull();
});
