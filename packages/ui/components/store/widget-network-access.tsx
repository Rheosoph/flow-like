"use client";

import { useTranslation } from "@flow-like/locales";
import { Globe, Radio, TriangleAlert } from "lucide-react";
import { useMemo } from "react";
import {
	type PackageWidgetNetworkView,
	type WidgetClassifiedPurpose,
	type WidgetDeclaredInput,
	type WidgetPurposeSource,
	type WidgetReviewFlag,
	describePackageWidgetNetwork,
	widgetReviewFlags,
	widgetSourceHost,
} from "../../lib/package-capabilities";
import type { PackageWidgetEntry } from "../../lib/schema/wasm";
import { cn } from "../../lib/utils";
import type { WidgetConsentChip } from "../a2ui/micro-widget-consent-view";
import {
	WIDGET_CAPABILITY_COPY,
	WIDGET_DIRECTIVE_LABELS,
	widgetPolicySourceLabel,
	widgetSourceAbout,
	widgetSourceRisk,
} from "../a2ui/micro-widget-network-copy";
import {
	type MicroWidgetCapability,
	WIDGET_CSP_DIRECTIVES,
	WIDGET_CSP_KEYS,
	type WidgetCspKey,
	type WidgetNetworkSource,
	type WidgetSourceKind,
	maxWidgetSourceLevel,
	policyCapabilities,
	widgetSourceLevelRank,
} from "../a2ui/micro-widget-policy";
import {
	WidgetHostCopy,
	WidgetPublisherNote,
	WidgetSourceChip,
	WidgetSourceLevelBadge,
} from "../a2ui/micro-widget-purpose-card";
import { Badge } from "../ui/badge";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../ui/card";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "../ui/table";

/** Capabilities that let a widget move data off the device or record the user. */
const ELEVATED_WIDGET_CAPABILITIES: ReadonlySet<MicroWidgetCapability> =
	new Set(["microphone", "downloads"]);

const ELEVATED_CHIP =
	"border-primary/35 bg-primary/10 font-normal text-primary";

const EMPTY_CELL = "—";

export function useWidgetPolicyLabels() {
	const { t } = useTranslation("store");
	const { t: common } = useTranslation("common");
	return useMemo(() => {
		const network = (key: WidgetCspKey): string =>
			WIDGET_DIRECTIVE_LABELS[key](common);
		return {
			network,
			capability: (name: MicroWidgetCapability): string =>
				WIDGET_CAPABILITY_COPY[name].label(common),
			origin: (source: string): string =>
				widgetPolicySourceLabel(source, common),
			directives: (keys: readonly WidgetCspKey[]): string =>
				WIDGET_CSP_KEYS.filter((key) => keys.includes(key))
					.map(network)
					.join(" · "),
			kind(kind: WidgetSourceKind): string {
				switch (kind) {
					case "service":
						return t("widgetSourceKindService", "Service");
					case "exact":
						return t("widgetSourceKindExact", "Single site");
					case "subdomains":
						return t("widgetSourceKindSubdomains", "Subdomains of one site");
					case "tenant-host":
						return t(
							"widgetSourceKindTenantHost",
							"One customer on shared hosting",
						);
					case "tenant-subdomains":
						return t(
							"widgetSourceKindTenantSubdomains",
							"Subdomains of one customer on shared hosting",
						);
					case "shared-host":
						return t("widgetSourceKindSharedHost", "Shared host");
					case "shared-wildcard":
						return t(
							"widgetSourceKindSharedWildcard",
							"Every customer of a shared platform",
						);
					case "platform-host":
						return t(
							"widgetSourceKindPlatformHost",
							"Provider host, tenancy unknown",
						);
					case "platform-storage":
						return t(
							"widgetSourceKindPlatformStorage",
							"This app's Flow-Like storage",
						);
					default:
						return kind;
				}
			},
		};
	}, [t, common]);
}

export function WidgetCapabilityBadges({
	capabilities,
	className,
}: {
	capabilities: readonly MicroWidgetCapability[];
	className?: string;
}) {
	const labels = useWidgetPolicyLabels();
	if (capabilities.length === 0) return null;
	return (
		<div className={cn("flex flex-wrap gap-1", className)}>
			{capabilities.map((name) => (
				<Badge
					key={name}
					variant="outline"
					data-widget-capability={name}
					className={cn(
						"text-xs font-normal",
						ELEVATED_WIDGET_CAPABILITIES.has(name) && ELEVATED_CHIP,
					)}
				>
					{labels.capability(name)}
				</Badge>
			))}
		</div>
	);
}

function useWidgetNetworkView(
	widget: Pick<PackageWidgetEntry, "contract" | "network">,
): PackageWidgetNetworkView {
	const { contract, network } = widget;
	return useMemo(
		() => describePackageWidgetNetwork({ contract, network }),
		[contract, network],
	);
}

/** One chip per host (`https://h` and `wss://h` merge), at the riskiest level of its sources. */
function storeChips(
	entries: readonly { host: string; class?: WidgetNetworkSource }[],
): WidgetConsentChip[] {
	const chips = new Map<string, WidgetConsentChip>();
	for (const { host, class: found } of entries) {
		const level = found?.level ?? "external";
		const existing = chips.get(host);
		if (existing) {
			existing.level = maxWidgetSourceLevel([existing.level, level]) ?? level;
			continue;
		}
		const wildcard = host.startsWith("*.");
		const bare = wildcard ? host.slice(2) : host;
		chips.set(host, {
			key: host,
			host: bare,
			wildcard,
			emphasis: found?.emphasis ?? bare,
			runtime: false,
			platformStorage: false,
			level,
			isNew: false,
			raised: false,
			included: true,
		});
	}
	return [...chips.values()];
}

function purposeChipEntries(sources: readonly WidgetPurposeSource[]) {
	return sources.map((entry) => ({
		host: entry.class?.host ?? widgetSourceHost(entry.source),
		class: entry.class,
	}));
}

function WidgetRuntimeBadge() {
	const { t } = useTranslation("store");
	return (
		<Badge
			variant="outline"
			className="gap-1 text-xs font-normal text-muted-foreground"
			data-widget-network-runtime
		>
			<Radio aria-hidden="true" className="h-3 w-3" />
			{t("widgetNetworkRuntimeBadge", "+ addresses provided at runtime")}
		</Badge>
	);
}

/** Card surface: "Network: N addresses", its level, the hosts, and that store previews stay offline. */
export function WidgetNetworkSummary({
	widget,
	showPreviewNote = false,
}: {
	widget: Pick<PackageWidgetEntry, "contract" | "network">;
	showPreviewNote?: boolean;
}) {
	const { t } = useTranslation("store");
	const { purposes, network, hosts, hasInputs } = useWidgetNetworkView(widget);
	const chips = useMemo(
		() =>
			storeChips(
				purposes.flatMap((purpose) => purposeChipEntries(purpose.sources)),
			).sort((left, right) =>
				left.key < right.key ? -1 : left.key > right.key ? 1 : 0,
			),
		[purposes],
	);
	if (hosts.length === 0 && !hasInputs) return null;
	return (
		<div className="space-y-1.5" data-widget-network>
			<div className="flex flex-wrap items-center gap-1">
				{hosts.length > 0 && (
					<Badge
						variant="outline"
						className={cn(
							"gap-1 text-xs font-normal",
							!network && ELEVATED_CHIP,
						)}
						data-widget-network-badge
					>
						<Globe aria-hidden="true" className="h-3 w-3" />
						{t("widgetNetworkAddresses", {
							defaultValue_one: "Network: {{count}} address",
							defaultValue_other: "Network: {{count}} addresses",
							count: hosts.length,
						})}
					</Badge>
				)}
				{hosts.length > 0 && network && (
					<WidgetSourceLevelBadge level={network.level} />
				)}
				{hasInputs && <WidgetRuntimeBadge />}
			</div>
			{chips.length > 0 && (
				<div className="flex flex-wrap gap-1">
					{chips.map((chip) => (
						<WidgetSourceChip key={chip.key} chip={chip} />
					))}
				</div>
			)}
			{showPreviewNote && (
				<p className="text-xs text-muted-foreground">
					{t(
						"widgetPreviewRunsWithoutNetwork",
						"Preview runs without network access",
					)}
				</p>
			)}
		</div>
	);
}

function WidgetHeading({ widget }: { widget: PackageWidgetEntry }) {
	return (
		<div className="flex min-w-0 items-baseline justify-between gap-2">
			<span className="truncate text-sm font-medium">{widget.name}</span>
			<span className="shrink-0 font-mono text-xs text-muted-foreground">
				{widget.id}
			</span>
		</div>
	);
}

function purposeDirectives(purpose: WidgetClassifiedPurpose): WidgetCspKey[] {
	const used = new Set<WidgetCspKey>([
		...purpose.sources.flatMap((source) => source.directives),
		...purpose.inputs.flatMap((input) => input.directives),
	]);
	return WIDGET_CSP_KEYS.filter((key) => used.has(key));
}

function byLevelDescending(
	purposes: readonly WidgetClassifiedPurpose[],
): WidgetClassifiedPurpose[] {
	const rank = (purpose: WidgetClassifiedPurpose) =>
		purpose.level ? widgetSourceLevelRank(purpose.level) : -1;
	return [...purposes].sort((left, right) => rank(right) - rank(left));
}

function WidgetTemplateText({ input }: { input: WidgetDeclaredInput }) {
	const { t } = useTranslation("store");
	const { template } = input;
	if (template?.subdomainsInput) {
		return (
			<>
				{t("widgetNetworkRuntimeSubdomains", "Subdomains from input")}{" "}
				<code className="font-mono text-foreground">
					{template.subdomainsInput}
				</code>
			</>
		);
	}
	if (template?.subdomains) {
		return (
			<>
				{t("widgetReviewTemplateSubdomains", "subdomains: {{list}}", {
					list: template.subdomains.join(", "),
				})}
			</>
		);
	}
	return null;
}

function WidgetRuntimeSlots({
	inputs,
}: {
	inputs: readonly WidgetDeclaredInput[];
}) {
	const { t } = useTranslation("store");
	const labels = useWidgetPolicyLabels();
	return (
		<div className="flex flex-col gap-1" data-widget-runtime-slots>
			<p className="flex items-center gap-1 text-xs text-muted-foreground">
				<Radio aria-hidden="true" className="h-3 w-3 shrink-0" />
				{t(
					"widgetNetworkRuntimeHeading",
					"Addresses provided while the app runs",
				)}
			</p>
			<ul className="space-y-0.5 text-xs text-muted-foreground">
				{inputs.map((input) => (
					<li key={input.path} data-widget-runtime-slot={input.path}>
						<code className="rounded bg-muted px-1 font-mono text-foreground">
							{input.path}
						</code>
						{" · "}
						{labels.directives(input.directives)}
						{input.template && (
							<>
								{" · "}
								<WidgetTemplateText input={input} />
							</>
						)}
					</li>
				))}
			</ul>
			<WidgetHostCopy>
				{t(
					"widgetNetworkRuntimeClassified",
					"Each address is checked and shown to the viewer for approval when it appears.",
				)}
			</WidgetHostCopy>
		</div>
	);
}

/**
 * Store density of a purpose (§14.5.6): level, uses, every address, its About
 * and Risk lines and the declared input slots; the publisher's reason last.
 * Nothing collapses.
 */
export function WidgetStorePurposeCard({
	purpose,
}: {
	purpose: WidgetClassifiedPurpose;
}) {
	const { t: common } = useTranslation("common");
	const labels = useWidgetPolicyLabels();
	const chips = useMemo(
		() => storeChips(purposeChipEntries(purpose.sources)),
		[purpose.sources],
	);
	const providers = useMemo(
		() => [
			...new Set(
				purpose.sources.flatMap((entry) =>
					entry.class?.provider ? [entry.class.provider] : [],
				),
			),
		],
		[purpose.sources],
	);
	const copy = useMemo(
		() => [
			...new Set(
				purpose.sources.flatMap(({ class: found }) =>
					found
						? [
								widgetSourceAbout(found, common),
								widgetSourceRisk(found, common),
							].filter((line): line is string => line !== null)
						: [],
				),
			),
		],
		[purpose.sources, common],
	);
	return (
		<section
			className="flex flex-col gap-1.5 rounded-md border p-2.5"
			data-widget-purpose
			data-widget-purpose-level={purpose.level}
		>
			<div className="flex flex-wrap items-center gap-x-2 gap-y-1">
				{purpose.level && <WidgetSourceLevelBadge level={purpose.level} />}
				<span className="text-xs text-muted-foreground">
					{labels.directives(purposeDirectives(purpose))}
				</span>
			</div>
			{chips.length > 0 && (
				<div className="flex flex-wrap items-center gap-1.5">
					{chips.map((chip) => (
						<WidgetSourceChip key={chip.key} chip={chip} />
					))}
					{providers.length > 0 && (
						<span translate="no" className="text-xs text-muted-foreground">
							{providers.join(", ")}
						</span>
					)}
				</div>
			)}
			{copy.map((line) => (
				<WidgetHostCopy key={line}>{line}</WidgetHostCopy>
			))}
			{purpose.inputs.length > 0 && (
				<WidgetRuntimeSlots inputs={purpose.inputs} />
			)}
			{purpose.reason && <WidgetPublisherNote reason={purpose.reason} />}
		</section>
	);
}

/** Package detail, Permissions tab: what each widget asks to reach, and why. */
export function WidgetNetworkAccessCard({
	widgets,
}: {
	widgets: readonly PackageWidgetEntry[];
}) {
	const { t } = useTranslation("store");
	if (widgets.length === 0) return null;
	return (
		<Card data-widget-network-access-card>
			<CardHeader>
				<CardTitle className="text-base">
					{t("widgetNetworkAccess", "Widget network access")}
				</CardTitle>
				<CardDescription>
					{t(
						"widgetNetworkAccessDescription",
						"Addresses each widget asks to reach. A widget reaches none of them until the person viewing it approves them.",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-3">
				{widgets.map((widget) => (
					<WidgetNetworkAccessRow key={widget.id} widget={widget} />
				))}
			</CardContent>
		</Card>
	);
}

function WidgetNetworkAccessRow({ widget }: { widget: PackageWidgetEntry }) {
	const { t } = useTranslation("store");
	const { purposes } = useWidgetNetworkView(widget);
	const ordered = useMemo(() => byLevelDescending(purposes), [purposes]);
	return (
		<div className="space-y-2 rounded-lg border p-3" data-widget-id={widget.id}>
			<WidgetHeading widget={widget} />
			{ordered.length > 0 ? (
				ordered.map((purpose, index) => (
					<WidgetStorePurposeCard
						key={`${purpose.reason}:${index}`}
						purpose={purpose}
					/>
				))
			) : (
				<p className="text-xs text-muted-foreground">
					{t("widgetNoNetworkAccess", "No network access")}
				</p>
			)}
		</div>
	);
}

/** Admin review: every browser capability, address and input a widget contract declares. */
export function WidgetCapabilitiesCard({
	widgets,
}: {
	widgets: readonly PackageWidgetEntry[];
}) {
	const { t } = useTranslation("store");
	if (widgets.length === 0) return null;
	return (
		<Card data-widget-capabilities-card>
			<CardHeader>
				<CardTitle>{t("widgetCapabilities", "Widget capabilities")}</CardTitle>
				<CardDescription>
					{t(
						"widgetCapabilitiesDescription",
						"Browser capabilities and network addresses each widget declares. Viewers must approve them before they take effect, so check that every address is needed.",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-3">
				{widgets.map((widget) => (
					<WidgetCapabilitiesRow key={widget.id} widget={widget} />
				))}
			</CardContent>
		</Card>
	);
}

function useReviewFlagText() {
	const { t } = useTranslation("store");
	return (flag: WidgetReviewFlag): string => {
		switch (flag.kind) {
			case "broad":
				return t(
					"widgetReviewFlagBroad",
					"{{source}}: anyone who signs up with {{provider}} could receive what the widget sends.",
					{ source: flag.source, provider: flag.provider },
				);
			case "shared":
				return t(
					"widgetReviewFlagShared",
					"{{source}}: anyone can publish content the widget loads.",
					{ source: flag.source },
				);
			case "runtime-connect":
				return t(
					"widgetReviewFlagRuntimeConnect",
					"Input {{input}} can add sites the widget sends data to.",
					{ input: flag.input },
				);
			case "platform-storage":
				return t(
					"widgetReviewFlagPlatformStorage",
					"Input {{input}} can reach this app's files in Flow-Like storage.",
					{ input: flag.input },
				);
		}
	};
}

function WidgetReviewFlags({ flags }: { flags: readonly WidgetReviewFlag[] }) {
	const { t } = useTranslation("store");
	const text = useReviewFlagText();
	if (flags.length === 0) return null;
	return (
		<div
			className="space-y-1 rounded-md border border-destructive/40 p-3"
			data-widget-review-flags
		>
			<p className="flex items-center gap-1.5 text-xs font-medium text-destructive">
				<TriangleAlert aria-hidden="true" className="h-3.5 w-3.5" />
				{t("widgetReviewFlags", "Check before approving")}
			</p>
			<ul className="list-disc space-y-0.5 pl-5 text-xs">
				{flags.map((flag) => (
					<li
						key={`${flag.kind}:${"source" in flag ? flag.source : flag.input}`}
						data-widget-review-flag={flag.kind}
						className="break-words"
					>
						{text(flag)}
					</li>
				))}
			</ul>
		</div>
	);
}

function rawDirectives(keys: readonly WidgetCspKey[]): string {
	return keys.map((key) => WIDGET_CSP_DIRECTIVES[key]).join(", ");
}

function WidgetSourcesTable({
	purposes,
}: {
	purposes: readonly WidgetClassifiedPurpose[];
}) {
	const { t } = useTranslation("store");
	const labels = useWidgetPolicyLabels();
	const rows = purposes.flatMap((purpose, index) =>
		purpose.sources.map((entry) => ({ number: index + 1, entry })),
	);
	if (rows.length === 0) return null;
	return (
		<Table data-widget-review-sources>
			<TableHeader>
				<TableRow>
					<TableHead>{t("widgetReviewColumnPurpose", "Purpose")}</TableHead>
					<TableHead>{t("widgetReviewColumnSource", "Source")}</TableHead>
					<TableHead>{t("widgetReviewColumnUse", "Use")}</TableHead>
					<TableHead>{t("widgetReviewColumnKind", "Kind")}</TableHead>
					<TableHead>{t("widgetReviewColumnLevel", "Level")}</TableHead>
					<TableHead>{t("widgetReviewColumnProvider", "Provider")}</TableHead>
				</TableRow>
			</TableHeader>
			<TableBody>
				{rows.map(({ number, entry }) => (
					<TableRow key={`${number}:${entry.source}`}>
						<TableCell className="tabular-nums">{number}</TableCell>
						<TableCell>
							<bdi dir="ltr" translate="no" className="font-mono text-xs">
								{entry.source}
							</bdi>
						</TableCell>
						<TableCell className="font-mono text-xs">
							{rawDirectives(entry.directives)}
						</TableCell>
						<TableCell className="text-xs">
							{entry.class ? labels.kind(entry.class.kind) : EMPTY_CELL}
						</TableCell>
						<TableCell>
							{entry.class ? (
								<WidgetSourceLevelBadge level={entry.class.level} />
							) : (
								EMPTY_CELL
							)}
						</TableCell>
						<TableCell className="text-xs" translate="no">
							{entry.class?.provider ?? t("widgetReviewNoProvider", "Unknown")}
						</TableCell>
					</TableRow>
				))}
			</TableBody>
		</Table>
	);
}

function templateCell(
	input: WidgetDeclaredInput,
	t: ReturnType<typeof useTranslation<"store">>["t"],
): string {
	if (input.template?.subdomainsInput) {
		return t(
			"widgetReviewTemplateSubdomainsInput",
			"subdomains from {{input}}",
			{ input: input.template.subdomainsInput },
		);
	}
	if (input.template?.subdomains) {
		return t("widgetReviewTemplateSubdomains", "subdomains: {{list}}", {
			list: input.template.subdomains.join(", "),
		});
	}
	return EMPTY_CELL;
}

function WidgetInputsTable({
	purposes,
}: {
	purposes: readonly WidgetClassifiedPurpose[];
}) {
	const { t } = useTranslation("store");
	const rows = purposes.flatMap((purpose, index) =>
		purpose.inputs.map((input) => ({ number: index + 1, input })),
	);
	if (rows.length === 0) return null;
	return (
		<Table data-widget-review-inputs>
			<TableHeader>
				<TableRow>
					<TableHead>{t("widgetReviewColumnPurpose", "Purpose")}</TableHead>
					<TableHead>{t("widgetReviewColumnInput", "Input")}</TableHead>
					<TableHead>{t("widgetReviewColumnUse", "Use")}</TableHead>
					<TableHead>{t("widgetReviewColumnTemplate", "Template")}</TableHead>
				</TableRow>
			</TableHeader>
			<TableBody>
				{rows.map(({ number, input }) => (
					<TableRow key={input.path}>
						<TableCell className="tabular-nums">{number}</TableCell>
						<TableCell>
							<code className="font-mono text-xs">{input.path}</code>
						</TableCell>
						<TableCell className="font-mono text-xs">
							{rawDirectives(input.directives)}
						</TableCell>
						<TableCell className="text-xs">{templateCell(input, t)}</TableCell>
					</TableRow>
				))}
			</TableBody>
		</Table>
	);
}

function WidgetCapabilitiesRow({ widget }: { widget: PackageWidgetEntry }) {
	const { t } = useTranslation("store");
	const capabilities = useMemo(
		() => policyCapabilities(widget.contract?.capabilities),
		[widget.contract],
	);
	const { purposes, network } = useWidgetNetworkView(widget);
	const flags = useMemo(() => widgetReviewFlags(purposes), [purposes]);
	return (
		<div className="space-y-3 rounded-lg border p-3" data-widget-id={widget.id}>
			<WidgetHeading widget={widget} />
			{capabilities.length === 0 && purposes.length === 0 ? (
				<p className="text-xs text-muted-foreground">
					{t(
						"widgetDeclaresNothing",
						"Declares no capabilities or network addresses",
					)}
				</p>
			) : (
				<>
					<WidgetCapabilityBadges capabilities={capabilities} />
					<WidgetReviewFlags flags={flags} />
					<WidgetSourcesTable purposes={purposes} />
					<WidgetInputsTable purposes={purposes} />
					{purposes.length > 0 && (
						<ol
							className="list-decimal space-y-1 pl-5 text-xs"
							data-widget-review-reasons
						>
							{purposes.map((purpose, index) => (
								<li key={`${purpose.reason}:${index}`}>
									<WidgetPublisherNote reason={purpose.reason} />
								</li>
							))}
						</ol>
					)}
					{network && (
						<p className="text-[10px] text-muted-foreground">
							{t(
								"widgetReviewDataVersion",
								"Classified with catalog {{catalogVersion}} and public suffix list {{pslVersion}}",
								{
									catalogVersion: network.catalogVersion,
									pslVersion: network.pslVersion,
								},
							)}
						</p>
					)}
				</>
			)}
		</div>
	);
}
