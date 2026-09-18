import { SOURCE_RESOURCES, type useTranslation } from "@flow-like/locales";
import {
	type MicroWidgetCapability,
	WIDGET_POLICY_REGISTRY_PREFIX,
	WIDGET_POLICY_SOURCE_ANY_REGISTRY,
	WIDGET_POLICY_SOURCE_HUB,
	WIDGET_POLICY_SOURCE_LOCAL,
	type WidgetCspKey,
	type WidgetNetworkSource,
	type WidgetSourceLevel,
	isRegistryPolicySource,
	isWidgetSourceAboutKey,
} from "./micro-widget-policy";

/**
 * Every sentence the consent UI says about a network source (§14.3.5, with
 * the calmer card copy of §14.9). Keys stay literal so `i18n:extract` sees
 * them; About keys come from the backend's catalog and are preserved by
 * `i18next.config.ts`. Params are the closed set `{ provider, domain, host }`
 * and server text is never rendered as prose.
 */

export type Translate = ReturnType<typeof useTranslation>["t"];

/** A type alias, not an interface, so it satisfies i18next's option dictionary. */
export type WidgetSourceCopyParams = {
	provider: string;
	domain: string;
	host: string;
};

export type WidgetSourceCopy = (
	t: Translate,
	params: WidgetSourceCopyParams,
) => string;

/** Kind of a source the backend could not classify (no `network` in the descriptor). */
export const WIDGET_SOURCE_KIND_UNCLASSIFIED = "unclassified";

export function widgetSourceCopyParams(
	source: Pick<WidgetNetworkSource, "provider" | "emphasis" | "host">,
): WidgetSourceCopyParams {
	return {
		provider: source.provider ?? source.emphasis,
		domain: source.emphasis,
		host: source.host,
	};
}

export const WIDGET_SOURCE_LEVEL_LABELS: Readonly<
	Record<WidgetSourceLevel, (t: Translate) => string>
> = {
	known: (t) => t("widgetSourceLevelKnown", "Identified service"),
	external: (t) => t("widgetSourceLevelExternal", "External site"),
	shared: (t) => t("widgetSourceLevelShared", "Hosts user content"),
	broad: (t) => t("widgetSourceLevelBroad", "Anyone can receive"),
};

/**
 * Risk sentences keyed by `kind:level`, or by `kind` when the level does not
 * change the wording. Details show these in full.
 */
export const WIDGET_SOURCE_RISK: Readonly<Record<string, WidgetSourceCopy>> = {
	"service:known": (t, params) =>
		t("widgetSourceRiskService", "Data goes to {{provider}}.", params),
	"service:external": (t, params) =>
		t(
			"widgetSourceRiskServiceReceives",
			"Data goes to {{provider}}. Anyone with a {{provider}} account could receive what the widget sends there.",
			params,
		),
	"service:shared": (t, params) =>
		t(
			"widgetSourceRiskServiceUserContent",
			"{{provider}} serves content its users upload, so what the widget loads there can come from anyone.",
			params,
		),
	exact: (t, params) =>
		t(
			"widgetSourceRiskExact",
			"Flow-Like does not know who runs this address. Whoever does receives what the widget sends.",
			params,
		),
	subdomains: (t, params) =>
		t(
			"widgetSourceRiskSubdomains",
			"Any address under {{domain}}. Flow-Like does not know who runs them, and addresses there can change hands.",
			params,
		),
	"tenant-host": (t, params) =>
		t(
			"widgetSourceRiskTenantHost",
			"One customer's space on {{provider}}. Flow-Like cannot tell who owns {{domain}}, and the name can pass to a new owner.",
			params,
		),
	"tenant-subdomains": (t, params) =>
		t(
			"widgetSourceRiskTenantSubdomains",
			"Addresses in one customer's space on {{provider}}. Flow-Like cannot tell who owns {{domain}}.",
			params,
		),
	"shared-host:shared": (t, params) =>
		t(
			"widgetSourceRiskSharedHostShared",
			"Many people publish files at {{host}}, so what the widget loads from it can come from anyone.",
			params,
		),
	"shared-host:broad": (t, params) =>
		t(
			"widgetSourceRiskSharedHostBroad",
			"Anyone with an account can store files at {{host}}, so anyone could receive what the widget sends there.",
			params,
		),
	"shared-wildcard:shared": (t, params) =>
		t(
			"widgetSourceRiskSharedWildcardShared",
			"Matches every site under {{domain}}. Anyone can publish there, so what the widget loads can come from anyone.",
			params,
		),
	"shared-wildcard:broad": (t, params) =>
		t(
			"widgetSourceRiskSharedWildcardBroad",
			"Matches every customer under {{domain}}. Anyone who signs up there could receive what the widget sends.",
			params,
		),
	"platform-host": (t, params) =>
		t(
			"widgetSourceRiskPlatformHost",
			"{{provider}} hosts many customers here. Flow-Like cannot tell whether {{host}} belongs to one customer or serves all of them.",
			params,
		),
	"platform-storage": (t, params) =>
		t(
			"widgetSourceRiskPlatformStorage",
			"Only this app's files in its Flow-Like storage.",
			params,
		),
	[WIDGET_SOURCE_KIND_UNCLASSIFIED]: (t, params) =>
		t(
			"widgetSourceRiskUnclassified",
			"Flow-Like could not check who runs this address, so it is shown at the highest risk.",
			params,
		),
};

/** Kinds a newer backend adds fall back to their level. */
export const WIDGET_SOURCE_RISK_FALLBACK: Readonly<
	Record<WidgetSourceLevel, WidgetSourceCopy>
> = {
	known: (t, params) =>
		t(
			"widgetSourceRiskFallbackKnown",
			"A service Flow-Like recognises.",
			params,
		),
	external: (t, params) =>
		t(
			"widgetSourceRiskFallbackExternal",
			"A recipient Flow-Like cannot verify.",
			params,
		),
	shared: (t, params) =>
		t(
			"widgetSourceRiskFallbackShared",
			"Shared hosting where content can come from anyone.",
			params,
		),
	broad: (t, params) =>
		t(
			"widgetSourceRiskFallbackBroad",
			"Shared hosting where anyone could receive what the widget sends.",
			params,
		),
};

/** The table entry for a classification, or null when only the level fallback applies. */
export function widgetSourceRiskEntry(
	kind: string,
	level: WidgetSourceLevel,
): string | null {
	const exact = `${kind}:${level}`;
	if (Object.hasOwn(WIDGET_SOURCE_RISK, exact)) return exact;
	return Object.hasOwn(WIDGET_SOURCE_RISK, kind) ? kind : null;
}

type RiskSource = Pick<
	WidgetNetworkSource,
	"kind" | "level" | "provider" | "emphasis" | "host"
>;

/** Full Risk sentence, as Details and store cards show it. */
export function widgetSourceRisk(source: RiskSource, t: Translate): string {
	const entry = widgetSourceRiskEntry(source.kind, source.level);
	const copy = entry
		? WIDGET_SOURCE_RISK[entry]
		: WIDGET_SOURCE_RISK_FALLBACK[source.level];
	return copy(t, widgetSourceCopyParams(source));
}

/**
 * One calm sentence for a consent card (§14.9): services and user-content
 * hosts say where data goes; the account-holder and "can come from anyone"
 * sentences stay in Details. `broad` always gets its full Risk line.
 */
export function widgetSourceCardRisk(source: RiskSource, t: Translate): string {
	if (
		source.level === "broad" ||
		source.kind === WIDGET_SOURCE_KIND_UNCLASSIFIED
	)
		return widgetSourceRisk(source, t);
	const params = widgetSourceCopyParams(source);
	if (source.level === "shared")
		return t(
			"widgetSourceRiskUserContent",
			"{{provider}} hosts content its users upload.",
			params,
		);
	if (source.kind === "service")
		return t("widgetSourceRiskService", "Data goes to {{provider}}.", params);
	return widgetSourceRisk(source, t);
}

/** About line of the source's provider; omitted for keys this build does not know. */
export function widgetSourceAbout(
	source: Pick<
		WidgetNetworkSource,
		"aboutKey" | "provider" | "emphasis" | "host"
	>,
	t: Translate,
): string | null {
	const key = source.aboutKey;
	if (
		!isWidgetSourceAboutKey(key) ||
		!Object.hasOwn(SOURCE_RESOURCES.common, key)
	) {
		return null;
	}
	return t(key, { provider: widgetSourceCopyParams(source).provider });
}

export const WIDGET_DIRECTIVE_LABELS: Readonly<
	Record<WidgetCspKey, (t: Translate) => string>
> = {
	connectSrc: (t) => t("widgetNetworkConnectSrc", "Send and receive data"),
	imgSrc: (t) => t("widgetNetworkImgSrc", "Load images"),
	fontSrc: (t) => t("widgetNetworkFontSrc", "Load fonts"),
	mediaSrc: (t) => t("widgetNetworkMediaSrc", "Stream audio and video"),
	styleSrc: (t) => t("widgetNetworkStyleSrc", "Load stylesheets"),
};

export interface WidgetCapabilityCopy {
	label: (t: Translate) => string;
	description: (t: Translate) => string;
}

export const WIDGET_CAPABILITY_COPY: Readonly<
	Record<MicroWidgetCapability, WidgetCapabilityCopy>
> = {
	microphone: {
		label: (t) => t("widgetCapabilityMicrophone", "Microphone"),
		description: (t) =>
			t(
				"widgetCapabilityMicrophoneDescription",
				"Flow-Like records through your microphone whenever the widget asks (up to 60 seconds and 8 MB per request) and hands the audio to the widget. A stop button is shown while recording.",
			),
	},
	downloads: {
		label: (t) => t("widgetCapabilityDownloads", "File downloads"),
		description: (t) =>
			t(
				"widgetCapabilityDownloadsDescription",
				"The widget may save files to your device through the browser's download flow. A download can be fetched from any site, so the widget can also use it to send data to any site.",
			),
	},
	media: {
		label: (t) => t("widgetCapabilityMedia", "Audio playback"),
		description: (t) =>
			t(
				"widgetCapabilityMediaDescription",
				"The widget may play media from its bundle and audio streams that this app's flows approved.",
			),
	},
	workers: {
		label: (t) => t("widgetCapabilityWorkers", "Background workers"),
		description: (t) =>
			t(
				"widgetCapabilityWorkersDescription",
				"The widget may run Web Workers from its bundle. Workers load only bundle files and blob: data, and reach only the sites approved for this widget.",
			),
	},
	wasm: {
		label: (t) => t("widgetCapabilityWasm", "WebAssembly"),
		description: (t) =>
			t(
				"widgetCapabilityWasmDescription",
				"The widget may compile and run WebAssembly modules shipped in its bundle.",
			),
	},
};

export type WidgetBannerLead =
	| { kind: "capabilities" }
	| { kind: "unclassified"; domain: string }
	| { kind: "broad" | "shared"; provider: string }
	| { kind: "external"; domain: string; others: number }
	| { kind: "known"; provider: string; others: number };

/** Sentence 1 of the aggregated banner (§14.5.2). */
export function widgetBannerLead(lead: WidgetBannerLead, t: Translate): string {
	switch (lead.kind) {
		case "capabilities":
			return t(
				"widgetCapabilityBannerLead",
				"This widget asks for the browser features below.",
			);
		case "unclassified":
			return t(
				"widgetNetworkBannerUnclassified",
				"Flow-Like could not check who receives what this widget sends to {{domain}}.",
				{ domain: lead.domain },
			);
		case "broad":
			return t(
				"widgetNetworkBannerBroad",
				"Anyone who signs up with {{provider}} could receive what this widget sends.",
				{ provider: lead.provider },
			);
		case "shared":
			return t(
				"widgetNetworkBannerShared",
				"Anyone can publish files on {{provider}}, so what this widget loads from there can come from anyone.",
				{ provider: lead.provider },
			);
		case "external":
			return lead.others > 0
				? t(
						"widgetNetworkBannerExternalMore",
						"Flow-Like cannot verify who receives what this widget sends to {{domain}} and {{count}} other sites.",
						{ domain: lead.domain, count: lead.others },
					)
				: t(
						"widgetNetworkBannerExternal",
						"Flow-Like cannot verify who receives what this widget sends to {{domain}}.",
						{ domain: lead.domain },
					);
		case "known":
			return lead.others > 0
				? t(
						"widgetNetworkBannerKnownMore",
						"This widget sends data to {{provider}} and {{count}} other services.",
						{ provider: lead.provider, count: lead.others },
					)
				: t(
						"widgetNetworkBannerKnown",
						"This widget sends data to {{provider}}.",
						{
							provider: lead.provider,
						},
					);
	}
}

export function widgetBannerSuffix(t: Translate): string {
	return t(
		"widgetNetworkBannerSuffix",
		"Anything the widget can see, including values this app gives it, can leave your device.",
	);
}

export type WidgetConsentTitle =
	| "runtime"
	| "broad"
	| "expanded"
	| "network"
	| "capabilities";

export function widgetConsentTitle(
	title: WidgetConsentTitle,
	t: Translate,
): string {
	switch (title) {
		case "runtime":
			return t(
				"widgetRuntimeDialogTitle",
				"Widget wants to load from new sites",
			);
		case "broad":
			return t(
				"widgetNetworkBroadDialogTitle",
				"Widget requests broad network access",
			);
		case "expanded":
			return t("widgetConsentExpandedTitle", "Widget asks for more access");
		case "network":
			return t("widgetNetworkDialogTitle", "Widget requests network access");
		case "capabilities":
			return t(
				"widgetCapabilityDialogTitle",
				"Widget requests browser capabilities",
			);
	}
}

/**
 * Why a runtime address was not offered (§14.5.4): host extraction issues and
 * server rejections share one reason per code. Format codes (`not-absolute`,
 * `not-a-url`, `scheme-not-allowed`, `trailing-dot`, `too-long` and the
 * server's structural codes) and codes from newer backends read as a format
 * problem.
 */
export function widgetRuntimeIssueReason(code: string, t: Translate): string {
	if (code === "port") return t("widgetRuntimeIssuePort", "uses a port number");
	if (code === "userinfo")
		return t(
			"widgetRuntimeIssueCredentials",
			"contains a user name or password",
		);
	if (code === "ip-literal")
		return t("widgetRuntimeIssueIp", "is an IP address");
	if (code.startsWith("template-"))
		return t(
			"widgetRuntimeIssueTemplate",
			"has placeholders the widget may not expand",
		);
	if (
		code === "too-many-values" ||
		code === "too-many-sources" ||
		code === "too-large"
	) {
		return t(
			"widgetRuntimeIssueTooMany",
			"more than the widget may add at once",
		);
	}
	if (code === "reserved-host" || code === "reserved-name")
		return t(
			"widgetRuntimeIssueReserved",
			"points at Flow-Like itself or a local-only name",
		);
	if (code === "platform-storage")
		return t(
			"widgetRuntimeIssuePlatformStorage",
			"is outside this app's files in Flow-Like storage",
		);
	if (code === "wildcard")
		return t("widgetRuntimeIssueWholeDomain", "would cover a whole domain");
	if (code === "unknown-slot")
		return t(
			"widgetRuntimeIssueNotDeclared",
			"came from an input the widget did not declare",
		);
	if (code === "engine")
		return t("widgetRuntimeIssueEngine", "not supported in this browser");
	return t("widgetRuntimeIssueFormat", "not a valid https or wss address");
}

/** Where the descriptor came from, for the dialog's source line. */
export function widgetPolicySourceLabel(source: string, t: Translate): string {
	if (source === WIDGET_POLICY_SOURCE_LOCAL)
		return t("widgetPolicySourceLocal", "Local package");
	if (source === WIDGET_POLICY_SOURCE_HUB)
		return t("widgetPolicySourceHub", "This server's package registry");
	if (source === WIDGET_POLICY_SOURCE_ANY_REGISTRY)
		return t("widgetPolicySourceAnyRegistry", "Package registry");
	if (isRegistryPolicySource(source))
		return t("widgetPolicySourceRegistry", "Registry {{host}}", {
			host: source.slice(WIDGET_POLICY_REGISTRY_PREFIX.length),
		});
	return source;
}
