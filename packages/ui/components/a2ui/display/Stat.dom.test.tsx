// @vitest-environment happy-dom

import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { StatComponent } from "../types";

vi.mock("../DataContext", () => ({
	useData: () => ({
		resolve: (value: { literalString?: string }) => value.literalString,
	}),
}));

import { A2UIStat } from "./Stat";

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

async function stat(extra: Record<string, string>) {
	const component = {
		type: "stat",
		label: { literalString: "Umsatz" },
		value: { literalString: "12.400 EUR" },
		...Object.fromEntries(
			Object.entries(extra).map(([k, v]) => [k, { literalString: v }]),
		),
	} as StatComponent;
	await act(() =>
		root.render(
			<A2UIStat component={component} componentId="s" surfaceId="s" />,
		),
	);
	const element = host.querySelector<HTMLElement>("[data-slot='stat']");
	if (!element) throw new Error("Expected a stat");
	return element;
}

describe("A2UI stat", () => {
	it("renders label, tabular value and caption on a quiet tile", async () => {
		const element = await stat({ caption: "letzte 30 Tage" });
		expect(element.textContent).toContain("Umsatz");
		expect(element.textContent).toContain("letzte 30 Tage");
		expect(element.querySelector(".tabular-nums")?.textContent).toBe(
			"12.400 EUR",
		);
		expect(element.className).toContain("bg-muted");
		expect(element.className).not.toContain("border");
		expect(element.querySelector("[data-slot='stat-change']")).toBeNull();
	});

	it.each([
		["up", "text-good"],
		["down", "text-critical"],
		["flat", "text-muted-foreground"],
	])("colours a %s change", async (trend, ink) => {
		const element = await stat({ change: "+4,2 %", trend });
		const change = element.querySelector("[data-slot='stat-change']");
		expect(change?.className).toContain(ink);
		expect(change?.textContent).toContain("+4,2 %");
		expect(change?.querySelector("svg")).not.toBeNull();
	});

	it("shows a change without a trend in muted ink and no arrow", async () => {
		const element = await stat({ change: "n/a" });
		const change = element.querySelector("[data-slot='stat-change']");
		expect(change?.className).toContain("text-muted-foreground");
		expect(change?.querySelector("svg")).toBeNull();
	});
});
