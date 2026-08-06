import { describe, expect, test } from "bun:test";
import { Window } from "happy-dom";
import { act, createElement, createRef, useEffect, useRef } from "react";
import { createRoot } from "react-dom/client";
import type { ISidebarActions } from "./interfaces";

describe("interface container", () => {
	test("keeps the interface mounted when it pushes its sidebar", async () => {
		const window = new Window({ url: "https://local/use" });
		Object.assign(globalThis, {
			document: window.document,
			HTMLElement: window.HTMLElement,
			Node: window.Node,
			navigator: window.navigator,
			requestAnimationFrame: window.requestAnimationFrame.bind(window),
			cancelAnimationFrame: window.cancelAnimationFrame.bind(window),
			window,
			IS_REACT_ACT_ENVIRONMENT: true,
		});

		const { Container } = await import("./container");

		let mounts = 0;
		const ref = createRef<ISidebarActions>();

		// Mirrors how every use-interface behaves: render, then publish a sidebar
		// from a mount effect.
		function Interface() {
			const pushed = useRef(false);
			useEffect(() => {
				mounts += 1;
				if (pushed.current) return;
				pushed.current = true;
				ref.current?.pushSidebar(createElement("div", null, "history"));
			}, []);
			return createElement("div", { "data-kind": "interface" });
		}

		const host = window.document.createElement("div");
		window.document.body.appendChild(host);
		const root = createRoot(host as unknown as HTMLElement);

		await act(() => {
			root.render(
				createElement(
					Container as never,
					{ ref } as never,
					createElement("div", null, createElement(Interface)),
				),
			);
		});
		await act(async () => {
			await Promise.resolve();
		});

		expect(mounts).toBe(1);
		expect(String(host.innerHTML)).toContain('data-kind="interface"');

		root.unmount();
	}, 60000);
});
