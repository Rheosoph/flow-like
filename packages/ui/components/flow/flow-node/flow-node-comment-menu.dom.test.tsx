// @vitest-environment happy-dom
import { act, useState } from "react";
import { type Root, createRoot } from "react-dom/client";
import {
	type Mock,
	afterEach,
	beforeEach,
	describe,
	expect,
	it,
	vi,
} from "vitest";
import type { INode } from "../../../lib/schema/flow/node";
import { FlowNodeCommentMenu } from "./flow-node-comment-menu";

const backend = vi.hoisted(() => ({
	getBoard: vi.fn(),
	executeCommand: vi.fn(),
	pushCommand: vi.fn(),
	invalidate: vi.fn(),
}));

vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));
vi.mock("../../../hooks", () => ({
	useInvalidateInvoke: () => backend.invalidate,
}));
vi.mock("../../../lib", () => ({
	updateNodeCommand: (value: { node: INode }) => value,
}));
vi.mock("../../../state/backend-state", () => ({
	useBackend: () => ({ boardState: backend }),
}));
vi.mock("../flow-history", () => ({
	useUndoRedo: () => ({ pushCommand: backend.pushCommand }),
}));

const node: INode = {
	id: "node",
	name: "node",
	friendly_name: "Node",
	description: "",
	category: "test",
	pins: {},
	comment: "Original comment",
};

let root: Root;
let container: HTMLDivElement;
let onOpenChange: Mock<(open: boolean) => void>;

function Harness() {
	const [open, setOpen] = useState(true);
	return open ? (
		<FlowNodeCommentMenu
			node={node}
			appId="app"
			boardId="board"
			open={open}
			onOpenChange={(value) => {
				onOpenChange(value);
				setOpen(value);
			}}
		/>
	) : null;
}

function textarea() {
	const element = document.querySelector("textarea");
	if (!element) throw new Error("Missing comment field");
	return element;
}

function button(label: string) {
	const element = [...document.querySelectorAll("button")].find(
		(candidate) => candidate.textContent?.trim() === label,
	);
	if (!element) throw new Error(`Missing button: ${label}`);
	return element;
}

async function edit(content: string) {
	await act(async () => {
		const field = textarea();
		Object.getOwnPropertyDescriptor(
			HTMLTextAreaElement.prototype,
			"value",
		)?.set?.call(field, content);
		field.dispatchEvent(new Event("input", { bubbles: true }));
	});
}

async function dismiss(method: "outside" | "close" | "escape") {
	await act(async () => {
		if (method === "close") button("Close").click();
		else if (method === "escape") {
			document.dispatchEvent(
				new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
			);
		} else {
			document.querySelector('[data-slot="dialog-overlay"]')?.dispatchEvent(
				new PointerEvent("pointerdown", {
					bubbles: true,
					pointerType: "mouse",
					button: 0,
				}),
			);
		}
	});
}

beforeEach(async () => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	vi.clearAllMocks();
	backend.getBoard.mockResolvedValue({ nodes: { node } });
	backend.executeCommand.mockResolvedValue({ id: "saved" });
	backend.pushCommand.mockResolvedValue(undefined);
	backend.invalidate.mockResolvedValue(undefined);
	onOpenChange = vi.fn<(open: boolean) => void>();
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
	await act(async () => root.render(<Harness />));
	await act(async () => new Promise((resolve) => setTimeout(resolve, 0)));
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
});

describe("node comment dismissal", () => {
	it.each(["outside", "close", "escape"] as const)(
		"saves the draft before closing via %s",
		async (method) => {
			await edit("Updated comment");
			await dismiss(method);
			expect(backend.executeCommand).toHaveBeenCalledExactlyOnceWith(
				"app",
				"board",
				{ node: { ...node, comment: "Updated comment" } },
			);
			expect(backend.pushCommand).toHaveBeenCalledWith({ id: "saved" });
			expect(onOpenChange).toHaveBeenCalledExactlyOnceWith(false);
			expect(document.querySelector('[role="dialog"]')).toBeNull();
		},
	);

	it("closes an untouched draft without writing over a newer comment", async () => {
		backend.getBoard.mockResolvedValue({
			nodes: { node: { ...node, comment: "A newer comment" } },
		});
		await dismiss("close");
		expect(backend.getBoard).not.toHaveBeenCalled();
		expect(backend.executeCommand).not.toHaveBeenCalled();
		expect(onOpenChange).toHaveBeenCalledWith(false);
	});

	it("keeps the draft open after a failed save and allows retry", async () => {
		backend.executeCommand.mockRejectedValueOnce(new Error("Save failed"));
		await edit("Keep this draft");
		await dismiss("outside");
		expect(onOpenChange).not.toHaveBeenCalled();
		expect(textarea().value).toBe("Keep this draft");
		expect(textarea().disabled).toBe(false);
		expect(document.querySelector('[role="alert"]')?.textContent).toBe(
			"Save failed",
		);
		await dismiss("close");
		expect(backend.executeCommand).toHaveBeenCalledTimes(2);
		expect(onOpenChange).toHaveBeenCalledWith(false);
	});

	it("keeps a conflicting draft open without replacing the newer comment", async () => {
		backend.getBoard.mockResolvedValue({
			nodes: { node: { ...node, comment: "A newer comment" } },
		});
		await edit("Keep this draft");
		await dismiss("escape");
		expect(backend.executeCommand).not.toHaveBeenCalled();
		expect(onOpenChange).not.toHaveBeenCalled();
		expect(textarea().value).toBe("Keep this draft");
		expect(document.querySelector('[role="alert"]')?.textContent).toContain(
			"changed while you were editing",
		);
	});

	it("blocks edits and duplicate dismissal while saving", async () => {
		let finishSave!: (value: { id: string }) => void;
		backend.executeCommand.mockReturnValueOnce(
			new Promise((resolve) => {
				finishSave = resolve;
			}),
		);
		await edit("Pending draft");
		await dismiss("outside");
		expect(textarea().disabled).toBe(true);
		expect(button("Cancel").disabled).toBe(true);
		expect(button("Save").disabled).toBe(true);
		await dismiss("close");
		await dismiss("escape");
		expect(backend.executeCommand).toHaveBeenCalledTimes(1);
		expect(onOpenChange).not.toHaveBeenCalled();
		await act(async () => finishSave({ id: "saved" }));
		expect(onOpenChange).toHaveBeenCalledExactlyOnceWith(false);
	});

	it("discards only when Cancel is chosen", async () => {
		await edit("Discard this draft");
		await act(async () => button("Cancel").click());
		expect(backend.executeCommand).not.toHaveBeenCalled();
		expect(onOpenChange).toHaveBeenCalledExactlyOnceWith(false);
	});
});
