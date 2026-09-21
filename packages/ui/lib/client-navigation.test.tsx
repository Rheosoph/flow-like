import { describe, expect, mock, test } from "bun:test";
import { AppRouterContext } from "next/dist/shared/lib/app-router-context.shared-runtime";
import { useRouter } from "next/navigation";
import { renderToStaticMarkup } from "react-dom/server";
import {
	type ClientNavigation,
	ClientNavigationContext,
	useClientHref,
	useClientRouter,
} from "./client-navigation";

function renderNavigation(navigation?: ClientNavigation) {
	const next: ReturnType<typeof useClientRouter> = {
		push: mock(() => {}),
		replace: mock(() => {}),
		back: mock(() => {}),
		forward: mock(() => {}),
		refresh: mock(() => {}),
		prefetch: mock(() => {}),
		experimental_gesturePush: mock(() => {}),
		bfcacheId: "segment-one",
	};
	let result:
		| {
				router: ReturnType<typeof useClientRouter>;
				native: ReturnType<typeof useRouter>;
				href: ReturnType<typeof useClientHref>;
		  }
		| undefined;
	function Probe() {
		result = {
			router: useClientRouter(),
			native: useRouter(),
			href: useClientHref(),
		};
		return null;
	}
	renderToStaticMarkup(
		<AppRouterContext.Provider value={next}>
			<ClientNavigationContext.Provider value={navigation}>
				<Probe />
			</ClientNavigationContext.Provider>
		</AppRouterContext.Provider>,
	);
	if (!result) throw new Error("The navigation probe did not render.");
	return { ...result, next };
}

describe("host navigation opt-in", () => {
	test("returns the original Next router and href outside an adapted host", () => {
		const { router, next, native, href } = renderNavigation();
		expect(router).toEqual(native);
		expect(href("/use?id=app&route=%2Forders")).toBe(
			"/use?id=app&route=%2Forders",
		);
		router.push("/desktop");
		expect(next.push).toHaveBeenCalledWith("/desktop");
	});

	test("delegates only push and replace, preserving navigation options", () => {
		const navigate = mock(() => {});
		const { router, next, native } = renderNavigation({
			navigate,
			href: (href) => href,
		});
		router.push("/use?id=app&route=%2Forders", { scroll: false });
		router.replace("?sessionId=chat");
		expect(navigate.mock.calls).toEqual([
			["/use?id=app&route=%2Forders", false, { scroll: false }],
			["?sessionId=chat", true, undefined],
		]);
		expect(next.push).not.toHaveBeenCalled();
		expect(next.replace).not.toHaveBeenCalled();
		for (const key of [
			"back",
			"forward",
			"refresh",
			"prefetch",
			"experimental_gesturePush",
			"bfcacheId",
		] as const) {
			expect(router[key]).toBe(native[key]);
		}
	});

	test("lets the host resolve hrefs without changing shared URL semantics", () => {
		const resolveHref = mock((href: string) => `host:${href}`);
		const { href } = renderNavigation({
			navigate: mock(() => {}),
			href: resolveHref,
		});
		expect(href).toBe(resolveHref);
		expect(href("/use?id=app&route=%2Forders")).toBe(
			"host:/use?id=app&route=%2Forders",
		);
		expect(resolveHref).toHaveBeenCalledWith("/use?id=app&route=%2Forders");
	});
});
