// @vitest-environment happy-dom

import { act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, test } from "vitest";
import { TerminalCard } from "./tool-terminal";

let host: HTMLDivElement;
let root: Root;

beforeEach(() => {
	(
		globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
	).IS_REACT_ACT_ENVIRONMENT = true;
	host = document.createElement("div");
	document.body.appendChild(host);
	root = createRoot(host);
});

afterEach(() => {
	act(() => root.unmount());
	host.remove();
});

const render = (terminal: Parameters<typeof TerminalCard>[0]["terminal"]) =>
	act(() => root.render(<TerminalCard terminal={terminal} />));

describe("TerminalCard", () => {
	test("is collapsed by default and opens the output on click", () => {
		render({ command: "ls -la", exitCode: 0, output: "a.txt\nb.txt" });
		expect(host.textContent).toContain("ls -la");
		expect(host.textContent).toContain("exit 0");
		expect(host.querySelector("pre")).toBeNull();
		act(() => host.querySelector("button")?.click());
		expect(host.querySelector("pre")?.textContent).toBe("a.txt\nb.txt");
	});

	test("shows a failing exit code and a timeout", () => {
		render({ command: "false", exitCode: 1, output: "" });
		expect(host.querySelector("[data-terminal-exit]")?.textContent).toBe(
			"exit 1",
		);
		render({ command: "sleep 99", exitCode: null, timedOut: true });
		expect(host.querySelector("[data-terminal-exit]")?.textContent).toBe(
			"timed out",
		);
		expect(host.querySelector("button")?.disabled).toBe(true);
	});
});
