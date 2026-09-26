// @vitest-environment happy-dom

import { UseRoutePage } from "@flow-like/flow-like-ui/components/interfaces/use-route-page";
import { useClientRouter } from "@flow-like/flow-like-ui/lib/client-navigation";
import { UseNavigationProvider } from "@flow-like/flow-like-ui/lib/use-navigation-provider";
import { useSearchParams } from "next/navigation";
import { act, createElement } from "react";
import { type Root, createRoot } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";

vi.mock("next/navigation", () => ({
	useRouter: () => native,
	usePathname: () => window.location.pathname,
	useSearchParams: () => new URLSearchParams(window.location.search),
}));

vi.mock(
	"@flow-like/flow-like-ui/components/interfaces/use-page-content",
	() => ({
		UsePageContent: ({ routePath }: { routePath?: string }) => {
			const params = useSearchParams();
			return createElement(
				"div",
				null,
				routePath ?? params.get("route") ?? `event:${params.get("eventId")}`,
			);
		},
	}),
);

let root: Root;
let container: HTMLDivElement;
let router: ReturnType<typeof useClientRouter>;
let routeMode: "path" | "query";
const originalPush = window.history.pushState.bind(window.history);
const originalReplace = window.history.replaceState.bind(window.history);
const native = {
	push: vi.fn((href: string) => {
		originalPush(null, "", href);
		render();
	}),
	replace: vi.fn((href: string) => {
		originalReplace(null, "", href);
		render();
	}),
	back: vi.fn(),
	forward: vi.fn(),
	refresh: vi.fn(),
	prefetch: vi.fn(),
	experimental_gesturePush: vi.fn(),
	bfcacheId: "use-page",
};

function Probe() {
	router = useClientRouter();
	return createElement(UseRoutePage, { eventConfig: {} });
}

function render() {
	root.render(
		createElement(UseNavigationProvider, {
			routeMode,
			// biome-ignore lint/correctness/noChildrenProp: The provider requires children in its props type.
			children: createElement(Probe),
		}),
	);
}

beforeEach(() => {
	vi.clearAllMocks();
	vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
	routeMode = "query";
	container = document.createElement("div");
	document.body.append(container);
	root = createRoot(container);
});

afterEach(async () => {
	await act(async () => root.unmount());
	container.remove();
	vi.restoreAllMocks();
	vi.unstubAllGlobals();
});

test("opening a query route renders it without redirecting to a local path", async () => {
	originalReplace(null, "", "/use?id=app&route=%2Forders");
	const replaceState = vi.spyOn(window.history, "replaceState");
	await act(async () => render());
	expect(container.textContent).toBe("/orders");
	expect(window.location.pathname).toBe("/use");
	expect(native.replace).not.toHaveBeenCalled();
	expect(replaceState).not.toHaveBeenCalled();
});

test("a path deep link redirects through the exported shell before rendering", async () => {
	originalReplace(null, "", "/use/orders?id=app&filter=open");
	await act(async () => render());
	expect(native.replace).toHaveBeenCalledExactlyOnceWith(
		"/use?id=app&filter=open&route=%2Forders",
		{ scroll: true },
	);
	expect(container.textContent).toBe("/orders");
	expect(window.location.pathname).toBe("/use");
});

test.each([true, false])(
	"web canonicalization preserves fragment scrolling (scroll: %s)",
	async (scroll) => {
		routeMode = "path";
		originalReplace(
			{ flowLikeUseScroll: scroll },
			"",
			"/use?id=app&route=%2Forders#details",
		);
		vi.spyOn(window.history, "replaceState").mockImplementation(
			(data, title, url) => {
				originalReplace(data, title, url);
				render();
			},
		);
		const target = document.createElement("div");
		target.id = "details";
		target.scrollIntoView = vi.fn();
		document.body.append(target);
		try {
			await act(async () => render());
			expect(window.location.pathname).toBe("/use/orders");
			expect(container.textContent).toBe("/orders");
			expect(window.history.state.flowLikeUseScroll).toBe(scroll);
			expect(Boolean(vi.mocked(target.scrollIntoView).mock.calls.length)).toBe(
				scroll,
			);
		} finally {
			target.remove();
		}
	},
);

test("route changes and Event redirects update the mounted use page", async () => {
	originalReplace(null, "", "/use?id=app&route=%2Forders");
	await act(async () => render());
	await act(async () => router.push("/use/reports/daily?id=app"));
	expect(window.location.pathname).toBe("/use");
	expect(container.textContent).toBe("/reports/daily");
	await act(async () => router.replace("/use?id=app&eventId=chat"));
	expect(container.textContent).toBe("event:chat");
	expect(window.location.pathname).toBe("/use");
});
