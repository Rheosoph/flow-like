/**
 * No-op `next/navigation` shim for Astro islands.
 *
 * Many flow-like-ui leaf components import the Next App Router hooks. In a
 * non-Next island `useRouter()` throws ("expected app router to be mounted"),
 * which blocks the FlowPilot bubble and interactive a2ui. This site has no Next
 * runtime, so we alias the whole module (astro.config `vite.resolve.alias`) to
 * inert implementations — navigation actions become no-ops, which is correct
 * for read-only marketing demos.
 */

export function useRouter() {
	return {
		push: () => {},
		replace: () => {},
		prefetch: () => Promise.resolve(),
		back: () => {},
		forward: () => {},
		refresh: () => {},
	};
}

export function usePathname() {
	return "/";
}

const EMPTY_SEARCH_PARAMS =
	typeof URLSearchParams !== "undefined" ? new URLSearchParams() : undefined;

export function useSearchParams() {
	return EMPTY_SEARCH_PARAMS;
}

export function useParams() {
	return {};
}

export function useSelectedLayoutSegment() {
	return null;
}

export function useSelectedLayoutSegments() {
	return [];
}

export function redirect() {}
export function permanentRedirect() {}
export function notFound() {}

export const RedirectType = { push: "push", replace: "replace" } as const;
