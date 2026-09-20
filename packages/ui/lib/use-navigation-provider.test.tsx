import { afterEach, describe, expect, mock, test } from "bun:test";
import {
	useClientHref,
	useClientRouter,
} from "@flow-like/flow-like-ui/lib/client-navigation";
import { QueryParamNavigationContext } from "@flow-like/flow-like-ui/lib/set-query-params";
import { AppRouterContext } from "next/dist/shared/lib/app-router-context.shared-runtime";
import { useContext } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { UseNavigationProvider } from "./use-navigation-provider";

const originalWindow = Object.getOwnPropertyDescriptor(globalThis, "window");
afterEach(() => {
	if (originalWindow)
		Object.defineProperty(globalThis, "window", originalWindow);
	else Reflect.deleteProperty(globalThis, "window");
});

function setup(path: string, origin = "https://app.test") {
	let current = new URL(path, origin);
	const location = {
		get href() {
			return current.href;
		},
		get pathname() {
			return current.pathname;
		},
		get origin() {
			return current.origin;
		},
		get hash() {
			return current.hash;
		},
		assign: mock(() => {}),
		replace: mock(() => {}),
	};
	const setUrl = (_data: unknown, _title: string, href: string) => {
		current = new URL(href, current);
	};
	const history = { pushState: mock(setUrl), replaceState: mock(setUrl) };
	Object.defineProperty(globalThis, "window", {
		configurable: true,
		value: { location, history, scrollTo: mock(() => {}) },
	});
	const native = {
		push: mock(() => {}),
		replace: mock(() => {}),
		back: mock(() => {}),
		forward: mock(() => {}),
		refresh: mock(() => {}),
		prefetch: mock(() => {}),
		experimental_gesturePush: mock(() => {}),
		bfcacheId: "use-page",
	};
	let result:
		| {
				router: ReturnType<typeof useClientRouter>;
				href: ReturnType<typeof useClientHref>;
				query: NonNullable<
					React.ContextType<typeof QueryParamNavigationContext>
				>;
		  }
		| undefined;
	function Probe() {
		const query = useContext(QueryParamNavigationContext);
		if (!query) throw new Error("Query navigation was not provided.");
		result = { router: useClientRouter(), href: useClientHref(), query };
		return null;
	}
	renderToStaticMarkup(
		<AppRouterContext.Provider value={native}>
			<UseNavigationProvider>
				<Probe />
			</UseNavigationProvider>
		</AppRouterContext.Provider>,
	);
	if (!result) throw new Error("The navigation probe did not render.");
	return { ...result, native, location, history, current: () => current };
}

describe("static use navigation", () => {
	test("routes between app paths through history without fetching Next payloads", () => {
		const { router, current, history, native } = setup("/use/orders?id=app");
		router.push("/use?id=app&route=%2Freports%2Fdaily&filter=open#totals");
		expect(current().pathname).toBe("/use/reports/daily");
		expect(current().searchParams.get("filter")).toBe("open");
		expect(current().hash).toBe("#totals");
		expect(history.pushState).toHaveBeenCalledTimes(1);
		router.replace("/use?id=app&eventId=chat");
		expect(current().pathname).toBe("/use");
		expect(current().searchParams.get("eventId")).toBe("chat");
		expect(history.replaceState).toHaveBeenCalledTimes(1);
		expect(native.push).not.toHaveBeenCalled();
		expect(native.replace).not.toHaveBeenCalled();
	});

	test("keeps query-only updates on the current path and preserves its fragment", () => {
		const { query, current } = setup("/use/orders?id=app#details");
		query("?id=app&sessionId=one&appQuery=tag%3Da%26tag%3Db", true);
		expect(current().pathname).toBe("/use/orders");
		expect(current().hash).toBe("#details");
		expect(current().searchParams.get("sessionId")).toBe("one");
		expect(current().searchParams.get("appQuery")).toBe("tag=a&tag=b");
	});

	test("enters through the exported shell while formatting clean new-tab links", () => {
		const { router, href, location, native } = setup("/library");
		const target = "/use?id=app&route=%2Forders";
		expect(href(target)).toBe("/use/orders?id=app");
		router.push(target);
		expect(native.push).toHaveBeenCalledWith(target, undefined);
		expect(location.assign).not.toHaveBeenCalled();
		router.replace(target);
		expect(native.replace).toHaveBeenCalledWith(target, undefined);
		expect(location.replace).not.toHaveBeenCalled();
	});

	test("leaves other routes and other origins to the existing router", () => {
		const { router, href, native } = setup("/use/orders?id=app");
		for (const target of [
			"/users",
			"/settings",
			"https://other.test/use?route=/a",
		]) {
			expect(href(target)).toBe(target);
			router.push(target, { scroll: false });
			expect(native.push).toHaveBeenLastCalledWith(target, { scroll: false });
		}
	});

	test("Tauri URLs use history while unrelated opaque origins stay external", () => {
		const { router, href, current, native } = setup(
			"/use/orders?id=app",
			"tauri://localhost",
		);
		router.push("/use?id=app&route=%2Freports");
		expect(current().href).toBe("tauri://localhost/use/reports?id=app");
		for (const target of [
			"other://localhost/use?route=/a",
			"tauri://other/use?route=/a",
			"file:///use?route=/a",
		]) {
			expect(href(target)).toBe(target);
			router.push(target);
			expect(native.push).toHaveBeenLastCalledWith(target, undefined);
		}
	});

	test("entering a packaged desktop app preserves its mounted providers", () => {
		for (const origin of [
			"tauri://localhost",
			"http://tauri.localhost",
			"https://tauri.localhost",
		]) {
			const { router, native, location } = setup("/library", origin);
			router.push("/use?id=app&route=%2F");
			expect(native.push).toHaveBeenCalledWith(
				"/use?id=app&route=%2F",
				undefined,
			);
			expect(location.assign).not.toHaveBeenCalled();
		}
	});

	test("a malformed deep link does not crash link rendering", () => {
		const { href } = setup("/use/orders?id=app");
		expect(href("/use/%invalid?id=app")).toBe("/use/%invalid?id=app");
		const entering = setup("/library");
		expect(() => entering.router.push("/use/%invalid?id=app")).not.toThrow();
		expect(entering.location.assign).toHaveBeenCalledWith(
			"/use/%invalid?id=app",
		);
		expect(entering.native.push).not.toHaveBeenCalled();
	});

	test("imperative fragment navigation scrolls unless explicitly disabled", () => {
		const original = Object.getOwnPropertyDescriptor(globalThis, "document");
		const scrollIntoView = mock(() => {});
		Object.defineProperty(globalThis, "document", {
			configurable: true,
			value: {
				getElementById: () => ({ scrollIntoView }),
				getElementsByName: () => [],
			},
		});
		try {
			const { router, current } = setup("/use/orders?id=app#one");
			router.push("#two");
			expect(current().hash).toBe("#two");
			expect(scrollIntoView).toHaveBeenCalledTimes(1);
			router.replace("#three", { scroll: false });
			expect(current().hash).toBe("#three");
			expect(scrollIntoView).toHaveBeenCalledTimes(1);
		} finally {
			if (original) Object.defineProperty(globalThis, "document", original);
			else Reflect.deleteProperty(globalThis, "document");
		}
	});
});
