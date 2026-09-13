import {
	type NotificationIconSource,
	notificationIconCandidates,
	notificationIconSource,
} from "@flow-like/flow-like-ui/lib/notification-icon";
import dynamicIconImports from "lucide-react/dynamicIconImports";
import {
	type NativeIconFetch,
	rasterizeNativeAppIcon,
	readNativeAppIcon,
} from "./native-app-icons";

export interface NativeNotificationIconSource {
	id: string;
	icon?: string | null;
	appIcon?: string | null;
}

export interface NativeNotificationIcon {
	png?: string;
	text?: string;
	template?: boolean;
}

function assertActive(signal: AbortSignal) {
	if (signal.aborted)
		throw new DOMException("Notification icon cancelled", "AbortError");
}

async function abortable<T>(
	operation: Promise<T>,
	signal: AbortSignal,
): Promise<T> {
	let cancel = () => {};
	const aborted = new Promise<never>((_, reject) => {
		cancel = () =>
			reject(new DOMException("Notification icon cancelled", "AbortError"));
		signal.addEventListener("abort", cancel, { once: true });
		if (signal.aborted) cancel();
	});
	try {
		return await Promise.race([operation, aborted]);
	} finally {
		signal.removeEventListener("abort", cancel);
	}
}

function boundedPNG(value: string) {
	if (value.length > 43_692) return false;
	try {
		const bytes = atob(value);
		return bytes.length <= 32_768 && bytes.startsWith("\x89PNG\r\n\x1a\n");
	} catch {
		return false;
	}
}

async function lucideImage(name: string): Promise<Blob> {
	const importer = dynamicIconImports[name as keyof typeof dynamicIconImports];
	if (!importer) throw new Error("Unknown notification icon");
	const { __iconNode } = await importer();
	const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg");
	for (const [key, value] of Object.entries({
		xmlns: "http://www.w3.org/2000/svg",
		width: "96",
		height: "96",
		viewBox: "0 0 24 24",
		fill: "none",
		stroke: "black",
		"stroke-width": "2",
		"stroke-linecap": "round",
		"stroke-linejoin": "round",
	}))
		svg.setAttribute(key, value);
	for (const [tag, attributes] of __iconNode) {
		const child = document.createElementNS("http://www.w3.org/2000/svg", tag);
		for (const [key, value] of Object.entries(attributes))
			if (key !== "key") child.setAttribute(key, String(value));
		svg.append(child);
	}
	return new Blob([new XMLSerializer().serializeToString(svg)], {
		type: "image/svg+xml",
	});
}

/** Publish portable display bytes only. URLs and image credentials stay outside the native cache. */
export function createNativeNotificationIconResolver(
	options: {
		fetch?: NativeIconFetch;
		rasterize?: (blob: Blob, signal: AbortSignal) => Promise<string>;
		lucide?: (name: string) => Promise<Blob>;
		fallback?: boolean;
	} = {},
) {
	const fetchIcon = options.fetch ?? globalThis.fetch;
	const rasterize = options.rasterize ?? rasterizeNativeAppIcon;
	const renderLucide = options.lucide ?? lucideImage;
	let current: AbortController | undefined;
	let currentScope: string | undefined;
	let disposed = false;
	const cache = new Map<
		string,
		{ icon: NativeNotificationIcon; expires: number }
	>();
	async function acquire(
		candidate: NotificationIconSource,
		signal: AbortSignal,
	): Promise<NativeNotificationIcon> {
		const acquisition = new AbortController();
		const cancel = () => acquisition.abort();
		signal.addEventListener("abort", cancel, { once: true });
		if (signal.aborted) cancel();
		const timeout = setTimeout(cancel, 10_000);
		try {
			const blob = await abortable(
				candidate.kind === "lucide"
					? renderLucide(candidate.value)
					: readNativeAppIcon(candidate.value, fetchIcon, acquisition.signal),
				acquisition.signal,
			);
			assertActive(acquisition.signal);
			const png = await abortable(
				rasterize(blob, acquisition.signal),
				acquisition.signal,
			);
			assertActive(acquisition.signal);
			assertActive(signal);
			if (!boundedPNG(png))
				throw new Error("Notification icon exceeds native image limits");
			return { png, template: candidate.kind === "lucide" };
		} finally {
			clearTimeout(timeout);
			acquisition.abort();
			signal.removeEventListener("abort", cancel);
		}
	}
	return {
		async resolve(
			scope: string,
			sources: NativeNotificationIconSource[],
		): Promise<Record<string, NativeNotificationIcon>> {
			current?.abort();
			current = new AbortController();
			const signal = current.signal;
			if (disposed) current.abort();
			assertActive(signal);
			if (currentScope !== scope) {
				cache.clear();
				currentScope = scope;
			}
			const entries = sources.slice(0, 8);
			const result: Record<string, NativeNotificationIcon> =
				Object.create(null);
			const pending = new Map<string, Promise<NativeNotificationIcon>>();
			let index = 0;
			await Promise.all(
				Array.from({ length: Math.min(4, entries.length) }, async () => {
					while (index < entries.length) {
						const source = entries[index++];
						const explicit = notificationIconSource(source.icon);
						const candidates =
							options.fallback === false
								? explicit
									? [explicit]
									: []
								: notificationIconCandidates(source.icon, source.appIcon);
						for (const candidate of candidates) {
							assertActive(signal);
							if (candidate.kind === "emoji") {
								result[source.id] = { text: candidate.value };
								break;
							}
							const key = `${candidate.kind}:${candidate.value}`;
							const cached = cache.get(key);
							if (cached && cached.expires > Date.now()) {
								result[source.id] = cached.icon;
								break;
							}
							try {
								let request = pending.get(key);
								if (!request) {
									request = acquire(candidate, signal);
									pending.set(key, request);
								}
								const icon = await request;
								assertActive(signal);
								// Large data URLs must not remain in the session cache.
								if (key.length <= 4_096)
									cache.set(key, { icon, expires: Date.now() + 15 * 60_000 });
								while (cache.size > 32)
									cache.delete(cache.keys().next().value as string);
								result[source.id] = icon;
								break;
							} catch {
								assertActive(signal);
							}
						}
					}
				}),
			);
			assertActive(signal);
			return result;
		},
		dispose() {
			disposed = true;
			current?.abort();
			cache.clear();
		},
	};
}
