"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	Ban,
	CalendarClock,
	CircleDashed,
	Hourglass,
	type LucideIcon,
	OctagonX,
	RefreshCw,
	TriangleAlert,
} from "lucide-react";
import {
	Fragment,
	type ReactNode,
	useCallback,
	useEffect,
	useMemo,
	useRef,
	useState,
} from "react";
import type {
	CertificatesRoute,
	CertificatesTab,
} from "../../../../lib/device-management/model/types";
import { TabsContent } from "../../../ui/tabs";
import { annunciatorSub } from "../primitives/annunciator";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { PageHeader } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { Headline } from "../primitives/headline";
import { InlineResult, type ResultTone } from "../primitives/inline-result";
import { cx } from "../primitives/tone";
import { type UnderlineTab, UnderlineTabs } from "../primitives/underline-tabs";
import { useDevicesRoute } from "../routing/use-devices-route";
import type { ScreenProps } from "../screen-props";
import { hubErrorCopy } from "../workspace/area-context";
import { useDeviceWorkspace } from "../workspace/device-workspace-provider";
import { useHubSupport } from "../workspace/use-hub";
import { AuthoritiesTab } from "./authorities-tab";
import {
	type AuthorityView,
	type CertificateRow,
	type FleetCounts,
	type WindowFilter,
	authorityView,
	canSign,
	fleetCounts,
	selfRenewing,
} from "./certificates-model";
import { ExpiryTab } from "./expiry-tab";
import { DeviceLink, certificateTitle, dayText, daysText } from "./parts";
import { RemindersTab } from "./reminders-tab";
import {
	type CertificateFleetRead,
	useAuthorities,
	useCertificateFleet,
	useSignedInName,
	useStillHere,
} from "./use-certificates";

const HOME: CertificatesRoute = { screen: "certificates", tab: "expiry" };
const EXPIRY_BLOCK_ID = "certificates-expiry";

/* The page's one conclusion (SPEC §6.4). */

function soonClause(
	t: DevicesT,
	soon: readonly CertificateRow[],
	after: boolean,
): string {
	const stuck = soon.filter((row) => row.failing).length;
	const renewing = soon.filter(selfRenewing).length;
	const [only] = soon;
	if (soon.length === 1 && only) {
		const span = daysText(t, only.days);
		const clause = after
			? t(
					"devices:certificates.headline.soonOneAfter",
					"one expires in {{span}}",
					{ span },
				)
			: t(
					"devices:certificates.headline.soonOne",
					"One certificate expires in {{span}}",
					{ span },
				);
		if (stuck)
			return t(
				"devices:certificates.headline.stuckOne",
				"{{clause}} and can't renew itself",
				{ clause },
			);
		return renewing
			? t(
					"devices:certificates.headline.renewingOne",
					"{{clause}} but renews itself",
					{ clause },
				)
			: clause;
	}
	const clause = after
		? t(
				"devices:certificates.headline.soonManyAfter",
				"{{count, number}} expire within 7 days",
				{ count: soon.length },
			)
		: t(
				"devices:certificates.headline.soonMany",
				"{{count, number}} certificates expire within 7 days",
				{ count: soon.length },
			);
	if (stuck)
		return t("devices:certificates.headline.stuckMany", {
			count: stuck,
			clause,
			defaultValue_one: "{{clause}}; {{count, number}} can't renew itself",
			defaultValue_other:
				"{{clause}}; {{count, number}} can't renew themselves",
		});
	return renewing === soon.length
		? t(
				"devices:certificates.headline.renewingMany",
				"{{clause}} but renew themselves",
				{ clause },
			)
		: clause;
}

function leadText(t: DevicesT, counts: FleetCounts): string | null {
	const clauses: string[] = [];
	if (counts.expired.length)
		clauses.push(
			t("devices:certificates.headline.expired", {
				count: counts.expired.length,
				defaultValue_one: "One certificate has expired",
				defaultValue_other: "{{count, number}} certificates have expired",
			}),
		);
	if (counts.week.length)
		clauses.push(soonClause(t, counts.week, clauses.length > 0));
	const [first, second] = clauses;
	if (!first) return null;
	return second
		? t("devices:certificates.headline.both", "{{first}} and {{second}}.", {
				first,
				second,
			})
		: t("devices:certificates.headline.single", "{{clause}}.", {
				clause: first,
			});
}

function silentSentence(t: DevicesT, counts: FleetCounts): string | null {
	const never = counts.never.length;
	const noAccess = counts.noAccess.length;
	if (never && noAccess)
		return t(
			"devices:certificates.headline.silentBoth",
			"{{first}}, and {{second}}.",
			{
				first: t("devices:certificates.headline.neverClause", {
					count: never,
					defaultValue_one:
						"{{count, number}} device hasn't reported certificates",
					defaultValue_other:
						"{{count, number}} devices haven't reported certificates",
				}),
				second: t("devices:certificates.headline.noAccessClause", {
					count: noAccess,
					defaultValue_one: "{{count, number}} is outside your access",
					defaultValue_other: "{{count, number}} are outside your access",
				}),
			},
		);
	if (never)
		return t("devices:certificates.headline.never", {
			count: never,
			defaultValue_one:
				"{{count, number}} device hasn't reported certificates.",
			defaultValue_other:
				"{{count, number}} devices haven't reported certificates.",
		});
	return noAccess
		? t("devices:certificates.headline.noAccess", {
				count: noAccess,
				defaultValue_one: "{{count, number}} device is outside your access.",
				defaultValue_other:
					"{{count, number}} devices are outside your access.",
			})
		: null;
}

function CertificatesHeadline({
	read,
	counts,
	authorities,
}: Readonly<{
	read: CertificateFleetRead;
	counts: FleetCounts;
	authorities: readonly AuthorityView[];
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	if (!read.loaded) return null;
	const { rows } = read.fleet;
	const urgent = leadText(t, counts);
	const lead =
		urgent ??
		(rows.length
			? t(
					"certificates.headline.calm",
					"No certificate expires in the next 7 days.",
				)
			: t(
					"certificates.headline.none",
					"No device has reported a certificate yet.",
				));
	const parts: ReactNode[] = [];
	const next = urgent ? undefined : rows.find((row) => row.days >= 0);
	if (next)
		parts.push(
			<Trans
				key="next"
				t={t}
				i18nKey="certificates.headline.next"
				defaults="The next one, <1>{{name}}</1> on <2/>, expires on {{date}}."
				values={{
					name: certificateTitle(next),
					date: dayText(time, next.notAfter),
				}}
				components={{
					1: <span className={next.detail ? "font-medium" : "font-mono"} />,
					2: (
						<DeviceLink
							deviceId={next.device.device_id}
							name={next.deviceName}
						/>
					),
				}}
			/>,
		);
	const silent = silentSentence(t, counts);
	if (silent) parts.push(silent);
	const ending = authorities.find((view) => view.signingSoon);
	if (ending)
		parts.push(
			t(
				"certificates.headline.authorityEnding",
				"Your authority {{label}} stops signing on {{date}}.",
				{
					label: ending.label,
					date: dayText(time, ending.signingExpiresAt),
				},
			),
		);
	const stuck = counts.week.find(
		(row) => row.failing && row.mode === "delegated",
	);
	if (stuck && !authorities.some(canSign))
		parts.push(
			t(
				"certificates.headline.needsAuthority",
				"Fixing the renewal of {{name}} needs an organisation authority that can sign, and none is on this computer.",
				{ name: certificateTitle(stuck) },
			),
		);
	return (
		<Headline
			lead={lead}
			rest={
				parts.length
					? parts.map((part, index) => (
							// biome-ignore lint/suspicious/noArrayIndexKey: the sentences are positional
							<Fragment key={index}>
								{index ? " " : null}
								{part}
							</Fragment>
						))
					: undefined
			}
		/>
	);
}

/* Summary windows: the same strip as the fleet's, with this page's six states. */

type WindowTone = "critical" | "warning" | "unknown" | "neutral";

interface SummaryWindow {
	id: WindowFilter;
	tone: WindowTone;
	icon: LucideIcon;
	label: string;
	count: number;
	names: readonly string[];
	/** Replaces the names line. */
	note?: string;
}

const LIT_INK: Record<WindowTone, string> = {
	critical: "text-critical",
	warning: "text-warning",
	unknown: "text-unknown",
	neutral: "text-foreground",
};

function summaryWindows(t: DevicesT, counts: FleetCounts): SummaryWindow[] {
	const named = (rows: readonly CertificateRow[]) =>
		rows.map((row) => `${certificateTitle(row)} · ${row.deviceName}`);
	const devices = (entries: FleetCounts["never"]) =>
		entries.map((entry) => entry.deviceName);
	return [
		{
			id: "expired",
			tone: counts.expiredInUse ? "critical" : "warning",
			icon: counts.expiredInUse ? OctagonX : TriangleAlert,
			label: t("devices:certificates.window.expired", "Expired"),
			count: counts.expired.length,
			names: named(counts.expired),
		},
		{
			id: "week",
			tone: "warning",
			icon: Hourglass,
			label: t("devices:certificates.window.week", "Expire within 7 d"),
			count: counts.week.length,
			names: named(counts.week),
		},
		{
			id: "month",
			tone: "neutral",
			icon: CalendarClock,
			label: t("devices:certificates.window.month", "Within 30 d"),
			count: counts.month.length,
			names: named(counts.month),
		},
		{
			id: "errors",
			tone: "warning",
			icon: RefreshCw,
			label: t("devices:certificates.window.errors", "Renewal failing"),
			count: counts.errors.length,
			names: named(counts.errors),
			...(!counts.errors.length && counts.renewalUnknown
				? {
						note: t("devices:certificates.window.renewalUnknown", {
							count: counts.renewalUnknown,
							defaultValue_one: "{{count, number}} unknown · needs a live read",
							defaultValue_other:
								"{{count, number}} unknown · need a live read",
						}),
					}
				: {}),
		},
		{
			id: "silent",
			tone: "unknown",
			icon: CircleDashed,
			label: t("devices:certificates.window.silent", "Not reported"),
			count: counts.never.length,
			names: devices(counts.never),
		},
		{
			id: "noaccess",
			tone: "unknown",
			icon: Ban,
			label: t("devices:certificates.window.noAccess", "No access"),
			count: counts.noAccess.length,
			names: devices(counts.noAccess),
		},
	];
}

function CertificateWindows({
	counts,
	pressed,
	onSelect,
}: Readonly<{
	counts: FleetCounts;
	pressed: WindowFilter | null;
	onSelect(filter: WindowFilter | null): void;
}>) {
	const { t } = useTranslation("devices");
	const windows = useMemo(() => summaryWindows(t, counts), [t, counts]);
	return (
		<div className="@container/annun flex min-w-0 flex-col gap-2">
			<fieldset
				aria-label={t(
					"certificates.window.label",
					"Certificates by state. Select a window to filter the list.",
				)}
				className="m-0 grid min-w-0 grid-cols-6 gap-px overflow-hidden rounded-lg border border-border bg-hairline p-0 @max-[900px]/annun:grid-cols-3 @max-[480px]/annun:grid-cols-2"
			>
				{windows.map((window) => {
					const lit = window.count > 0;
					const isPressed = pressed === window.id;
					const ink = lit ? LIT_INK[window.tone] : "text-muted-foreground";
					const sub =
						window.note ??
						annunciatorSub(t, { count: window.count, names: window.names });
					const Icon = window.icon;
					return (
						<button
							key={window.id}
							type="button"
							data-window={window.id}
							data-tone={window.tone}
							data-lit={lit}
							aria-pressed={isPressed}
							title={window.names.length ? window.names.join(", ") : sub}
							onClick={() => onSelect(isPressed ? null : window.id)}
							className={cx(
								"grid min-w-0 cursor-pointer grid-cols-[auto_minmax(0,1fr)] grid-rows-[auto_auto] items-center gap-x-2.5 border-0 px-3.5 py-2.5 text-left text-muted-foreground focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring",
								lit && window.tone === "critical"
									? "bg-critical-bg"
									: "bg-card hover:bg-row-hover",
								isPressed && "outline-2 -outline-offset-2 outline-foreground",
							)}
						>
							<span
								className={cx(
									"row-span-2 font-mono text-[22px] leading-6.5 font-medium tabular-nums",
									ink,
								)}
							>
								{window.count}
							</span>
							<span
								className={cx(
									"inline-flex min-w-0 items-center gap-1.25 truncate text-ui font-medium",
									lit && ink,
								)}
							>
								<Icon aria-hidden className="size-3.5 shrink-0" />
								<span className="truncate">{window.label}</span>
							</span>
							<span className="truncate text-xs text-muted-foreground">
								{sub}
							</span>
						</button>
					);
				})}
			</fieldset>
			<p className="max-w-[110ch] text-xs text-muted-foreground">
				{t(
					"certificates.window.caption",
					'Each device reports its certificate IDs, fingerprints and expiry dates to the hub on change and at least hourly, so expiry needs no password. "Within 30 d" includes the 7-day window. Revoked devices aren\'t counted.',
				)}
			</p>
		</div>
	);
}

/* Header. */

interface RefreshNote {
	tone: ResultTone;
	text: string;
}

function CertificatesHeader({
	read,
}: Readonly<{ read: CertificateFleetRead }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { href, navigate } = useDevicesRoute();
	const { host } = useHubSupport();
	const name = useSignedInName();
	const stillHere = useStillHere();
	const { clock } = useDeviceWorkspace();
	const [busy, setBusy] = useState(false);
	const [note, setNote] = useState<RefreshNote | null>(null);
	const home = { screen: "fleet", view: "devices" } as const;
	const { fleet } = read;

	const refresh = async () => {
		const here = stillHere();
		const from = read.freshness.dataFrom ?? read.freshness.at;
		setBusy(true);
		setNote(null);
		const error = await read.refresh().catch(() => undefined);
		if (!here()) return;
		setBusy(false);
		const at = time.clock(Math.floor(clock.now() / 1000));
		setNote(
			error
				? {
						tone: "warning",
						text:
							from === undefined
								? t(
										"certificates.header.refreshFailed",
										"Couldn't refresh at {{time}}: {{reason}}",
										{ time: at, reason: hubErrorCopy(t, error.code) },
									)
								: t(
										"certificates.header.refreshFailedFrom",
										"Couldn't refresh at {{time}}: {{reason}} Showing reports from {{from}}.",
										{
											time: at,
											reason: hubErrorCopy(t, error.code),
											from: time.clock(from),
										},
									),
					}
				: {
						tone: "good",
						text: t(
							"certificates.header.refreshed",
							"Checked at {{time}}. Certificate reports are current.",
							{ time: at },
						),
					},
		);
	};

	const reported = read.loaded
		? t(
				"certificates.header.reported",
				"{{certificates}} reported by {{reporting, number}} of {{devices}} on {{host}}",
				{
					certificates: t("certificates.count.certificates", {
						count: fleet.rows.length,
						defaultValue_one: "{{count, number}} certificate",
						defaultValue_other: "{{count, number}} certificates",
					}),
					reporting: fleet.devicesWithCertificates,
					devices: t("certificates.count.devices", {
						count: fleet.devices.length,
						defaultValue_one: "{{count, number}} device",
						defaultValue_other: "{{count, number}} devices",
					}),
					host,
				},
			)
		: t(
				"certificates.header.reading",
				"Reading certificate reports from {{host}}",
				{
					host,
				},
			);
	const local = name
		? t(
				"certificates.header.authoritiesFor",
				"authorities on this computer for {{name}}",
				{ name },
			)
		: t("certificates.header.authorities", "authorities on this computer");
	return (
		<PageHeader
			crumbs={[
				{
					label: t("certificates.header.crumbDevices", "Devices"),
					href: href(home),
					onNavigate: () => navigate(home),
				},
				{ label: t("certificates.title", "Certificates") },
			]}
			title={t("certificates.title", "Certificates")}
			sub={`${reported} · ${local}`}
			actions={
				<>
					{note ? (
						<InlineResult tone={note.tone} onDismiss={() => setNote(null)}>
							{note.text}
						</InlineResult>
					) : null}
					<DvButton icon={RefreshCw} busy={busy} onClick={() => void refresh()}>
						{t("certificates.header.refresh", "Refresh")}
					</DvButton>
				</>
			}
		/>
	);
}

function useTabs(
	t: DevicesT,
	counts: FleetCounts,
	authorities: readonly AuthorityView[],
): UnderlineTab<CertificatesTab>[] {
	return useMemo(() => {
		const broken = authorities.filter((view) => !canSign(view)).length;
		const ending = authorities.filter(
			(view) => view.signingSoon || view.rootSoon,
		).length;
		return [
			{
				value: "expiry",
				label: t("devices:certificates.tab.expiry", "Expiry across devices"),
				shortLabel: t("devices:certificates.tab.expiryShort", "Expiry"),
				...(counts.needYou
					? {
							count: {
								count: counts.needYou,
								tone: counts.expiredInUse
									? ("critical" as const)
									: ("warning" as const),
								label: t("devices:certificates.tab.expiryCount", {
									count: counts.needYou,
									defaultValue_one: "{{count, number}} certificate needs you",
									defaultValue_other: "{{count, number}} certificates need you",
								}),
							},
						}
					: {}),
			},
			{
				value: "authorities",
				label: t(
					"devices:certificates.tab.authorities",
					"Organisation authorities",
				),
				shortLabel: t(
					"devices:certificates.tab.authoritiesShort",
					"Authorities",
				),
				...(broken
					? {
							count: {
								count: broken,
								tone: "critical" as const,
								label: t(
									"devices:certificates.tab.authoritiesBroken",
									"An authority can't sign",
								),
							},
						}
					: ending
						? {
								count: {
									count: ending,
									tone: "warning" as const,
									label: t(
										"devices:certificates.tab.authoritiesEnding",
										"A signing key or root expires soon",
									),
								},
							}
						: {}),
			},
			{
				value: "reminders",
				label: t("devices:certificates.tab.reminders", "Reminders"),
			},
		];
	}, [t, counts, authorities]);
}

/** SPEC §5.8 / IA §6.2 N8: certificate expiry across the fleet without unlocking, and the organisation authorities kept on this computer. */
export function CertificatesScreen({ route }: Readonly<ScreenProps>) {
	const { t } = useTranslation("devices");
	const { navigate, clearParam } = useDevicesRoute();
	const certificates = route.screen === "certificates" ? route : HOME;
	const tab = certificates.tab ?? "expiry";
	const read = useCertificateFleet();
	const store = useAuthorities();
	const [filter, setFilter] = useState<WindowFilter | null>(null);
	const [deviceId, setDeviceId] = useState<string | null>(null);
	const [creating, setCreating] = useState(false);
	const [restoring, setRestoring] = useState(false);
	const leaveGuard = useRef<(() => boolean) | null>(null);

	const counts = useMemo(() => fleetCounts(read.fleet), [read.fleet]);
	const authorities = useMemo(
		() => store.authorities.map((entry) => authorityView(entry, read.now)),
		[store.authorities, read.now],
	);
	const tabs = useTabs(t, counts, authorities);

	const setTab = useCallback(
		(next: CertificatesTab) => {
			if (next !== "authorities" && leaveGuard.current?.()) return;
			navigate({ screen: "certificates", tab: next }, { replace: true });
		},
		[navigate],
	);

	// One-shot deep link: open the wizard on its tab, then drop the param so a reload doesn't reopen it.
	const { action } = certificates;
	useEffect(() => {
		if (action !== "create-authority") return;
		setCreating(true);
		if (tab === "authorities") clearParam("action");
		else
			navigate(
				{ screen: "certificates", tab: "authorities" },
				{ replace: true },
			);
	}, [action, tab, clearParam, navigate]);

	const selectWindow = (next: WindowFilter | null) => {
		setFilter(next);
		if (tab !== "expiry") setTab("expiry");
		requestAnimationFrame(() =>
			document
				.getElementById(EXPIRY_BLOCK_ID)
				?.scrollIntoView({ block: "start", behavior: "smooth" }),
		);
	};
	const restoreAuthority = () => {
		setRestoring(true);
		setTab("authorities");
	};

	return (
		<>
			<CertificatesHeader read={read} />
			<CertificatesHeadline
				read={read}
				counts={counts}
				authorities={authorities}
			/>
			{read.loaded ? (
				<CertificateWindows
					counts={counts}
					pressed={filter}
					onSelect={selectWindow}
				/>
			) : null}
			<UnderlineTabs
				label={t("certificates.tabs", "Certificate sections")}
				tabs={tabs}
				value={tab}
				onValueChange={setTab}
			>
				<TabsContent value="expiry" className="mt-4 flex flex-col gap-4">
					<ExpiryTab
						read={read}
						counts={counts}
						filter={filter}
						onFilter={setFilter}
						deviceId={deviceId}
						onDevice={setDeviceId}
						canSignHere={authorities.some(canSign)}
						onRestoreAuthority={restoreAuthority}
					/>
				</TabsContent>
				<TabsContent value="authorities" className="mt-4 flex flex-col gap-4">
					<AuthoritiesTab
						store={store}
						authorities={authorities}
						rows={read.fleet.rows}
						creating={creating}
						onCreating={setCreating}
						restoring={restoring}
						onRestoring={setRestoring}
						leaveGuard={leaveGuard}
					/>
				</TabsContent>
				<TabsContent value="reminders" className="mt-4 flex flex-col gap-4">
					<RemindersTab read={read} />
				</TabsContent>
			</UnderlineTabs>
		</>
	);
}
