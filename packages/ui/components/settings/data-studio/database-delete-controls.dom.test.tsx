import { afterAll, beforeEach, describe, expect, mock, test } from "bun:test";
import { Window } from "happy-dom";
import {
	type ChangeEvent,
	type InputHTMLAttributes,
	type ReactNode,
	type TextareaHTMLAttributes,
	act,
	useState,
} from "react";
import type { IDatabaseSelector } from "../../../state/backend-state/db-state";

const window = new Window();
const globals = {
	window,
	document: window.document,
	navigator: window.navigator,
	HTMLElement: window.HTMLElement,
	Event: window.Event,
	IS_REACT_ACT_ENVIRONMENT: true,
};
// bun keeps globals and module mocks for every later file in the process, so both are
// captured first and put back in afterAll.
const globalDescriptors = Object.keys(globals).map(
	(key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const,
);
Object.assign(globalThis, globals);
const actual = {
	backendState: { ...(await import("../../../state/backend-state")) },
	reactQuery: { ...(await import("@tanstack/react-query")) },
	sonner: { ...(await import("sonner")) },
	button: { ...(await import("../../ui/button")) },
	dialog: { ...(await import("../../ui/dialog")) },
	input: { ...(await import("../../ui/input")) },
	textarea: { ...(await import("../../ui/textarea")) },
	dropdownMenu: { ...(await import("../../ui/dropdown-menu")) },
	label: { ...(await import("../../ui/label")) },
};
const { createRoot } = await import("react-dom/client");
const calls: { method: string; args: unknown[] }[] = [];
const dbState = {
	queryItems: async (...args: unknown[]) => {
		calls.push({ method: "query", args });
		return [{ id: "row-1" }];
	},
	removeItems: async (...args: unknown[]) => {
		calls.push({ method: "remove", args });
	},
	dropTable: async (...args: unknown[]) => {
		calls.push({ method: "drop", args });
		return { ontologies: [], saved_queries: [], warnings: [] };
	},
};
mock.module("../../../state/backend-state", () => ({
	...actual.backendState,
	useBackend: () => ({ dbState }),
}));
mock.module("@tanstack/react-query", () => ({
	...actual.reactQuery,
	useQueryClient: () => ({ invalidateQueries: async () => {} }),
}));
mock.module("sonner", () => ({
	...actual.sonner,
	toast: { success: () => {} },
}));
mock.module("../../ui/button", () => ({
	...actual.button,
	Button: ({
		children,
		onClick,
		disabled,
	}: { children: ReactNode; onClick?: () => void; disabled?: boolean }) => (
		<button type="button" disabled={disabled} onClick={onClick}>
			{children}
		</button>
	),
}));
const container = ({ children }: { children?: ReactNode }) => (
	<div>{children}</div>
);
mock.module("../../ui/dialog", () => ({
	...actual.dialog,
	Dialog: ({ open, children }: { open: boolean; children: ReactNode }) =>
		open ? <div>{children}</div> : null,
	DialogContent: container,
	DialogHeader: container,
	DialogTitle: container,
	DialogDescription: container,
	DialogFooter: container,
}));
mock.module("../../ui/input", () => ({
	...actual.input,
	Input: ({ onChange, ...props }: InputHTMLAttributes<HTMLInputElement>) => (
		<input
			{...props}
			onInput={(event) => onChange?.(event as ChangeEvent<HTMLInputElement>)}
		/>
	),
}));
mock.module("../../ui/textarea", () => ({
	...actual.textarea,
	Textarea: ({
		onChange,
		...props
	}: TextareaHTMLAttributes<HTMLTextAreaElement>) => (
		<textarea
			{...props}
			onInput={(event) => onChange?.(event as ChangeEvent<HTMLTextAreaElement>)}
		/>
	),
}));
mock.module("../../ui/dropdown-menu", () => ({
	...actual.dropdownMenu,
	DropdownMenuItem: ({
		children,
		onSelect,
	}: { children: ReactNode; onSelect: () => void }) => (
		<button type="button" onClick={onSelect}>
			{children}
		</button>
	),
}));
mock.module("../../ui/label", () => ({
	...actual.label,
	Label: ({ children, htmlFor }: { children: ReactNode; htmlFor: string }) => (
		<label htmlFor={htmlFor}>{children}</label>
	),
}));
const {
	DatabaseDeleteDialog,
	DatabaseDeleteMenuItems,
	databaseDeleteOperations,
} = await import("./database-delete-controls");
type Operation = Parameters<
	typeof DatabaseDeleteMenuItems
>[0]["operations"][number];
const host = document.createElement("div");
document.body.append(host);
const root = createRoot(host);

function Controls({ selector }: { selector: IDatabaseSelector }) {
	const [operation, setOperation] = useState<Operation | null>(null);
	return (
		<>
			<DatabaseDeleteMenuItems
				operations={databaseDeleteOperations(selector, true)}
				onSelect={setOperation}
			/>
			<DatabaseDeleteDialog
				operation={operation}
				onOperationChange={setOperation}
				appId="app"
				table="events"
				userScoped
				selector={selector}
				onChanged={() => {}}
				onTableDeleted={() => {}}
			/>
		</>
	);
}

async function render(selector: IDatabaseSelector) {
	await act(async () =>
		root.render(
			<Controls key={JSON.stringify(selector)} selector={selector} />,
		),
	);
}
async function click(label: string) {
	const button = Array.from(host.getElementsByTagName("button")).find(
		(item) => item.textContent === label,
	);
	expect(button).toBeDefined();
	await act(async () => button?.click());
}
async function input(id: string, value: string) {
	const field = document.getElementById(id) as HTMLInputElement;
	await act(async () => {
		field.value = value;
		field.dispatchEvent(new Event("input", { bubbles: true }));
	});
}

beforeEach(async () => {
	calls.length = 0;
	await act(async () => root.render(null));
});
afterAll(async () => {
	await act(async () => root.unmount());
	mock.restore();
	mock.module("../../../state/backend-state", () => actual.backendState);
	mock.module("@tanstack/react-query", () => actual.reactQuery);
	mock.module("sonner", () => actual.sonner);
	mock.module("../../ui/button", () => actual.button);
	mock.module("../../ui/dialog", () => actual.dialog);
	mock.module("../../ui/input", () => actual.input);
	mock.module("../../ui/textarea", () => actual.textarea);
	mock.module("../../ui/dropdown-menu", () => actual.dropdownMenu);
	mock.module("../../ui/label", () => actual.label);
	for (const [key, descriptor] of globalDescriptors) {
		if (descriptor) Object.defineProperty(globalThis, key, descriptor);
		else Reflect.deleteProperty(globalThis, key);
	}
});

describe("database destructive action targets", () => {
	test("preview and deletion preserve the explicit filter, branch and user scope", async () => {
		const selector = { branch: "experiment" };
		await render(selector);
		await click("Delete rows…");
		await input("database-delete-filter", "id = 'row-1'");
		await click("Preview matching rows");
		await input("database-delete-confirmation", "events");
		await click("Delete matching rows");
		expect(calls).toEqual([
			{
				method: "query",
				args: [
					"app",
					"events",
					{ filter: "id = 'row-1'" },
					0,
					11,
					true,
					selector,
				],
			},
			{
				method: "remove",
				args: ["app", "events", "id = 'row-1'", true, selector],
			},
		]);
	});
	test("changing the filter invalidates preview and confirmation", async () => {
		await render({ branch: "main" });
		await click("Delete rows…");
		await input("database-delete-filter", "id = 'row-1'");
		await click("Preview matching rows");
		await input("database-delete-confirmation", "events");
		await input("database-delete-filter", "id = 'row-2'");
		await click("Delete matching rows");
		expect(calls.some((call) => call.method === "remove")).toBe(false);
	});
	test("snapshots hide row deletion and explicitly describe table-wide deletion", async () => {
		await render({ branch: "experiment", version: 7 });
		expect(host.textContent).not.toContain("Delete rows");
		await click("Delete table…");
		expect(host.textContent).toContain("including every branch, version, tag");
		await input("database-delete-confirmation", "events");
		await click("Delete entire table");
		expect(calls).toEqual([{ method: "drop", args: ["app", "events", true] }]);
	});
});
