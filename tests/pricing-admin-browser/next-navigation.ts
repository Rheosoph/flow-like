import { useMemo, useSyncExternalStore } from "react";
const subscribe = (callback: () => void) => {
	window.addEventListener("popstate", callback);
	return () => window.removeEventListener("popstate", callback);
};
const snapshot = () => window.location.pathname + window.location.search;
const router = {
	push: (url: string) => {
		history.pushState({}, "", url);
		window.dispatchEvent(new PopStateEvent("popstate"));
	},
	replace: (url: string) => {
		history.replaceState({}, "", url);
		window.dispatchEvent(new PopStateEvent("popstate"));
	},
	refresh() {},
	back() {
		history.back();
	},
	prefetch() {},
};
export const useRouter = () => router;
export function useSearchParams() {
	const url = useSyncExternalStore(subscribe, snapshot);
	return useMemo(() => new URLSearchParams(url.split("?")[1] ?? ""), [url]);
}
export function usePathname() {
	return useSyncExternalStore(subscribe, snapshot).split("?")[0];
}
export const useParams = () => ({});
export const redirect = (url: string) => router.replace(url);
export const notFound = () => null;
