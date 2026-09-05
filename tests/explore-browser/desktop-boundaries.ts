import type { GenericFetcher } from "../../packages/ui/components/pages/store/store-package-detail";

type NativeInvoke = <T>(
	command: string,
	args?: Record<string, unknown>,
) => Promise<T>;
let registry: GenericFetcher | undefined;
let native: NativeInvoke | undefined;

export function configureDesktopBoundaries(
	fetcher: GenericFetcher,
	invoke: NativeInvoke,
) {
	registry = fetcher;
	native = invoke;
}

export const fetcher: GenericFetcher = (...args) => {
	if (!registry) throw new Error("Desktop fixture API was not configured");
	return registry(...args);
};

export const invoke: NativeInvoke = (...args) => {
	if (!native) throw new Error("Desktop fixture native API was not configured");
	return native(...args);
};

export async function listen() {
	return () => {};
}
export async function emit() {}
