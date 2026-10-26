// @vitest-environment happy-dom
import { act } from "react";
import { type Root, hydrateRoot } from "react-dom/client";
import { renderToString } from "react-dom/server";
import { afterEach, beforeEach, expect, it, vi } from "vitest";

const harness = vi.hoisted(() => ({
	prefetch: vi.fn(),
	providerReady: undefined as Promise<unknown> | undefined,
}));
vi.mock("next/navigation", () => ({ usePathname: () => "/use/people" }));
vi.mock("../lib/oidc-configuration", () => ({
	prefetchWebOidcConfiguration: harness.prefetch,
}));
vi.mock("./web-navigation-provider", () => ({
	WebNavigationProvider: ({ children }: { children: React.ReactNode }) =>
		children,
}));
vi.mock("next/dynamic", async () => {
	const { lazy } = await import("react");
	return {
		default: (load: () => Promise<React.ComponentType>) =>
			lazy(() => {
				const loaded = load().then((component) => ({ default: component }));
				harness.providerReady = loaded;
				return loaded;
			}),
	};
});
vi.mock("./app-providers", async () => {
	const { ThemeProvider } = await import("next-themes");
	return {
		AppProviders: ({ children }: { children: React.ReactNode }) => (
			<ThemeProvider attribute="class" defaultTheme="system">
				{children}
			</ThemeProvider>
		),
	};
});

let root: Root | undefined;
let container: HTMLDivElement;
beforeEach(() => {
	(
		globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
	).IS_REACT_ACT_ENVIRONMENT = true;
	localStorage.clear();
	harness.prefetch.mockClear();
	container = document.createElement("div");
	document.body.appendChild(container);
});
afterEach(async () => {
	if (root) await act(async () => root?.unmount());
	container.remove();
	vi.unstubAllGlobals();
});

it("hydrates the existing server-only theme script without replacing the provider tree", async () => {
	const browserWindow = window;
	vi.stubGlobal("window", undefined);
	let html: string;
	try {
		vi.resetModules();
		const { ClientProviders } = await import("./client-providers");
		const content = (
			<ClientProviders>
				<p>Loading account</p>
			</ClientProviders>
		);
		// Next waits for the provider import before finishing its static export.
		try {
			renderToString(content);
		} catch (error) {
			if (
				!(error instanceof Error) ||
				!error.message.includes("A component suspended")
			)
				throw error;
		}
		await harness.providerReady;
		html = renderToString(content);
		expect(harness.prefetch).not.toHaveBeenCalled();
	} finally {
		vi.stubGlobal("window", browserWindow);
	}
	// The installed next-themes patch emits this script only during SSR.
	expect(html).toContain("<script");
	container.innerHTML = html;
	const serverContent = container.querySelector("p");
	const recoverableErrors: unknown[] = [];
	vi.resetModules();
	const { ClientProviders } = await import("./client-providers");
	await act(async () => {
		root = hydrateRoot(
			container,
			<ClientProviders>
				<p>Loading account</p>
			</ClientProviders>,
			{ onRecoverableError: (error) => recoverableErrors.push(error) },
		);
	});
	expect(recoverableErrors).toEqual([]);
	expect(container.querySelector("p")).toBe(serverContent);
	expect(harness.prefetch).toHaveBeenCalledOnce();
});
