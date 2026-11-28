// @vitest-environment happy-dom

import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { BadgeComponent } from "../types";

vi.mock("../DataContext", () => ({
	useData: () => ({
		resolve: (value: { literalString?: string }) => value.literalString,
	}),
}));

import { A2UIBadge } from "./Badge";

let root: Root;
let host: HTMLDivElement;

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true });
	host = document.createElement("div");
	document.body.appendChild(host);
	root = createRoot(host);
});

afterEach(async () => {
	await act(() => root.unmount());
	host.remove();
});

async function badge(extra: Record<string, string>) {
	const component = {
		type: "badge",
		content: { literalString: "Neu" },
		...Object.fromEntries(
			Object.entries(extra).map(([k, v]) => [k, { literalString: v }]),
		),
	} as BadgeComponent;
	await act(() =>
		root.render(
			<A2UIBadge
				component={component}
				componentId="b"
				surfaceId="s"
				renderChild={() => null}
			/>,
		),
	);
	const element = host.querySelector<HTMLElement>("[data-slot='badge']");
	if (!element) throw new Error("Expected a badge");
	return element;
}

describe("A2UI badge tone", () => {
	it.each([
		["neutral", "bg-unknown-bg", "text-unknown"],
		["info", "bg-info-bg", "text-info"],
		["success", "bg-good-bg", "text-good"],
		["warning", "bg-warning-bg", "text-warning"],
		["danger", "bg-critical-bg", "text-critical"],
	])("renders %s as a soft tint", async (tone, fill, ink) => {
		const element = await badge({ tone });
		expect(element.className).toContain(fill);
		expect(element.className).toContain(ink);
		expect(element.className).not.toContain("bg-primary");
	});

	it("lets tone win over variant", async () => {
		const element = await badge({ variant: "destructive", tone: "success" });
		expect(element.className).toContain("bg-good-bg");
		expect(element.className).not.toContain("bg-destructive");
	});

	it("keeps the variant look without a tone", async () => {
		const element = await badge({ variant: "secondary" });
		expect(element.className).toContain("bg-secondary");
	});
});
