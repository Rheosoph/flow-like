// @vitest-environment happy-dom
import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test } from "vitest";
import { ChatBox, type ChatBoxModel } from "./chatbox";

let root: Root;
let host: HTMLDivElement;
const realMatchMedia = window.matchMedia;
const realWidth = window.innerWidth;

function phone(matches: boolean) {
	window.matchMedia = ((query: string) => ({
		matches: query.includes("hover: hover") ? !matches : matches,
		media: query,
		addEventListener() {},
		removeEventListener() {},
	})) as unknown as typeof window.matchMedia;
	Object.defineProperty(window, "innerWidth", {
		configurable: true,
		value: matches ? 390 : 1280,
	});
}

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	host = document.createElement("div");
	document.body.append(host);
	root = createRoot(host);
});

afterEach(async () => {
	await act(async () => root.unmount());
	host.remove();
	window.matchMedia = realMatchMedia;
	Object.defineProperty(window, "innerWidth", {
		configurable: true,
		value: realWidth,
	});
});

const model: ChatBoxModel = {
	value: "a",
	options: [{ id: "a", label: "Model A" }],
	onChange: () => {},
};

async function render(withSettings: boolean) {
	await act(async () => {
		root.render(
			<ChatBox
				onSendMessage={async () => {}}
				fileUpload
				audioInput={false}
				model={withSettings ? model : undefined}
				availableTools={withSettings ? ["Reason"] : []}
				defaultActiveTools={[]}
			/>,
		);
	});
}

const button = (label: string) =>
	Array.from(host.querySelectorAll("button")).find(
		(candidate) => candidate.getAttribute("aria-label") === label,
	);

describe("the phone composer", () => {
	test("folds the chat's controls behind + and opens them as a tray", async () => {
		phone(true);
		await render(true);
		const toggle = button("Attachments and options");
		expect(toggle).toBeDefined();
		expect(toggle?.getAttribute("aria-expanded")).toBe("false");
		const tray = host.querySelector(
			`[id="${toggle?.getAttribute("aria-controls")}"]`,
		);
		expect(tray?.hasAttribute("inert")).toBe(true);
		await act(async () => toggle?.click());
		expect(
			button("Hide attachments and options")?.getAttribute("aria-expanded"),
		).toBe("true");
		expect(tray?.hasAttribute("inert")).toBe(false);
		expect(button("Add attachment")).toBeDefined();
	});

	test("keeps + as the attach menu when the chat has nothing else to fold", async () => {
		phone(true);
		await render(false);
		expect(button("Attachments and options")).toBeUndefined();
		expect(button("Add attachment")).toBeDefined();
	});

	test("keeps the toolbar row on a desktop", async () => {
		phone(false);
		await render(true);
		expect(button("Attachments and options")).toBeUndefined();
		expect(button("Add attachment")).toBeDefined();
		expect(button("Choose tools")).toBeDefined();
	});
});
