import { useSyncExternalStore } from "react";

const subscribe = (callback: () => void) => {
	window.addEventListener("popstate", callback);
	return () => window.removeEventListener("popstate", callback);
};
const router = {
	push(url: string) {
		history.pushState({}, "", url);
		window.dispatchEvent(new PopStateEvent("popstate"));
	},
	replace(url: string) {
		history.replaceState({}, "", url);
		window.dispatchEvent(new PopStateEvent("popstate"));
	},
};
export const useRouter = () => router;
export const usePathname = () =>
	useSyncExternalStore(subscribe, () => location.pathname);
