import type { IHomeWidget } from "../types";

export type HomeAppRendering =
	| "standard"
	| "compact"
	| "list"
	| "editorial"
	| "icons"
	| "carousel";

export const HOME_APP_RENDERINGS: [HomeAppRendering, string][] = [
	["standard", "Standard library cards"],
	["compact", "Compact cards"],
	["list", "List"],
	["editorial", "Editorial feature"],
	["icons", "App icons"],
	["carousel", "Card carousel"],
];

export const HOME_MODEL_RENDERINGS: ["standard" | "list", string][] = [
	["standard", "Standard model cards"],
	["list", "Compact list"],
];

export type HomePackageRendering = "standard" | "compact" | "featured";

export const HOME_PACKAGE_RENDERINGS: [HomePackageRendering, string][] = [
	["standard", "Standard Explore cards"],
	["compact", "Compact cards"],
	["featured", "Featured cards"],
];

export function homePackageRendering(
	config: Record<string, unknown>,
	legacyVariant?: string,
): HomePackageRendering {
	const requested = config.rendering ?? legacyVariant;
	if (requested === "list") return "compact";
	return (
		HOME_PACKAGE_RENDERINGS.find(([value]) => value === requested)?.[0] ??
		"standard"
	);
}

/** Older layouts stored content rendering in the widget's surface variant. */
export function homeAppRendering(
	config: Record<string, unknown>,
	legacyVariant?: string,
): HomeAppRendering {
	const requested = config.rendering ?? legacyVariant;
	if (requested === "spotlight") return "editorial";
	return (
		HOME_APP_RENDERINGS.find(([value]) => value === requested)?.[0] ??
		"standard"
	);
}

export function homeModelRendering(
	config: Record<string, unknown>,
	legacyVariant?: string,
): "standard" | "list" {
	return (config.rendering ?? legacyVariant) === "list" ? "list" : "standard";
}

export function homeLinksRendering(
	config: Record<string, unknown>,
	legacyVariant?: string,
): "grid" | "list" {
	return (config.rendering ?? legacyVariant) === "list" ? "list" : "grid";
}

export function textConfig(
	config: Record<string, unknown>,
	key: string,
	fallback = "",
): string {
	return typeof config[key] === "string" ? (config[key] as string) : fallback;
}

export function numberConfig(
	config: Record<string, unknown>,
	key: string,
	fallback = 6,
): number {
	const value = Number(config[key] ?? fallback);
	return Number.isFinite(value)
		? Math.min(50, Math.max(1, Math.round(value)))
		: fallback;
}

export function stringList(
	config: Record<string, unknown>,
	key: string,
): string[] {
	const value = config[key];
	return Array.isArray(value)
		? value.filter((item): item is string => typeof item === "string")
		: [];
}

export interface HomeContentProps {
	widget: IHomeWidget;
	editing?: boolean;
	onUpdate?: (config: Record<string, unknown>) => void;
}

export function safeHomeHref(value: string): string | undefined {
	const href = value.trim();
	if (
		Array.from(href).some(
			(char) =>
				char === "\\" || char.charCodeAt(0) < 32 || char.charCodeAt(0) === 127,
		)
	)
		return undefined;
	if (/^\/(?!\/)/.test(href)) return href;
	try {
		const url = new URL(href);
		return ["https:", "http:", "mailto:", "tel:"].includes(url.protocol)
			? url.href
			: undefined;
	} catch {
		return undefined;
	}
}

export interface HomeEmbedTarget {
	appId: string;
	routePath: string;
	eventId: string | null;
	queryParams: Record<string, string>;
}

const RESERVED_QUERY_KEYS = new Set([
	"id",
	"route",
	"eventId",
	"__proto__",
	"constructor",
	"prototype",
]);

/** Widget routing fields stay separate from query values supplied to an app. */
export function parseHomeEmbedTarget(
	config: Record<string, unknown>,
): HomeEmbedTarget {
	const mode = textConfig(config, "target", "landing");
	const rawRoute = mode === "route" ? textConfig(config, "route", "/") : "/";
	const queryStart = rawRoute.indexOf("?");
	const routePart = queryStart < 0 ? rawRoute : rawRoute.slice(0, queryStart);
	const inlineQuery = queryStart < 0 ? "" : rawRoute.slice(queryStart + 1);
	const routePath = `/${routePart.replace(/^\/+/, "")}`;
	const params = new URLSearchParams(inlineQuery);
	new URLSearchParams(textConfig(config, "query").replace(/^\?/, "")).forEach(
		(value, key) => params.set(key, value),
	);
	const queryParams: Record<string, string> = {};
	params.forEach((value, key) => {
		if (!RESERVED_QUERY_KEYS.has(key)) queryParams[key] = value;
	});
	return {
		appId: textConfig(config, "appId"),
		routePath,
		eventId: mode === "event" ? textConfig(config, "eventId") || null : null,
		queryParams,
	};
}

export function homeEmbedHref(target: HomeEmbedTarget): string {
	const params = new URLSearchParams({ id: target.appId });
	if (target.eventId) params.set("eventId", target.eventId);
	else params.set("route", target.routePath);
	for (const [key, value] of Object.entries(target.queryParams)) {
		if (!RESERVED_QUERY_KEYS.has(key)) params.set(key, value);
	}
	return `/use?${params.toString()}`;
}

export function mergeHomeEmbedNavigation(
	target: HomeEmbedTarget,
	next: {
		routePath?: string | null;
		eventId?: string | null;
		queryParams?: Record<string, string>;
	},
): HomeEmbedTarget {
	return {
		...target,
		routePath: next.routePath ?? target.routePath,
		eventId: next.eventId === undefined ? target.eventId : next.eventId,
		queryParams: next.queryParams
			? { ...next.queryParams }
			: target.queryParams,
	};
}
