import { useMemo, useSyncExternalStore } from "react";

const navigationEvent = "pricing-fixture:navigation";
let installed = false;
function installHistorySync() {
	if (installed || typeof window === "undefined") return;
	installed = true;
	for (const method of ["pushState", "replaceState"] as const) {
		const original = window.history[method].bind(window.history);
		window.history[method] = (...args: Parameters<History[typeof method]>) => {
			// Match Next's loop guard for internal history updates.
			if (args[0]?.__NA || args[0]?._N) {
				original(args[0], args[1], args[2]);
				return;
			}
			original({ ...args[0], __NA: true }, args[1], args[2]);
			window.dispatchEvent(new Event(navigationEvent));
		};
	}
}
const subscribe = (callback: () => void) => {
	installHistorySync();
	window.addEventListener("popstate", callback);
	window.addEventListener(navigationEvent, callback);
	return () => {
		window.removeEventListener("popstate", callback);
		window.removeEventListener(navigationEvent, callback);
	};
};
const snapshot = () => window.location.pathname + window.location.search;
const router = {
	push: (url: string) => {
		installHistorySync();
		history.pushState(null, "", url);
	},
	replace: (url: string) => {
		installHistorySync();
		history.replaceState(null, "", url);
	},
	refresh() {},
	back() {
		history.back();
	},
	prefetch() {},
};
export const useRouter = () => router;
export function useSearchParams() {
	const url = useSyncExternalStore(subscribe, snapshot, () => "/subscription");
	return useMemo(() => new URLSearchParams(url.split("?")[1] ?? ""), [url]);
}
export function usePathname() {
	return useSyncExternalStore(subscribe, snapshot, () => "/subscription").split(
		"?",
	)[0];
}
export const useParams = () => ({});
export const redirect = (url: string) => router.replace(url);
export const notFound = () => null;
