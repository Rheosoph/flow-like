// @vitest-environment happy-dom

import { type ComponentProps, act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { type IComment, ICommentType } from "../../lib/schema/flow/board";
import { CommentNode } from "./comment-node";

vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));
vi.mock("@xyflow/react", () => ({
	NodeResizer: () => null,
	useReactFlow: () => ({
		getNodes: () =>
			current ? [{ id: current.id, data: { comment: current } }] : [],
	}),
}));
vi.mock("../ui", async () => ({
	Button: (await import("../ui/button")).Button,
	TextEditor: ({
		initialContent,
		editable,
		onChange,
	}: {
		initialContent: string;
		editable: boolean;
		onChange?: (content: string) => void;
	}) =>
		editable ? (
			<textarea
				aria-label="Comment text"
				defaultValue={initialContent}
				onInput={(event) => onChange?.(event.currentTarget.value)}
			/>
		) : (
			<p>{initialContent}</p>
		),
}));
vi.mock("./comment-node/comment-color-picker", () => ({
	CommentColorPicker: () => null,
}));
vi.mock("./comment-node/comment-node-toolbar", () => ({
	CommentNodeToolbar: ({ onEdit }: { onEdit: () => void }) => (
		<button type="button" onClick={onEdit}>
			Edit
		</button>
	),
}));
vi.mock("sonner", () => ({ toast: { success: vi.fn() } }));

let root: Root;
let container: HTMLDivElement;
let current: IComment;
const upsert = vi.fn<(comment: IComment) => Promise<void>>();

beforeEach(async () => {
	vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
	current = {
		id: "comment",
		content: "Original comment",
		comment_type: ICommentType.Text,
		coordinates: [0, 0, 0],
		timestamp: { secs_since_epoch: 1, nanos_since_epoch: 0 },
	};
	upsert.mockReset().mockImplementation(async (comment) => {
		current = comment;
	});
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
	const onUpsert: CommentNode["data"]["onUpsert"] = upsert;
	const props = {
		id: current.id,
		selected: true,
		data: {
			comment: current,
			onUpsert,
			appId: "app",
			boardId: "board",
			hash: "hash",
		},
	} as ComponentProps<typeof CommentNode>;
	await act(async () => root.render(<CommentNode {...props} />));
	await click("Edit");
	// Radix installs its outside-pointer listener after opening the dialog.
	await new Promise((resolve) => setTimeout(resolve, 0));
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
	vi.unstubAllGlobals();
});

function dialog() {
	return document.querySelector('[role="dialog"]');
}

function editor() {
	const element = document.querySelector<HTMLTextAreaElement>(
		'textarea[aria-label="Comment text"]',
	);
	if (!element) throw new Error("Missing comment editor");
	return element;
}

function button(label: string) {
	const element = [...document.querySelectorAll("button")].find(
		(candidate) => candidate.textContent === label,
	);
	if (!element) throw new Error(`Missing button: ${label}`);
	return element;
}

async function click(label: string) {
	await act(async () => button(label).click());
}

async function edit(content = "Edited comment") {
	await act(async () => {
		const input = editor();
		input.value = content;
		input.dispatchEvent(new Event("input", { bubbles: true }));
	});
}

async function dismiss(method: string) {
	await act(async () => {
		if (method === "outside click") {
			document.querySelector('[data-slot="dialog-overlay"]')?.dispatchEvent(
				new PointerEvent("pointerdown", {
					bubbles: true,
					pointerType: "mouse",
				}),
			);
		} else if (method === "Escape") {
			editor().dispatchEvent(
				new KeyboardEvent("keydown", { key: "Escape", bubbles: true }),
			);
		} else {
			button(method).click();
		}
	});
}

it.each(["outside click", "Close", "Escape", "Save"])(
	"saves the edited comment before closing via %s",
	async (method) => {
		await edit();
		current = { ...current, color: "blue", coordinates: [10, 20, 0] };
		await dismiss(method);
		expect(upsert).toHaveBeenCalledExactlyOnceWith(
			expect.objectContaining({
				content: "Edited comment",
				color: "blue",
				coordinates: [10, 20, 0],
			}),
		);
		expect(dialog()).toBeNull();
		await click("Edit");
		expect(editor().value).toBe("Edited comment");
	},
);

it("closes an untouched draft without overwriting a newer comment", async () => {
	current = { ...current, content: "Changed elsewhere" };
	await dismiss("Close");
	expect(upsert).not.toHaveBeenCalled();
	expect(dialog()).toBeNull();
});

it("keeps the draft open after a failed dismissal save and allows retry", async () => {
	upsert.mockRejectedValueOnce(new Error("Unable to save"));
	await edit();
	await dismiss("outside click");
	expect(editor().value).toBe("Edited comment");
	expect(document.querySelector('[role="alert"]')?.textContent).toBe(
		"Unable to save",
	);
	await dismiss("Close");
	expect(upsert).toHaveBeenCalledTimes(2);
	expect(dialog()).toBeNull();
});

it("preserves the draft when another writer changes the comment", async () => {
	await edit();
	current = { ...current, content: "Changed elsewhere" };
	await dismiss("Escape");
	expect(upsert).not.toHaveBeenCalled();
	expect(editor().value).toBe("Edited comment");
	expect(document.querySelector('[role="alert"]')?.textContent).toContain(
		"changed while you were editing",
	);
});

it("waits for the pending save and blocks duplicate dismissals", async () => {
	let finish!: () => void;
	upsert.mockImplementationOnce(
		() =>
			new Promise<void>((resolve) => {
				finish = resolve;
			}),
	);
	await edit();
	const input = editor();
	await dismiss("Close");
	expect(dialog()).not.toBeNull();
	expect(input.closest("[inert]")).not.toBeNull();
	expect(button("Cancel").disabled).toBe(true);
	await dismiss("Close");
	await dismiss("Escape");
	expect(upsert).toHaveBeenCalledTimes(1);
	expect(editor()).toBe(input);
	expect(input.value).toBe("Edited comment");
	await act(async () => finish());
	expect(dialog()).toBeNull();
});

it("still discards the draft when Cancel is explicitly clicked", async () => {
	await edit();
	await click("Cancel");
	expect(upsert).not.toHaveBeenCalled();
	expect(dialog()).toBeNull();
	await click("Edit");
	expect(editor().value).toBe("Original comment");
});
