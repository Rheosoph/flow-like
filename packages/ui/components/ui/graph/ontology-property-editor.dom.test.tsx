import { afterEach, expect, test } from "bun:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Window } from "happy-dom";
import { type ReactNode, act } from "react";
import { useBackendStore } from "../../../state/backend-state";

let cleanup: (() => Promise<void>) | undefined;
afterEach(async () => {
	await cleanup?.();
	cleanup = undefined;
});

async function setup() {
	const window = new Window();
	Object.assign(window, { SyntaxError, TypeError });
	Object.assign(globalThis, {
		document: window.document,
		Element: window.Element,
		Event: window.Event,
		CustomEvent: window.CustomEvent,
		InputEvent: window.InputEvent,
		KeyboardEvent: window.KeyboardEvent,
		FocusEvent: window.FocusEvent,
		PointerEvent: window.PointerEvent,
		NodeFilter: window.NodeFilter,
		HTMLElement: window.HTMLElement,
		HTMLButtonElement: window.HTMLButtonElement,
		HTMLInputElement: window.HTMLInputElement,
		HTMLTextAreaElement: window.HTMLTextAreaElement,
		MouseEvent: window.MouseEvent,
		Node: window.Node,
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
	});
	const { createRoot } = await import("react-dom/client");
	const container = window.document.createElement("div");
	window.document.body.append(container);
	const root = createRoot(container as unknown as HTMLElement);
	const client = new QueryClient({
		defaultOptions: { queries: { retry: false } },
	});
	const previous = useBackendStore.getState().backend;
	cleanup = async () => {
		await act(async () => root.unmount());
		client.clear();
		useBackendStore.setState({ backend: previous });
		await window.happyDOM.close();
	};
	const type = async (input: HTMLInputElement, value: string) => {
		await act(async () => {
			const setter = Object.getOwnPropertyDescriptor(
				window.HTMLInputElement.prototype,
				"value",
			)?.set;
			setter?.call(input, value);
			input.dispatchEvent(
				new window.Event("input", { bubbles: true }) as never,
			);
		});
	};
	const key = async (target: Element, keyName: string) => {
		await act(async () => {
			target.dispatchEvent(
				new window.KeyboardEvent("keydown", {
					key: keyName,
					bubbles: true,
				}) as never,
			);
		});
	};
	return {
		window,
		container: container as unknown as HTMLElement,
		type,
		key,
		render: (children: ReactNode) =>
			act(async () => {
				root.render(
					<QueryClientProvider client={client}>{children}</QueryClientProvider>,
				);
			}),
	};
}

const textField = {
	name: "name",
	kind: "string" as const,
	integer: false,
	nullable: true,
	temporal: null,
};

test("inline editor saves the parsed value on Enter and shows stale and errors", async () => {
	const { container, type, key, render } = await setup();
	const { InlinePropertyEditor } = await import("./ontology-property-editor");
	const { StaleObjectError } = await import(
		"../../../lib/ontology-object-edit"
	);
	const saved: unknown[] = [];
	let done = 0;
	let mode: "stale" | "error" | "ok" = "stale";
	const onSave = async (value: unknown) => {
		saved.push(value);
		if (mode === "stale") throw new StaleObjectError({ name: "Theirs" });
		if (mode === "error") throw new Error("Server said no");
	};
	await render(
		<InlinePropertyEditor
			name="name"
			value="Ada"
			editability={{ editor: "text", field: textField }}
			onSave={onSave}
			onDone={() => {
				done += 1;
			}}
		/>,
	);
	const input = container.querySelector("input") as HTMLInputElement;
	expect(input.value).toBe("Ada");
	await type(input, "Grace");
	expect(input.value).toBe("Grace");
	await key(input, "Enter");
	expect(saved).toEqual(["Grace"]);
	expect(container.textContent).toContain("Someone changed this value");
	expect(container.textContent).toContain("Theirs");
	expect(done).toBe(0);

	mode = "error";
	await key(input, "Enter");
	expect(container.querySelector('[role="alert"]')?.textContent).toContain(
		"Server said no",
	);
	expect(input.value).toBe("Grace");

	mode = "ok";
	await key(input, "Enter");
	expect(saved).toEqual(["Grace", "Grace", "Grace"]);
	expect(done).toBe(1);

	await key(input, "Escape");
	expect(done).toBe(2);
}, 30_000);

test("an unchanged value closes without saving and integer drafts validate", async () => {
	const { container, type, key, render } = await setup();
	const { InlinePropertyEditor } = await import("./ontology-property-editor");
	const saved: unknown[] = [];
	let done = 0;
	await render(
		<InlinePropertyEditor
			name="score"
			value={3}
			editability={{
				editor: "integer",
				field: { ...textField, name: "score", kind: "number", integer: true },
			}}
			onSave={async (value) => {
				saved.push(value);
			}}
			onDone={() => {
				done += 1;
			}}
		/>,
	);
	const input = container.querySelector("input") as HTMLInputElement;
	await key(input, "Enter");
	expect(saved).toEqual([]);
	expect(done).toBe(1);
	await type(input, "1.5");
	const draftError = container.querySelector('[role="alert"]');
	expect(draftError?.textContent).toBe("Enter a whole number");
	expect(draftError?.id).toBeTruthy();
	expect(input.getAttribute("aria-describedby")).toBe(draftError?.id ?? null);
	expect(input.getAttribute("aria-invalid")).toBe("true");
	await key(input, "Enter");
	expect(saved).toEqual([]);
	await type(input, "7");
	expect(container.querySelector('[role="alert"]')).toBeNull();
	expect(input.hasAttribute("aria-describedby")).toBe(false);
	await key(input, "Enter");
	expect(saved).toEqual([7]);
	await type(input, "");
	const setEmpty = [...container.querySelectorAll("button")].find((button) =>
		button.textContent?.includes("Set empty"),
	);
	await act(async () => {
		setEmpty?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
	});
	expect(input.getAttribute("placeholder")).toBe("Empty");
}, 30_000);

test("locked properties render a lock with the reason", async () => {
	const { container, render } = await setup();
	const { PropertyLockHint, PropertyEditButton } = await import(
		"./ontology-property-editor"
	);
	let clicked = 0;
	await render(
		<>
			<PropertyLockHint reason="kind" kind="geometry" />
			<PropertyEditButton
				name="title"
				onClick={() => {
					clicked += 1;
				}}
			/>
		</>,
	);
	const buttons = [...container.querySelectorAll("button")];
	expect(buttons[0].getAttribute("aria-label")).toBe(
		"Geometry values can't be edited here yet.",
	);
	expect(buttons[1].getAttribute("aria-label")).toBe("Edit title");
	await act(async () => {
		buttons[1].dispatchEvent(new MouseEvent("click", { bubbles: true }));
	});
	expect(clicked).toBe(1);
}, 30_000);

test("a temporal property is labelled and stays locked while its save runs", async () => {
	const { container, key, render } = await setup();
	const { InlinePropertyEditor } = await import("./ontology-property-editor");
	const saved: unknown[] = [];
	let settle: (() => void) | undefined;
	let done = 0;
	await render(
		<InlinePropertyEditor
			name="seen"
			value={1_700_000_000_000}
			editability={{
				editor: "temporal",
				field: {
					...textField,
					name: "seen",
					kind: "date",
					temporal: { unit: "millisecond", wire: "number" },
				},
			}}
			onSave={(value) => {
				saved.push(value);
				return new Promise<void>((resolve) => {
					settle = resolve;
				});
			}}
			onDone={() => {
				done += 1;
			}}
		/>,
	);
	const input = container.querySelector("input") as HTMLInputElement;
	const button = (label: string) =>
		[...container.querySelectorAll("button")].find((candidate) =>
			candidate.textContent?.includes(label),
		);
	expect(input.getAttribute("aria-label")).toBe("seen");
	await act(async () => {
		button("Now")?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
	});
	await key(input, "Enter");
	expect(saved).toHaveLength(1);
	expect(input.disabled).toBe(true);
	expect(button("Now")?.disabled).toBe(true);
	expect(button("Clear")?.disabled).toBe(true);

	await key(input, "Escape");
	expect(done).toBe(0);
	await act(async () => settle?.());
	expect(done).toBe(1);
}, 30_000);

test("schema fields load only when enabled", async () => {
	const { render } = await setup();
	const { useObjectEditFields } = await import("./use-object-edit-fields");
	const calls: string[] = [];
	useBackendStore.setState({
		backend: {
			dbState: {
				getSchema: async (_appId: string, table: string) => {
					calls.push(table);
					return {
						fields: [
							{ name: "id", data_type: "Utf8", nullable: false },
							{ name: "score", data_type: "Int64", nullable: true },
						],
					};
				},
			},
		} as never,
	});
	let latest: ReturnType<typeof useObjectEditFields> | undefined;
	function Probe({ enabled }: { enabled: boolean }) {
		latest = useObjectEditFields("app", ["people", "people"], false, enabled);
		return null;
	}
	await render(<Probe enabled={false} />);
	expect(calls).toEqual([]);
	await render(<Probe enabled />);
	await act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 20));
	});
	expect(calls).toEqual(["people"]);
	expect(latest?.byTable.get("people")?.get("score")?.integer).toBe(true);
	expect(latest?.failed.size).toBe(0);
}, 30_000);
