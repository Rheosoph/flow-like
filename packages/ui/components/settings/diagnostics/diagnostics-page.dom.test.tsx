// @vitest-environment happy-dom
import React, { type ReactNode, act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
	clearRealtimeStatus,
	useRealtimeStatus,
} from "../../../lib/realtime/realtime-status";
import { DiagnosticsPage } from "./diagnostics-page";

const fixture = vi.hoisted(() => ({
	toast: { success: vi.fn(), error: vi.fn() },
	writeText: vi.fn(async (_text: string) => undefined),
}));

vi.mock("sonner", () => ({ toast: fixture.toast }));
vi.mock("next/link", () => ({
	default: ({ children, href }: { children: ReactNode; href: string }) => (
		<a href={href}>{children}</a>
	),
}));
vi.mock("../../../state/backend-state", () => ({
	useBackend: () => ({ userState: { getInfo: async () => ({}) } }),
	useBackendReady: () => true,
	useAuthStatusStore: (select: (state: { signedIn?: boolean }) => unknown) =>
		select({ signedIn: true }),
}));
vi.mock("../../../hooks/use-invoke", () => ({
	useInvoke: () => ({ data: { id: "user-42" } }),
}));
vi.mock("../../../hooks/use-developer-mode", () => ({
	useDeveloperMode: () => ({
		developerMode: true,
		setDeveloperMode: () => undefined,
	}),
}));

const BUILD = {
	version: "1.2.3",
	commit: "76cce5ff0abc",
	builtAt: "2026-10-21T08:00:00.000Z",
	mode: "production",
};

let root: Root;
let container: HTMLDivElement;

async function until(predicate: () => boolean) {
	for (let attempt = 0; attempt < 100 && !predicate(); attempt++) {
		await act(async () => {
			await new Promise((resolve) => setTimeout(resolve, 10));
		});
	}
	expect(predicate()).toBe(true);
}

function button(label: string) {
	const match = [...container.querySelectorAll("button")].find((entry) =>
		entry.textContent?.includes(label),
	);
	if (!match) throw new Error(`No button labelled ${label}`);
	return match;
}

beforeEach(() => {
	Object.assign(globalThis, { IS_REACT_ACT_ENVIRONMENT: true, React });
	Object.defineProperty(navigator, "clipboard", {
		configurable: true,
		value: { writeText: fixture.writeText },
	});
	fixture.writeText.mockClear();
	fixture.toast.success.mockClear();
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
	clearRealtimeStatus();
});

describe("DiagnosticsPage", () => {
	it("shows skeletons first, then the probed sections and session", async () => {
		await act(async () => {
			root.render(
				<DiagnosticsPage
					build={BUILD}
					apiBaseUrl="https://api.example.com"
					storageHref="/settings/storage"
				/>,
			);
		});
		const ready = () => container.textContent?.includes("user-42") ?? false;
		if (!ready()) {
			expect(container.querySelector('[aria-busy="true"]')).not.toBeNull();
		}
		await until(ready);
		const text = container.textContent ?? "";
		expect(text).toContain("About & diagnostics");
		expect(text).toContain("1.2.3");
		expect(text).toContain("76cce5f");
		expect(text).toContain("https://api.example.com");
		expect(text).toContain("WebAssembly SIMD");
		expect(text).toContain("React Flow");
		expect(
			container.querySelector('a[href="/settings/storage"]'),
		).not.toBeNull();
		expect(container.querySelector('a[href="/settings"]')).not.toBeNull();
	});

	it("copies the plain-text report with build, API and session", async () => {
		await act(async () => {
			root.render(
				<DiagnosticsPage build={BUILD} apiBaseUrl="https://api.example.com" />,
			);
		});
		await until(() => !button("Copy report").disabled);
		await act(async () => {
			button("Copy report").click();
		});
		await until(() => fixture.writeText.mock.calls.length === 1);
		const report = fixture.writeText.mock.calls[0]?.[0] ?? "";
		expect(report).toContain("# Flow-Like diagnostics");
		expect(report).toContain("Version: 1.2.3");
		expect(report).toContain("API base URL: https://api.example.com");
		expect(report).toContain("Account ID: user-42");
		expect(report).toContain("Developer mode: yes");
		expect(report).toContain("Live updates: not reported");
		expect(fixture.toast.success).toHaveBeenCalledOnce();
	});

	it("shows that live updates fell back to polling and where the provider failed", async () => {
		useRealtimeStatus.setState({
			status: {
				runner: "in-tab",
				state: "connected",
				provider: {
					transport: "aws_mqtt",
					live: false,
					failure: {
						stage: "handshake",
						message: "no answer within 10 s",
						elapsedMs: 10_004,
						at: Date.UTC(2026, 10, 9, 12),
					},
				},
			},
		});
		await act(async () => {
			root.render(<DiagnosticsPage build={BUILD} />);
		});
		await until(() => container.textContent?.includes("user-42") ?? false);
		const text = container.textContent ?? "";
		expect(text).toContain("Live updates");
		expect(text).toContain("Polling");
		expect(text).toContain(
			"Runs in this tab because the shared worker did not start.",
		);
		expect(text).toContain(
			"WebSocket handshake after 10.0 s (no answer within 10 s)",
		);

		await act(async () => {
			useRealtimeStatus.setState({
				status: {
					runner: "shared-worker",
					state: "connected",
					provider: { transport: "aws_mqtt", live: true },
				},
			});
		});
		expect(container.textContent).toContain("Live via AWS IoT");
	});
});
