import {
	TIMED_OUT,
	attempt,
	attemptAsync,
	finiteNumber,
	nonEmptyString,
	optionalBoolean,
	readGlobal,
	readProperty,
	withTimeout,
} from "./safe";
import type {
	DiagnosticsBrand,
	DiagnosticsBrowser,
	DiagnosticsEnvironment,
	DiagnosticsHardware,
} from "./types";

export const HIGH_ENTROPY_HINTS = [
	"platformVersion",
	"architecture",
	"bitness",
	"model",
	"fullVersionList",
] as const;

const CLIENT_HINTS_TIMEOUT_MS = 2000;
const GREASE_BRAND = /^not[^a-z]*a[^a-z]*brand$/i;

export type BrowserIdentity = Pick<
	DiagnosticsBrowser,
	| "name"
	| "version"
	| "os"
	| "osVersion"
	| "architecture"
	| "bitness"
	| "model"
	| "mobile"
	| "clientHints"
	| "brands"
>;

type Pattern = readonly [name: string, pattern: RegExp];

const BROWSER_PATTERNS: readonly Pattern[] = [
	["Microsoft Edge", /\bEdg(?:e|A|iOS)?\/([\d.]+)/],
	["Opera", /\b(?:OPR|OPiOS)\/([\d.]+)/],
	["Samsung Internet", /\bSamsungBrowser\/([\d.]+)/],
	["Firefox", /\b(?:Firefox|FxiOS)\/([\d.]+)/],
	["Chrome", /\b(?:Chrome|CriOS)\/([\d.]+)/],
	["Safari", /\bVersion\/([\d.]+).*\bSafari\//],
];

const OS_PATTERNS: readonly Pattern[] = [
	["Windows", /\bWindows NT ([\d.]+)/],
	["iOS", /\b(?:iPhone|iPad|iPod)\b.*?\bOS ([\d_]+)/],
	["Android", /\bAndroid ([\d.]+)/],
	["ChromeOS", /\bCrOS \S+ ([\d.]+)/],
	["macOS", /\bMac OS X ([\d_.]+)/],
	["Linux", /\bLinux\b/],
];

const NO_MATCH: {
	readonly name: string | null;
	readonly version: string | null;
} = { name: null, version: null };

function matchFirst(userAgent: string, patterns: readonly Pattern[]) {
	for (const [name, pattern] of patterns) {
		const match = pattern.exec(userAgent);
		if (match) return { name, version: match[1]?.replaceAll("_", ".") ?? null };
	}
	return NO_MATCH;
}

function cleanBrands(value: unknown) {
	if (!Array.isArray(value)) return [];
	return value.flatMap((entry) => {
		const brand = nonEmptyString(readProperty(entry, "brand"));
		const version = nonEmptyString(readProperty(entry, "version"));
		return brand && version ? [{ brand, version }] : [];
	});
}

const isRealBrand = ({ brand }: DiagnosticsBrand) => !GREASE_BRAND.test(brand);
const isBranded = ({ brand }: DiagnosticsBrand) => brand !== "Chromium";

export function primaryBrand(brands: readonly DiagnosticsBrand[]) {
	const real = brands.filter(isRealBrand);
	return real.find(isBranded) ?? real[0] ?? null;
}

function windowsRelease(platformVersion: string) {
	const major = Number.parseInt(platformVersion, 10);
	if (!Number.isFinite(major) || major <= 0) return null;
	return major >= 13 ? "11" : "10";
}

function clientHintsOsVersion(
	platform: string | null,
	platformVersion: string | null,
) {
	if (!platformVersion) return null;
	if (platform !== "Windows") return platformVersion;
	const release = windowsRelease(platformVersion);
	return release ? `${release} (${platformVersion})` : platformVersion;
}

function userAgentOsVersion(os: typeof NO_MATCH) {
	return os.name === "Windows" && os.version ? `NT ${os.version}` : os.version;
}

export function identityFromUserAgent(userAgent: string | null) {
	const ua = userAgent ?? "";
	const browser = matchFirst(ua, BROWSER_PATTERNS);
	const os = matchFirst(ua, OS_PATTERNS);
	const identity: BrowserIdentity = {
		name: browser.name,
		version: browser.version,
		os: os.name,
		osVersion: userAgentOsVersion(os),
		architecture: null,
		bitness: null,
		model: null,
		mobile: ua ? /\bMobi/.test(ua) : null,
		clientHints: false,
		brands: [],
	};
	return identity;
}

export function identityFromClientHints(uaData: unknown, hints: unknown) {
	const fullVersions = cleanBrands(readProperty(hints, "fullVersionList"));
	const brands = fullVersions.length
		? fullVersions
		: cleanBrands(readProperty(uaData, "brands"));
	const primary = primaryBrand(brands);
	const platform =
		nonEmptyString(readProperty(hints, "platform")) ??
		nonEmptyString(readProperty(uaData, "platform"));
	const identity: BrowserIdentity = {
		name: primary?.brand ?? null,
		version: primary?.version ?? null,
		os: platform,
		osVersion: clientHintsOsVersion(
			platform,
			nonEmptyString(readProperty(hints, "platformVersion")),
		),
		architecture: nonEmptyString(readProperty(hints, "architecture")),
		bitness: nonEmptyString(readProperty(hints, "bitness")),
		model: nonEmptyString(readProperty(hints, "model")),
		mobile: optionalBoolean(readProperty(uaData, "mobile")),
		clientHints: true,
		brands,
	};
	return identity;
}

async function readHighEntropyValues(uaData: unknown) {
	const getHighEntropyValues = readProperty<
		(hints: readonly string[]) => Promise<unknown>
	>(uaData, "getHighEntropyValues");
	if (typeof getHighEntropyValues !== "function") return null;
	const values = await withTimeout(
		getHighEntropyValues.call(uaData, [...HIGH_ENTROPY_HINTS]),
		CLIENT_HINTS_TIMEOUT_MS,
	);
	return values === TIMED_OUT ? null : values;
}

const firstKnown = (value: string | null, fallback: string | null) =>
	value ?? fallback;

function withFallback(identity: BrowserIdentity, fallback: BrowserIdentity) {
	return {
		...identity,
		name: firstKnown(identity.name, fallback.name),
		version: firstKnown(identity.version, fallback.version),
		os: firstKnown(identity.os, fallback.os),
		osVersion: firstKnown(identity.osVersion, fallback.osVersion),
	};
}

async function probeIdentity(navigator: unknown, userAgent: string | null) {
	const fromUserAgent = identityFromUserAgent(userAgent);
	const uaData = readProperty(navigator, "userAgentData");
	if (!uaData) return fromUserAgent;
	const hints = await attemptAsync(() => readHighEntropyValues(uaData), null);
	const fromHints = attempt(
		() => identityFromClientHints(uaData, hints),
		fromUserAgent,
	);
	return withFallback(fromHints, fromUserAgent);
}

const isString = (value: unknown): value is string => typeof value === "string";

function probeLanguages(navigator: unknown) {
	const languages = readProperty<unknown>(navigator, "languages");
	if (Array.isArray(languages) && languages.length > 0) {
		return languages.filter(isString);
	}
	const language = nonEmptyString(readProperty(navigator, "language"));
	return language ? [language] : [];
}

function size(width: unknown, height: unknown) {
	const w = finiteNumber(width);
	const h = finiteNumber(height);
	return w !== null && h !== null ? { width: w, height: h } : null;
}

function mediaMatches(query: string) {
	const matchMedia =
		readGlobal<(query: string) => MediaQueryList>("matchMedia");
	if (typeof matchMedia !== "function") return null;
	return attempt(
		() => optionalBoolean(matchMedia.call(globalThis, query).matches),
		null,
	);
}

function probeColorScheme() {
	if (mediaMatches("(prefers-color-scheme: dark)")) return "dark";
	if (mediaMatches("(prefers-color-scheme: light)")) return "light";
	return null;
}

export async function probeBrowser(): Promise<DiagnosticsBrowser> {
	const navigator = readGlobal<unknown>("navigator");
	const screen = readGlobal<unknown>("screen");
	const userAgent = nonEmptyString(readProperty(navigator, "userAgent"));
	const identity = await probeIdentity(navigator, userAgent);
	return {
		...identity,
		userAgent,
		languages: attempt(() => probeLanguages(navigator), []),
		timeZone: attempt(
			() => nonEmptyString(Intl.DateTimeFormat().resolvedOptions().timeZone),
			null,
		),
		online: optionalBoolean(readProperty(navigator, "onLine")),
		cookieEnabled: optionalBoolean(readProperty(navigator, "cookieEnabled")),
		viewport: size(readGlobal("innerWidth"), readGlobal("innerHeight")),
		screen: size(readProperty(screen, "width"), readProperty(screen, "height")),
		devicePixelRatio: finiteNumber(readGlobal("devicePixelRatio")),
		colorScheme: probeColorScheme(),
		reducedMotion: mediaMatches("(prefers-reduced-motion: reduce)"),
	};
}

export function probeHardware(): DiagnosticsHardware {
	const navigator = readGlobal<unknown>("navigator");
	return {
		logicalCores: finiteNumber(readProperty(navigator, "hardwareConcurrency")),
		deviceMemoryGb: finiteNumber(readProperty(navigator, "deviceMemory")),
		maxTouchPoints: finiteNumber(readProperty(navigator, "maxTouchPoints")),
	};
}

export function probeEnvironment(): DiagnosticsEnvironment {
	return {
		origin: nonEmptyString(readProperty(readGlobal("location"), "origin")),
		apiBaseUrl: null,
		secureContext: optionalBoolean(readGlobal("isSecureContext")),
		crossOriginIsolated: optionalBoolean(readGlobal("crossOriginIsolated")),
	};
}
