// @vitest-environment happy-dom
import { StrictMode, act } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const harness = vi.hoisted(() => ({
	pathname: "/use/people" as string | null,
	prefetch: vi.fn(),
	providerLoads: vi.fn(),
}));

vi.mock("next/navigation", () => ({ usePathname: () => harness.pathname }));
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
			lazy(async () => {
				harness.providerLoads();
				return { default: await load() };
			}),
	};
});
vi.mock("./app-providers", () => new Promise<never>(() => {}));

let ClientProviders: typeof import("./client-providers").ClientProviders;
let root: Root;
let container: HTMLDivElement;

beforeEach(async () => {
	vi.resetModules();
	({ ClientProviders } = await import("./client-providers"));
	(
		globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }
	).IS_REACT_ACT_ENVIRONMENT = true;
	harness.pathname = "/use/people";
	harness.prefetch.mockClear();
	harness.providerLoads.mockClear();
	container = document.createElement("div");
	document.body.appendChild(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
});

async function render() {
	await act(async () =>
		root.render(
			<ClientProviders>
				<p>Published content</p>
			</ClientProviders>,
		),
	);
}

describe("ClientProviders startup", () => {
	it("starts the public configuration request while account providers are still loading", async () => {
		await render();
		expect(harness.prefetch).toHaveBeenCalledOnce();
		expect(container.textContent).toBe("");
	});

	it.each(["/a", "/a/app-1/people"])(
		"keeps anonymous hosted route %s independent of account startup",
		async (pathname) => {
			harness.pathname = pathname;
			await render();
			expect(container.textContent).toBe("Published content");
			expect(harness.prefetch).not.toHaveBeenCalled();
			expect(harness.providerLoads).not.toHaveBeenCalled();
		},
	);

	it("preserves account provider loading while the router pathname is unknown", async () => {
		harness.pathname = null;
		await render();
		expect(harness.prefetch).toHaveBeenCalledOnce();
		expect(container.textContent).toBe("");
	});

	it("prefetches when navigating into the account app without repeating on every account route", async () => {
		harness.pathname = "/a/app-1";
		await render();
		harness.pathname = "/use/people";
		await render();
		expect(harness.prefetch).toHaveBeenCalledOnce();
		harness.pathname = "/settings";
		await render();
		expect(harness.prefetch).toHaveBeenCalledOnce();
	});

	it("starts only one prefetch while the provider chunk suspends under StrictMode", async () => {
		await act(async () =>
			root.render(
				<StrictMode>
					<ClientProviders>
						<p>Content</p>
					</ClientProviders>
				</StrictMode>,
			),
		);
		expect(harness.prefetch).toHaveBeenCalledOnce();
	});
});
