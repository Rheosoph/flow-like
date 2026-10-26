// @vitest-environment happy-dom

import { act, useEffect } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test, vi } from "vitest";
import { LARGE_STYLESHEET_THRESHOLD } from "../../../lib/use-scoped-css";
import { ChatAppearance } from "./appearance";

type SheetOutcome = "resolve" | "reject";

const pendingSheets = vi.hoisted(
	() => [] as Array<(outcome: SheetOutcome) => void>,
);

vi.mock("@flow-like/locales", () => ({
	useTranslation: () => ({ t: (_key: string, fallback: string) => fallback }),
}));
vi.mock("../../../state/backend-state", () => ({ useBackend: () => ({}) }));
vi.mock("../../../hooks/use-asset-source", () => ({
	useAssetSource: (_appId: string | undefined, src: string | undefined) => ({
		src,
		isLoading: false,
		refresh: () => {},
	}),
}));
vi.mock("../../../lib/scoped-css-worker-client", async () => {
	const { safeScopedCss } = await import("../../../lib/css-utils");
	return {
		scopeCssOffThread: (
			css: string,
			scopeSelector: string,
			options: { scopeRoot?: boolean },
		) =>
			new Promise((resolve, reject) => {
				pendingSheets.push((outcome) =>
					outcome === "resolve"
						? resolve(safeScopedCss(css, scopeSelector, options))
						: reject(new Error("worker crashed")),
				);
			}),
	};
});

const stylesheetsAtMount: string[] = [];
let container: HTMLDivElement;
let root: Root;

/** Reads the chat theme once at mount, the way the empty-state orb does. */
function ThemeProbe() {
	useEffect(() => {
		stylesheetsAtMount.push(document.querySelector("style")?.textContent ?? "");
	}, []);
	return <div data-probe="chat" />;
}

function largeSheet(color: string) {
	const rule = `[data-fl-chat-message="assistant"] { color: ${color}; }\n`;
	return `:root { --primary: ${color}; }\n${rule.repeat(
		Math.ceil(LARGE_STYLESHEET_THRESHOLD / rule.length),
	)}`;
}

async function render(customCss: string) {
	await act(async () => {
		root.render(
			<ChatAppearance
				eventId="chat"
				config={{ custom_css: customCss }}
				fallback={<p data-probe="fallback">Loading conversation</p>}
			>
				<ThemeProbe />
			</ChatAppearance>,
		);
	});
}

async function settleSheets(outcome: SheetOutcome = "resolve") {
	await act(async () => {
		for (const settle of pendingSheets.splice(0)) settle(outcome);
		await new Promise((resolve) => setTimeout(resolve, 0));
	});
}

const probe = (name: string) =>
	container.querySelector(`[data-probe="${name}"]`);
const stylesheet = () => container.querySelector("style")?.textContent ?? "";

beforeEach(() => {
	vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
	pendingSheets.length = 0;
	stylesheetsAtMount.length = 0;
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
	vi.unstubAllGlobals();
});

describe("ChatAppearance custom stylesheet", () => {
	test("paints a small stylesheet with the chat's first commit", async () => {
		await render(":root { --primary: hotpink; }");

		expect(pendingSheets).toHaveLength(0);
		expect(probe("fallback")).toBeNull();
		expect(stylesheetsAtMount).toHaveLength(1);
		expect(stylesheetsAtMount[0]).toContain("--primary: hotpink");
	});

	test("holds the chat behind the fallback until a large stylesheet is attached", async () => {
		await render(largeSheet("hotpink"));

		expect(probe("fallback")).not.toBeNull();
		expect(probe("chat")).toBeNull();
		expect(stylesheet()).toBe("");

		await settleSheets();

		expect(probe("fallback")).toBeNull();
		expect(stylesheetsAtMount).toHaveLength(1);
		expect(stylesheetsAtMount[0]).toContain("--primary: hotpink");
	});

	test("keeps the conversation and its sheet while an edited stylesheet is scoped", async () => {
		await render(largeSheet("hotpink"));
		await settleSheets();
		await render(largeSheet("teal"));

		expect(probe("chat")).not.toBeNull();
		expect(stylesheet()).toContain("--primary: hotpink");

		await settleSheets();

		expect(stylesheet()).toContain("--primary: teal");
		expect(stylesheetsAtMount).toHaveLength(1);
	});

	test("reveals the chat unstyled when the stylesheet cannot be scoped", async () => {
		const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
		await render(largeSheet("hotpink"));
		await settleSheets("reject");

		expect(probe("chat")).not.toBeNull();
		expect(stylesheet()).toBe("");
		expect(warn).toHaveBeenCalled();
		warn.mockRestore();
	});
});
