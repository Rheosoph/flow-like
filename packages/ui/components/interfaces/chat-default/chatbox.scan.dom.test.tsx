// @vitest-environment happy-dom
import { act, createRef } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import {
	type ComposerAttachSource,
	useComposerCaptureStore,
} from "../../../state/composer-capture";
import { ChatBox, type ChatBoxRef } from "./chatbox";

let root: Root;
let host: HTMLDivElement;
let unregister: (() => void) | undefined;

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
});

afterEach(async () => {
	await act(async () => root.unmount());
	host.remove();
	unregister?.();
	unregister = undefined;
});

function registerDevice(sources: ComposerAttachSource[]) {
	unregister = useComposerCaptureStore.getState().register(sources);
}

async function render({
	fileUpload = true,
	attachSources,
}: {
	fileUpload?: boolean;
	attachSources?: readonly ComposerAttachSource[];
} = {}) {
	const ref = createRef<ChatBoxRef>();
	await act(async () => {
		root.render(
			<ChatBox
				ref={ref}
				onSendMessage={async () => {}}
				fileUpload={fileUpload}
				audioInput={false}
				availableTools={[]}
				defaultActiveTools={[]}
				attachSources={attachSources}
			/>,
		);
	});
	return ref;
}

/** The menu portals into `document.body`, outside the host. */
function menuEntries() {
	return Array.from(
		document.body.querySelectorAll<HTMLElement>(
			"[data-radix-popper-content-wrapper] button, [data-radix-popper-content-wrapper] label",
		),
	).map((entry) => entry.textContent);
}

function menuEntry(label: string) {
	return Array.from(
		document.body.querySelectorAll<HTMLElement>("button, label"),
	).find((candidate) => candidate.textContent === label);
}

function attachButton() {
	return Array.from(host.querySelectorAll("button")).find(
		(candidate) => candidate.getAttribute("aria-label") === "Add attachment",
	);
}

async function openAttachMenu() {
	await act(async () => attachButton()?.click());
}

describe("ChatBox attach sources", () => {
	test("the attach menu offers only files and folders without sources", async () => {
		await render();
		await openAttachMenu();

		expect(menuEntries()).toEqual(["Files", "Folder"]);
	});

	test("device and chat sources are listed in menu order before files", async () => {
		registerDevice([
			{ kind: "take_photo", attach: async () => ({}) },
			{ kind: "barcode", attach: async () => ({}) },
			{ kind: "scan_document", attach: async () => ({}) },
		]);
		const attachSources = [
			{ kind: "storage" as const, attach: async () => ({}) },
		];
		await render({ attachSources });
		await openAttachMenu();

		expect(menuEntries()).toEqual([
			"Scan document",
			"Scan barcodes",
			"Take photo",
			"From storage",
			"Files",
			"Folder",
		]);
	});

	test("a finished scan is attached and closes the menu", async () => {
		const scan = vi.fn(async () => ({
			files: [
				new File(["%PDF"], "scan-2026-11-10-120000.pdf", {
					type: "application/pdf",
				}),
			],
		}));
		registerDevice([{ kind: "scan_document", attach: scan }]);
		const ref = await render();
		await openAttachMenu();

		await act(async () => menuEntry("Scan document")?.click());

		expect(scan).toHaveBeenCalledTimes(1);
		expect(ref.current?.getAttachedFiles().map((file) => file.name)).toEqual([
			"scan-2026-11-10-120000.pdf",
		]);
		expect(menuEntry("Files")).toBeUndefined();
	});

	test("scanned text is appended to the draft", async () => {
		registerDevice([
			{ kind: "scan_text", attach: async () => ({ text: " Invoice 42 " }) },
		]);
		const ref = await render();
		await act(async () => ref.current?.setInput("Total?"));
		await openAttachMenu();

		await act(async () => menuEntry("Scan text")?.click());

		expect(ref.current?.getInput()).toBe("Total?\n\nInvoice 42");
	});

	test("a cancelled capture changes nothing", async () => {
		registerDevice([{ kind: "scan_document", attach: async () => ({}) }]);
		const ref = await render();
		await openAttachMenu();

		await act(async () => menuEntry("Scan document")?.click());

		expect(ref.current?.getAttachedFiles()).toEqual([]);
		expect(ref.current?.getInput()).toBe("");
	});

	test("a chat without file upload still offers text sources only", async () => {
		registerDevice([
			{ kind: "scan_document", attach: async () => ({}) },
			{ kind: "location", attach: async () => ({}) },
		]);
		await render({ fileUpload: false });
		await openAttachMenu();

		expect(menuEntries()).toEqual(["Current location"]);
	});

	test("a chat without file upload and without text sources has no plus", async () => {
		registerDevice([{ kind: "take_photo", attach: async () => ({}) }]);
		await render({ fileUpload: false });

		expect(attachButton()).toBeUndefined();
	});
});
