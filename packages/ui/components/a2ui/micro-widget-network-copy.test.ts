import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { SOURCE_RESOURCES } from "@flow-like/locales";
import {
	type Translate,
	WIDGET_CAPABILITY_COPY,
	WIDGET_DIRECTIVE_LABELS,
	WIDGET_SOURCE_LEVEL_LABELS,
	WIDGET_SOURCE_RISK,
	WIDGET_SOURCE_RISK_FALLBACK,
	type WidgetBannerLead,
	type WidgetConsentTitle,
	widgetBannerLead,
	widgetBannerSuffix,
	widgetConsentTitle,
	widgetPolicySourceLabel,
	widgetRuntimeIssueReason,
	widgetSourceAbout,
	widgetSourceCardRisk,
	widgetSourceRisk,
	widgetSourceRiskEntry,
} from "./micro-widget-network-copy";
import {
	WIDGET_SOURCE_ABOUT_KEYS,
	WIDGET_SOURCE_LEVELS,
	type WidgetSourceLevel,
} from "./micro-widget-policy";
import { RUNTIME_VALUE_ISSUE_CODES } from "./micro-widget-runtime-sources";

const en = SOURCE_RESOURCES.common as Record<string, string>;
const SCHEMA = join(import.meta.dir, "../../../wasm/schema");

function readJson<T>(path: string): T {
	return JSON.parse(readFileSync(join(SCHEMA, path), "utf8")) as T;
}

const catalog = readJson<{ providers: { id: string; aboutKey?: string }[] }>(
	"data/widget_source_catalog.json",
);
const classification = readJson<{
	classifications: { source: string; kind?: string; level?: string }[];
}>("tests/fixtures/widget_source_classification.json");

interface Call {
	key: string;
	fallback: string | undefined;
	options: Record<string, unknown>;
}

/** Records every lookup and renders the English resource like i18next would. */
function recorder() {
	const calls: Call[] = [];
	const t = ((
		key: string,
		second?: string | Record<string, unknown>,
		third?: Record<string, unknown>,
	) => {
		const fallback = typeof second === "string" ? second : undefined;
		const options = (typeof second === "object" ? second : third) ?? {};
		calls.push({ key, fallback, options });
		const count = options.count;
		const resolved =
			typeof count === "number"
				? (en[`${key}_${count === 1 ? "one" : "other"}`] ?? en[key])
				: en[key];
		return (resolved ?? fallback ?? key).replace(
			/{{(\w+)}}/g,
			(_, name: string) => String(options[name]),
		);
	}) as unknown as Translate;
	return { t, calls };
}

function resourceFor(call: Call): string | undefined {
	return typeof call.options.count === "number"
		? en[`${call.key}_other`]
		: en[call.key];
}

const SOURCE = {
	kind: "service",
	level: "known" as WidgetSourceLevel,
	provider: "Example Maps",
	emphasis: "example.com",
	host: "tiles.example.com",
};

/** Every literal table the dialog renders, driven once. */
function driveEveryTable(t: Translate): void {
	for (const level of WIDGET_SOURCE_LEVELS) {
		WIDGET_SOURCE_LEVEL_LABELS[level](t);
		WIDGET_SOURCE_RISK_FALLBACK[level](t, {
			provider: "P",
			domain: "d.com",
			host: "h.d.com",
		});
		widgetSourceCardRisk({ ...SOURCE, level, kind: "exact" }, t);
		widgetSourceCardRisk({ ...SOURCE, level }, t);
	}
	for (const copy of Object.values(WIDGET_SOURCE_RISK))
		copy(t, { provider: "P", domain: "d.com", host: "h.d.com" });
	for (const label of Object.values(WIDGET_DIRECTIVE_LABELS)) label(t);
	for (const copy of Object.values(WIDGET_CAPABILITY_COPY)) {
		copy.label(t);
		copy.description(t);
	}
	const leads: WidgetBannerLead[] = [
		{ kind: "capabilities" },
		{ kind: "unclassified", domain: "d.com" },
		{ kind: "broad", provider: "P" },
		{ kind: "shared", provider: "P" },
		{ kind: "external", domain: "d.com", others: 0 },
		{ kind: "external", domain: "d.com", others: 2 },
		{ kind: "known", provider: "P", others: 0 },
		{ kind: "known", provider: "P", others: 1 },
	];
	for (const lead of leads) widgetBannerLead(lead, t);
	widgetBannerSuffix(t);
	const titles: WidgetConsentTitle[] = [
		"runtime",
		"broad",
		"expanded",
		"network",
		"capabilities",
	];
	for (const title of titles) widgetConsentTitle(title, t);
	for (const source of ["local", "hub", "registry:*", "registry:hub.x.com"])
		widgetPolicySourceLabel(source, t);
	for (const code of [...RUNTIME_VALUE_ISSUE_CODES, ...SERVER_CODES])
		widgetRuntimeIssueReason(code, t);
}

const SERVER_CODES = [
	"reserved-host",
	"reserved-name",
	"wildcard",
	"unknown-slot",
	"engine",
	"too-large",
	"invalid-host",
];

describe("widget source About keys", () => {
	const aboutKeys = Object.keys(en).filter((key) =>
		key.startsWith("widgetSourceAbout"),
	);

	test("every catalog aboutKey has English copy and is known to the host", () => {
		const catalogKeys = catalog.providers
			.map((provider) => provider.aboutKey)
			.filter((key): key is string => typeof key === "string");
		expect(catalogKeys.length).toBeGreaterThan(0);
		for (const key of catalogKeys) {
			expect(en[key]).toBeString();
			expect(WIDGET_SOURCE_ABOUT_KEYS as readonly string[]).toContain(key);
		}
	});

	test("no About key is dead", () => {
		const used = new Set(
			catalog.providers.map((provider) => provider.aboutKey),
		);
		expect(aboutKeys.filter((key) => !used.has(key))).toEqual([]);
		expect([...WIDGET_SOURCE_ABOUT_KEYS].sort() as string[]).toEqual(
			[...aboutKeys].sort(),
		);
	});

	test("an About line is omitted for keys this build does not know", () => {
		const { t } = recorder();
		expect(
			widgetSourceAbout(
				{ ...SOURCE, aboutKey: "widgetSourceAboutMapTiles" },
				t,
			),
		).toBe("Example Maps is a map tile service.");
		expect(
			widgetSourceAbout(
				{ ...SOURCE, aboutKey: "widgetSourceAboutTeleport" as never },
				t,
			),
		).toBeNull();
	});
});

describe("widget source Risk copy", () => {
	test("every kind and level in the golden classifications has its own Risk entry", () => {
		const pairs = new Set(
			classification.classifications
				.filter((row) => row.kind && row.level)
				.map((row) => `${row.kind}:${row.level}`),
		);
		expect(pairs.size).toBeGreaterThan(5);
		for (const pair of pairs) {
			const [kind, level] = pair.split(":");
			expect(
				widgetSourceRiskEntry(kind, level as WidgetSourceLevel),
			).not.toBeNull();
		}
	});

	test("stale builds raise subdomains to broad and still say what the source is", () => {
		expect(widgetSourceRiskEntry("subdomains", "broad")).toBe("subdomains");
		expect(widgetSourceRiskEntry("tenant-subdomains", "broad")).toBe(
			"tenant-subdomains",
		);
	});

	test("kinds from a newer backend fall back to their level", () => {
		const { t } = recorder();
		expect(widgetSourceRiskEntry("teleport", "shared")).toBeNull();
		expect(
			widgetSourceRisk({ ...SOURCE, kind: "teleport", level: "shared" }, t),
		).toBe("Shared hosting where content can come from anyone.");
	});

	test("cards stay calm below broad; Details keep the full sentence", () => {
		const { t } = recorder();
		const cesium = {
			...SOURCE,
			provider: "Cesium ion",
			emphasis: "cesium.com",
			host: "api.cesium.com",
		};
		expect(widgetSourceCardRisk({ ...cesium, level: "external" }, t)).toBe(
			"Data goes to Cesium ion.",
		);
		expect(widgetSourceRisk({ ...cesium, level: "external" }, t)).toContain(
			"Anyone with a Cesium ion account",
		);
		expect(widgetSourceCardRisk({ ...cesium, level: "shared" }, t)).toBe(
			"Cesium ion hosts content its users upload.",
		);
		expect(widgetSourceRisk({ ...cesium, level: "shared" }, t)).toContain(
			"can come from anyone",
		);
		const bucket = {
			kind: "shared-wildcard",
			level: "broad" as const,
			provider: "Amazon S3",
			emphasis: "s3.eu-central-1.amazonaws.com",
			host: "*.s3.eu-central-1.amazonaws.com",
		};
		expect(widgetSourceCardRisk(bucket, t)).toBe(widgetSourceRisk(bucket, t));
	});

	test("params come from the closed set", () => {
		const keys = Object.keys(en).filter(
			(key) =>
				key.startsWith("widgetSourceAbout") ||
				key.startsWith("widgetSourceRisk"),
		);
		for (const key of keys) {
			const params = [...en[key].matchAll(/{{(\w+)}}/g)].map(
				(match) => match[1],
			);
			for (const param of params)
				expect(["provider", "domain", "host"]).toContain(param);
		}
	});

	test("every Risk key is reachable from the tables", () => {
		const { t, calls } = recorder();
		driveEveryTable(t);
		const used = new Set(calls.map((call) => call.key));
		const riskKeys = Object.keys(en).filter((key) =>
			key.startsWith("widgetSourceRisk"),
		);
		expect(riskKeys.filter((key) => !used.has(key))).toEqual([]);
	});
});

describe("consent copy tables", () => {
	test("every literal key has English copy that matches its default", () => {
		const { t, calls } = recorder();
		driveEveryTable(t);
		expect(calls.length).toBeGreaterThan(50);
		for (const call of calls) {
			const resource = resourceFor(call);
			expect({ key: call.key, resource }).toEqual({
				key: call.key,
				resource: call.fallback ?? resource,
			});
			expect(resource).toBeString();
		}
	});

	test("runtime issue codes map as the §14.5.4 table says", () => {
		const { t, calls } = recorder();
		const keyOf = (code: string) => {
			widgetRuntimeIssueReason(code, t);
			return calls.at(-1)?.key;
		};
		expect(keyOf("not-a-url")).toBe("widgetRuntimeIssueFormat");
		expect(keyOf("trailing-dot")).toBe("widgetRuntimeIssueFormat");
		expect(keyOf("port")).toBe("widgetRuntimeIssuePort");
		expect(keyOf("userinfo")).toBe("widgetRuntimeIssueCredentials");
		expect(keyOf("ip-literal")).toBe("widgetRuntimeIssueIp");
		expect(keyOf("template-unexpanded")).toBe("widgetRuntimeIssueTemplate");
		expect(keyOf("too-many-sources")).toBe("widgetRuntimeIssueTooMany");
		expect(keyOf("too-large")).toBe("widgetRuntimeIssueTooMany");
		expect(keyOf("reserved-name")).toBe("widgetRuntimeIssueReserved");
		expect(keyOf("platform-storage")).toBe("widgetRuntimeIssuePlatformStorage");
		expect(keyOf("wildcard")).toBe("widgetRuntimeIssueWholeDomain");
		expect(keyOf("unknown-slot")).toBe("widgetRuntimeIssueNotDeclared");
		expect(keyOf("engine")).toBe("widgetRuntimeIssueEngine");
		expect(keyOf("from-the-future")).toBe("widgetRuntimeIssueFormat");
	});

	test("level labels follow the calm naming", () => {
		const { t } = recorder();
		expect(
			WIDGET_SOURCE_LEVELS.map((level) => WIDGET_SOURCE_LEVEL_LABELS[level](t)),
		).toEqual([
			"Identified service",
			"External site",
			"Hosts user content",
			"Anyone can receive",
		]);
	});

	test("remembered runtime addresses never claim an expiry", () => {
		expect(en.widgetConsentRememberRuntimeNote).not.toMatch(/day/i);
	});
});
