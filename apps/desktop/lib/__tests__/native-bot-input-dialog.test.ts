// @vitest-environment happy-dom
import {
	type ComponentProps,
	type PropsWithChildren,
	act,
	createElement,
} from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@flow-like/flow-like-ui/components/ui/button", () => ({
	Button: ({ children, disabled, onClick }: ComponentProps<"button">) =>
		createElement("button", { type: "button", disabled, onClick }, children),
}));
vi.mock("@flow-like/flow-like-ui/components/ui/textarea", () => ({
	Textarea: (props: ComponentProps<"textarea">) =>
		createElement("textarea", props),
}));
vi.mock("@flow-like/flow-like-ui/components/ui/dialog", () => {
	const Box = ({ children }: PropsWithChildren) =>
		createElement("div", null, children);
	return {
		Dialog: ({
			open,
			children,
			onOpenChange,
		}: PropsWithChildren<{
			open: boolean;
			onOpenChange: (open: boolean) => void;
		}>) =>
			open
				? createElement(
						"div",
						null,
						children,
						createElement(
							"button",
							{ type: "button", onClick: () => onOpenChange(false) },
							"Close dialog",
						),
					)
				: null,
		DialogContent: Box,
		DialogDescription: Box,
		DialogHeader: Box,
		DialogTitle: Box,
	};
});

import {
	type NativeBotInput,
	NativeBotInputDialog,
	type NativeBotInputResult,
} from "../../components/native-bot-input-dialog";

let root: Root;
let host: HTMLDivElement;
const input: NativeBotInput = {
	id: "capture-1",
	scope: "hub:alice",
	botId: "research",
	operation: "voice",
};
beforeEach(() => {
	vi.resetAllMocks();
	mocks.invoke.mockResolvedValue({});
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
});
afterEach(async () => {
	await act(async () => root.unmount());
	host.remove();
	vi.unstubAllGlobals();
});

async function mount(
	props: Partial<ComponentProps<typeof NativeBotInputDialog>> = {},
) {
	const callbacks = {
		isCurrent: vi.fn(() => true),
		onSend: vi.fn(async (_result: NativeBotInputResult) => {}),
		onClose: vi.fn(),
		...props,
	};
	await act(async () =>
		root.render(
			createElement(NativeBotInputDialog, {
				input,
				...callbacks,
				key: (props.input ?? input).id,
			}),
		),
	);
	return callbacks;
}

function button(label: string) {
	const element = [...host.querySelectorAll("button")].find(
		(candidate) => candidate.textContent === label,
	);
	if (!element) throw new Error(`Missing button: ${label}`);
	return element;
}
async function click(label: string) {
	await act(async () => button(label).click());
}
function textarea() {
	const element = host.querySelector("textarea");
	if (!element) throw new Error("Missing input field");
	return element;
}
async function edit(value: string) {
	await act(async () => {
		const setter = Object.getOwnPropertyDescriptor(
			HTMLTextAreaElement.prototype,
			"value",
		)?.set;
		if (!setter) throw new Error("Missing textarea value setter");
		setter.call(textarea(), value);
		textarea().dispatchEvent(new Event("input", { bubbles: true }));
	});
}

test("dictation stages editable text and sends only after explicit confirmation", async () => {
	mocks.invoke.mockResolvedValue({ text: "Captured words" });
	const { onSend, onClose } = await mount();
	expect(mocks.invoke).not.toHaveBeenCalled();
	expect(button("Send to bot").disabled).toBe(true);
	await click("Start dictation");
	expect(mocks.invoke).toHaveBeenCalledExactlyOnceWith("native_bot_request", {
		requestId: input.id,
		options: { scope: input.scope, botId: input.botId, operation: "voice" },
	});
	expect(textarea().value).toBe("Captured words");
	expect(onSend).not.toHaveBeenCalled();
	expect(onClose).not.toHaveBeenCalled();
	await edit("  Reviewed words  ");
	await click("Send to bot");
	expect(onSend).toHaveBeenCalledExactlyOnceWith({
		text: "Reviewed words",
		files: [],
	});
	expect(onClose).toHaveBeenCalledOnce();
});

test("scanned attachments can be removed before the reviewed message is sent", async () => {
	mocks.invoke.mockResolvedValue({
		text: "Scanned text",
		files: ["/shared/receipt.pdf", "/shared/private.pdf"],
	});
	const { onSend } = await mount({
		input: { ...input, operation: "document" },
	});
	await click("Open camera");
	expect(host.textContent).toContain("receipt.pdf");
	expect(host.textContent).toContain("private.pdf");
	expect(onSend).not.toHaveBeenCalled();
	const privateItem = [...host.querySelectorAll("li")].find((item) =>
		item.textContent?.includes("private.pdf"),
	);
	await act(async () => privateItem?.querySelector("button")?.click());
	await click("Send to bot");
	expect(onSend).toHaveBeenCalledExactlyOnceWith({
		text: "Scanned text",
		files: ["/shared/receipt.pdf"],
	});
});

test("closing cancels native capture and its late result cannot replace a new draft", async () => {
	const pending = Promise.withResolvers<NativeBotInputResult>();
	mocks.invoke.mockImplementation((command: string) =>
		command === "native_bot_request" ? pending.promise : Promise.resolve(),
	);
	const onClose = vi.fn(() => root.render(null));
	const first = await mount({ onClose });
	await click("Start dictation");
	expect(button("Send to bot").disabled).toBe(true);
	await click("Close dialog");
	expect(onClose).toHaveBeenCalledOnce();
	expect(mocks.invoke).toHaveBeenCalledWith("native_bot_cancel_request", {
		requestId: input.id,
	});
	const next = await mount({
		input: { ...input, id: "capture-2", text: "New draft" },
	});
	await act(async () => pending.resolve({ text: "Old private transcript" }));
	expect(textarea().value).toBe("New draft");
	expect(first.onSend).not.toHaveBeenCalled();
	expect(next.onSend).not.toHaveBeenCalled();
});

test("a scope switch rejects pending capture results and later send attempts", async () => {
	const pending = Promise.withResolvers<NativeBotInputResult>();
	mocks.invoke.mockReturnValue(pending.promise);
	const isCurrent = vi.fn(() => true);
	const { onSend, onClose } = await mount({
		isCurrent,
		input: { ...input, text: "Original draft" },
	});
	await click("Start dictation");
	isCurrent.mockReturnValue(false);
	await act(async () => pending.resolve({ text: "Other workspace result" }));
	expect(textarea().value).toBe("Original draft");
	await click("Send to bot");
	await click("Start dictation");
	expect(mocks.invoke).toHaveBeenCalledTimes(1);
	expect(onSend).not.toHaveBeenCalled();
	expect(onClose).not.toHaveBeenCalled();
});

test("unavailable local AI shows its error without sending or a cloud fallback", async () => {
	const fetch = vi.fn();
	vi.stubGlobal("fetch", fetch);
	mocks.invoke.mockRejectedValue(new Error("On-device model is unavailable"));
	const { onSend, onClose } = await mount({
		input: { ...input, operation: "summarize", text: "Private notes" },
	});
	expect(mocks.invoke).not.toHaveBeenCalled();
	await click("Process locally");
	expect(host.querySelector('[role="alert"]')?.textContent).toBe(
		"On-device model is unavailable",
	);
	expect(textarea().value).toBe("Private notes");
	expect(mocks.invoke).toHaveBeenCalledExactlyOnceWith("native_bot_request", {
		requestId: input.id,
		options: {
			scope: input.scope,
			botId: input.botId,
			operation: "summarize",
			text: "Private notes",
		},
	});
	expect(fetch).not.toHaveBeenCalled();
	expect(onSend).not.toHaveBeenCalled();
	expect(onClose).not.toHaveBeenCalled();
	mocks.invoke.mockResolvedValue({ text: "Local summary" });
	await click("Process locally");
	expect(textarea().value).toBe("Local summary");
	expect(host.querySelector('[role="alert"]')).toBeNull();
	expect(onSend).not.toHaveBeenCalled();
});

test("local processing enforces UTF-8 bytes before invoking the model", async () => {
	await mount({
		input: { ...input, operation: "extract", text: `${"é".repeat(4096)}x` },
	});
	await click("Process locally");
	expect(host.querySelector('[role="alert"]')?.textContent).toContain("8 KiB");
	expect(mocks.invoke).not.toHaveBeenCalled();
	await edit("é".repeat(4096));
	mocks.invoke.mockResolvedValue({ text: '["Extracted detail"]' });
	await click("Process locally");
	expect(mocks.invoke).toHaveBeenCalledTimes(1);
	expect(textarea().value).toBe('["Extracted detail"]');
});

test("an unavailable native result preserves input and reports that processing did not run", async () => {
	mocks.invoke.mockResolvedValue({ available: false });
	const { onSend } = await mount({
		input: { ...input, operation: "summarize", text: "Private source" },
	});
	await click("Process locally");
	expect(textarea().value).toBe("Private source");
	expect(host.querySelector('[role="alert"]')?.textContent).toContain(
		"was not processed",
	);
	expect(onSend).not.toHaveBeenCalled();
});

test("cancelled native processing leaves the source editable without claiming a result", async () => {
	mocks.invoke.mockResolvedValue({ cancelled: true });
	const { onSend } = await mount({
		input: { ...input, operation: "extract", text: "Private source" },
	});
	await click("Process locally");
	expect(textarea().getAttribute("aria-label")).toBe("Text to process");
	expect(textarea().value).toBe("Private source");
	expect(host.querySelector('[role="alert"]')).toBeNull();
	expect(onSend).not.toHaveBeenCalled();
});

test("send failure preserves reviewed content and permits explicit retry", async () => {
	const onSend = vi
		.fn()
		.mockRejectedValueOnce(new Error("Network unavailable"));
	const { onClose } = await mount({
		onSend,
		input: { ...input, text: "Reviewed draft" },
	});
	await click("Send to bot");
	expect(host.querySelector('[role="alert"]')?.textContent).toBe(
		"Network unavailable",
	);
	expect(textarea().value).toBe("Reviewed draft");
	expect(onClose).not.toHaveBeenCalled();
	await click("Send to bot");
	expect(onSend).toHaveBeenCalledTimes(2);
	expect(onClose).toHaveBeenCalledOnce();
});

test("send completion after a scope switch does not close the new context", async () => {
	const pending = Promise.withResolvers<void>();
	const isCurrent = vi.fn(() => true);
	const onSend = vi.fn(() => pending.promise);
	const { onClose } = await mount({
		isCurrent,
		onSend,
		input: { ...input, text: "Reviewed draft" },
	});
	await click("Send to bot");
	isCurrent.mockReturnValue(false);
	await act(async () => pending.resolve());
	expect(onSend).toHaveBeenCalledOnce();
	expect(onClose).not.toHaveBeenCalled();
});

test("a send finishing after dismissal cannot close a replacement dialog", async () => {
	const pending = Promise.withResolvers<void>();
	const onClose = vi.fn(() => root.render(null));
	await mount({
		onSend: () => pending.promise,
		onClose,
		input: { ...input, text: "First draft" },
	});
	await click("Send to bot");
	await click("Close dialog");
	await mount({ input: { ...input, id: "capture-2", text: "New draft" } });
	await act(async () => pending.resolve());
	expect(onClose).toHaveBeenCalledTimes(1);
	expect(textarea().value).toBe("New draft");
});
