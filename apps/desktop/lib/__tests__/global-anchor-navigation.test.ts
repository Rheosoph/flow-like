// @vitest-environment happy-dom

import { act, createElement } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import GlobalAnchorHandler from "../../components/global-anchor-component";

const mocks = vi.hoisted(() => ({
	push: vi.fn(),
	newWindow: vi.fn(),
	openUrl: vi.fn(),
}));

vi.mock("@flow-like/flow-like-ui", () => ({
	DropdownMenu: () => null,
	DropdownMenuContent: () => null,
	DropdownMenuItem: () => null,
	DropdownMenuTrigger: () => null,
}));
vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));
vi.mock("@flow-like/flow-like-ui/lib/client-navigation", () => ({
	useClientRouter: () => ({ push: mocks.push }),
	useClientHref: () => (href: string) =>
		href.includes("route=%2Forders") ? "/use/orders?id=app" : href,
}));
vi.mock("@tauri-apps/api/webviewWindow", () => ({
	WebviewWindow: class {
		constructor(label: string, options: unknown) {
			mocks.newWindow(label, options);
		}
		once() {}
	},
}));
vi.mock("@tauri-apps/plugin-opener", () => ({ openUrl: mocks.openUrl }));
vi.mock("../platform", () => ({
	isIOSDevice: () => false,
	isTauriRuntime: () => true,
}));

let root: Root;
let container: HTMLDivElement;

beforeEach(async () => {
	vi.clearAllMocks();
	vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
	window.history.replaceState(null, "", "/use/current?id=app");
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
	await act(async () => root.render(createElement(GlobalAnchorHandler)));
});

afterEach(async () => {
	await act(async () => root.unmount());
	document.body.replaceChildren();
	vi.unstubAllGlobals();
});

async function clickLink(
	href: string,
	options: MouseEventInit = {},
	prepare?: (anchor: HTMLAnchorElement) => void,
) {
	const anchor = document.createElement("a");
	anchor.href = href;
	anchor.textContent = "Open page";
	prepare?.(anchor);
	document.body.append(anchor);
	const event = new MouseEvent("click", {
		bubbles: true,
		cancelable: true,
		button: 0,
		...options,
	});
	await act(async () => {
		anchor.dispatchEvent(event);
	});
	return event;
}

test("plain local app links use the host navigation adapter", async () => {
	const event = await clickLink("/use/orders?id=app&filter=open");
	expect(event.defaultPrevented).toBe(true);
	expect(mocks.push).toHaveBeenCalledWith("/use/orders?id=app&filter=open");
});

test("component actions can consume a link before global navigation", async () => {
	await clickLink("/use/orders?id=app", {}, (anchor) => {
		anchor.addEventListener("click", (event) => event.preventDefault());
	});
	expect(mocks.push).not.toHaveBeenCalled();
});

test("same-page fragments and downloads keep browser handling", async () => {
	const fragment = await clickLink("/use/current?id=app#details");
	expect(fragment.defaultPrevented).toBe(false);
	await clickLink("/use/orders?id=app", {}, (anchor) => {
		anchor.download = "page.html";
	});
	expect(mocks.push).not.toHaveBeenCalled();
});

test("external URLs and deceptive use prefixes do not become app navigation", async () => {
	await clickLink("https://other.example.test/use/orders?id=app");
	await clickLink("tauri://other/use/orders?id=app");
	await clickLink("/users?id=app");
	expect(mocks.push).not.toHaveBeenCalled();
});

test("modified clicks open a new native window with the canonical app href", async () => {
	await clickLink("/use?id=app&route=%2Forders", { ctrlKey: true });
	expect(mocks.push).not.toHaveBeenCalled();
	expect(mocks.newWindow).toHaveBeenCalledWith(
		expect.any(String),
		expect.objectContaining({ url: "/use/orders?id=app" }),
	);
});
