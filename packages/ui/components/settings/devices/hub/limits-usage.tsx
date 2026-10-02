"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { Gauge } from "lucide-react";
import type { ReactNode } from "react";
import type { PendingSetup } from "../../../../lib/device-management/model/types";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { Block } from "../primitives/block";
import { Meter } from "../primitives/meter";
import { StateView } from "../primitives/state-view";
import { TONE_TEXT, cx } from "../primitives/tone";
import { useRouteLink } from "../routing/use-devices-route";
import {
	type HubRead,
	type HubSupportRead,
	hubErrorCopy,
	useDeviceUsage,
	useHubSupport,
	usePendingSetups,
} from "../workspace";
import { Figure, Hint, HubReadStamp, Tech, useLocale } from "./hub-parts";
import {
	type DeviceSlots,
	type HubLimits,
	limitTone,
	limitsOf,
	slotsOf,
} from "./use-hub-facts";

const NAMES_SHOWN = 3;
const LINK = "underline decoration-border-strong underline-offset-2";
const FLEET = { screen: "fleet", view: "devices" } as const;

interface LimitFacts {
	hub: HubSupportRead;
	limits: HubLimits;
	slots?: DeviceSlots;
	/** Packages created in the last 24 hours, as the hub counts them. */
	today?: number;
	/** The hub is older than its usage report (BG3): limits only. */
	interim: boolean;
	/** The usage report exists but its last read failed and nothing is known yet. */
	usageError?: string;
	waiting: PendingSetup[];
	stamp: ReactNode;
}

function useLimitFacts(): LimitFacts {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const hub = useHubSupport();
	const usage: HubRead<unknown> = useDeviceUsage();
	const setups = usePendingSetups();
	const { support } = hub;
	const counts = support.usage;
	const reported = !usage.missingOnHub && counts !== undefined;
	const read = reported ? usage : hub;
	const slots = slotsOf(support);
	const failed = !counts && support.state === "on" ? usage.error : undefined;
	return {
		hub,
		limits: limitsOf(support),
		...(slots ? { slots } : {}),
		...(counts ? { today: counts.enrollments_last_24h } : {}),
		interim: usage.missingOnHub,
		...(failed ? { usageError: hubErrorCopy(t, failed.code) } : {}),
		waiting: setups.filter(
			(setup) => setup.state === "pending" && setup.expiresAt > time.nowS,
		),
		stamp: (
			<HubReadStamp
				freshness={read.freshness}
				loading={support.state === "checking"}
			/>
		),
	};
}

const hasLimits = (limits: HubLimits) => Object.keys(limits).length > 0;

const share = (used: number, max: number) => (max > 0 ? (used / max) * 100 : 0);

/** Why the counts are missing: an older hub (BG3 interim) or a failed read. Never an error banner. */
const usageNote = (t: DevicesT, facts: LimitFacts) => {
	if (facts.interim)
		return t(
			"devices:hub.limits.interim",
			"This hub doesn't report how much of them you use yet, so only the limits are shown.",
		);
	if (facts.usageError)
		return t(
			"devices:hub.limits.usageFailed",
			"Your usage couldn't be read: {{cause}} The limits are from the hub's settings.",
			{ cause: facts.usageError },
		);
	return undefined;
};

/** "factory-line-3 (expires in 20h), test-vm (expires in 2h) and 2 more". */
function WaitingNames({ waiting }: Readonly<{ waiting: PendingSetup[] }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	if (!waiting.length) return null;
	const shown = waiting.slice(0, NAMES_SHOWN);
	const more = waiting.length - shown.length;
	return (
		<span className="text-muted-foreground">
			{shown.map((setup, index) => (
				<span key={setup.enrollmentId}>
					{index ? ", " : ""}
					<span className="font-mono text-foreground">{setup.name}</span>{" "}
					{t("hub.limits.expires", "(expires {{when}})", {
						when: time.ago(setup.expiresAt),
					})}
				</span>
			))}
			{more > 0
				? ` ${t("hub.limits.more", "and {{count, number}} more", { count: more })}`
				: null}
		</span>
	);
}

function NoLimits() {
	const { t } = useTranslation("devices");
	return (
		<StateView
			kind="unsupported"
			title={t("hub.limits.none.title", "This hub doesn't state its limits.")}
			text={t(
				"hub.limits.none.text",
				"Setup still works: the hub refuses a setup that would go over a limit and says which one.",
			)}
		/>
	);
}

function LoadingLimits() {
	const { t } = useTranslation("devices");
	return (
		<StateView
			kind="loading"
			title={t("hub.limits.loading", "Loading limits")}
		/>
	);
}

/* N5 step 0: three counts against their limits. */

function CompactCount({ used, max }: Readonly<{ used?: number; max: number }>) {
	const { t } = useTranslation("devices");
	return (
		<span className="whitespace-nowrap text-ink-2 tabular-nums">
			{used === undefined
				? t("hub.limits.upTo", "up to {{max, number}}", { max })
				: t("hub.limits.usedOf", "{{used, number}} of {{max, number}}", {
						used,
						max,
					})}
		</span>
	);
}

function CompactRow({
	label,
	used,
	max,
	hint,
}: Readonly<{
	label: string;
	used?: number;
	max: number;
	hint: ReactNode;
}>) {
	const { t } = useTranslation("devices");
	return (
		<div data-limit="" className="flex flex-col gap-1.25">
			<div className="flex items-baseline justify-between gap-2 text-ui">
				<span>{label}</span>
				<CompactCount used={used} max={max} />
			</div>
			{used === undefined ? null : (
				<Meter
					segments={[{ value: share(used, max), tone: limitTone(used, max) }]}
					label={t(
						"hub.limits.meter",
						"{{label}}: {{used, number}} of {{max, number}}",
						{ label, used, max },
					)}
				/>
			)}
			<Hint>{hint}</Hint>
		</div>
	);
}

function DevicesHint({ slots }: Readonly<{ slots?: DeviceSlots }>) {
	const { t } = useTranslation("devices");
	const also = t("hub.limits.devices.also", "Unused setup packages count too.");
	if (!slots) return also;
	if (slots.over)
		return `${t(
			"hub.limits.devices.over",
			"No slots left: {{count, number}} over the limit.",
			{ count: slots.used - slots.max },
		)} ${also}`;
	if (slots.full)
		return `${t("hub.limits.devices.full", "No slots left.")} ${also}`;
	return `${t(
		"hub.limits.devices.left",
		"{{count, number}} left on your account.",
		{ count: slots.max - slots.used },
	)} ${also}`;
}

function PendingHint({ facts }: Readonly<{ facts: LimitFacts }>) {
	const { t } = useTranslation("devices");
	const { slots, limits, waiting } = facts;
	const max = limits.pending;
	if (slots && max !== undefined && slots.pending >= max)
		return t(
			"hub.limits.pending.full",
			"None left. Cancel one in Pending setups, or start one of those devices.",
		);
	if (waiting.length) return <WaitingNames waiting={waiting} />;
	return t(
		"hub.limits.pending.counts",
		"A package counts until a device uses it or it expires.",
	);
}

function DayHint({ facts }: Readonly<{ facts: LimitFacts }>) {
	const { t } = useTranslation("devices");
	const { today, limits } = facts;
	const max = limits.perDay;
	if (today !== undefined && max !== undefined && today >= max)
		return t(
			"hub.limits.day.full",
			"None left today. The next one is possible once the oldest of them is 24 hours old.",
		);
	return t("hub.limits.day.cancelled", "Cancelled packages count too.");
}

function CompactRows({ facts }: Readonly<{ facts: LimitFacts }>) {
	const { t } = useTranslation("devices");
	const { limits, slots, today } = facts;
	const note = usageNote(t, facts);
	return (
		<div className="flex flex-col gap-3">
			{limits.devices === undefined ? null : (
				<CompactRow
					label={t("hub.limits.devices.short", "Devices")}
					used={slots?.used}
					max={limits.devices}
					hint={<DevicesHint slots={slots} />}
				/>
			)}
			{limits.pending === undefined ? null : (
				<CompactRow
					label={t("hub.limits.pending.label", "Unused setup packages")}
					used={slots?.pending}
					max={limits.pending}
					hint={<PendingHint facts={facts} />}
				/>
			)}
			{limits.perDay === undefined ? null : (
				<CompactRow
					label={t("hub.limits.day.short", "Packages in the last 24 h")}
					used={today}
					max={limits.perDay}
					hint={<DayHint facts={facts} />}
				/>
			)}
			{note ? <Hint>{note}</Hint> : null}
		</div>
	);
}

function CompactBody({ facts }: Readonly<{ facts: LimitFacts }>) {
	if (facts.hub.support.state === "checking") return <LoadingLimits />;
	if (!hasLimits(facts.limits)) return <NoLimits />;
	return <CompactRows facts={facts} />;
}

function CompactLimits() {
	const { t } = useTranslation("devices");
	const facts = useLimitFacts();
	return (
		<Block
			icon={Gauge}
			title={t("hub.limits.yours", "Your limits")}
			stamp={facts.stamp}
		>
			<CompactBody facts={facts} />
		</Block>
	);
}

/* N10: every limit with its number, your usage and what it means. */

function Usage({
	used,
	max,
	note,
}: Readonly<{ used: number; max: number; note?: ReactNode }>) {
	const { t } = useTranslation("devices");
	const text = t("hub.limits.you", "You: {{used, number}} of {{max, number}}", {
		used,
		max,
	});
	return (
		<div data-usage="" className="flex flex-col gap-1">
			<Meter
				segments={[{ value: share(used, max), tone: limitTone(used, max) }]}
				label={text}
			/>
			<p className="flex flex-wrap items-center gap-x-1.5 gap-y-0.5 text-xs text-ink-2">
				<span className="tabular-nums">{text}</span>
				{note}
			</p>
		</div>
	);
}

/** The limit as a figure, or "Not stated" when the hub doesn't say. */
function LimitFigure({
	value,
	unit,
}: Readonly<{ value?: number; unit?: ReactNode }>) {
	const { t } = useTranslation("devices");
	const locale = useLocale();
	if (value === undefined)
		return (
			<Figure
				className="text-muted-foreground"
				value={t("hub.limits.notStated", "Not stated")}
			/>
		);
	return (
		<Figure value={new Intl.NumberFormat(locale).format(value)} unit={unit} />
	);
}

function LimitCell({
	label,
	figure,
	tech,
	children,
}: Readonly<{
	label: string;
	figure: ReactNode;
	tech?: string;
	children?: ReactNode;
}>) {
	return (
		<li
			data-limit=""
			className="flex min-w-0 flex-col gap-1.5 bg-card px-4 pt-3 pb-3.5"
		>
			<span className="text-label font-semibold tracking-[0.06em] text-muted-foreground uppercase">
				{label}
			</span>
			<div className="flex items-baseline">
				{figure}
				{tech ? <Tech>{tech}</Tech> : null}
			</div>
			{children}
		</li>
	);
}

function SlotsFullNote({ slots }: Readonly<{ slots: DeviceSlots }>) {
	const { t } = useTranslation("devices");
	return (
		<Hint className={cx("mt-auto pt-0.5", TONE_TEXT.warning)}>
			{slots.over
				? t(
						"hub.limits.devices.overNote",
						"You're using {{used, number}} of {{max, number}} device slots. Devices you have keep working; setting up another one fails until you're under the limit or the hub operator raises it.",
						{ used: slots.used, max: slots.max },
					)
				: t(
						"hub.limits.devices.fullNote",
						"You're using all {{max, number}} device slots. Devices you have keep working; setting up another one fails until one is free or the hub operator raises the limit.",
						{ max: slots.max },
					)}
		</Hint>
	);
}

function SlotsNote({ slots }: Readonly<{ slots: DeviceSlots }>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	if (slots.full) return <SlotsFullNote slots={slots} />;
	if (!slots.near) return null;
	return (
		<Hint className="mt-auto pt-0.5">
			{t("hub.limits.devices.nearNote", {
				used: slots.used,
				max: slots.max,
				count: slots.pending,
				defaultValue_one:
					"You're using {{used, number}} of {{max, number}} device slots, including {{count, number}} unused setup package.",
				defaultValue_other:
					"You're using {{used, number}} of {{max, number}} device slots, including {{count, number}} unused setup packages.",
			})}{" "}
			<a {...link(FLEET)} className={LINK}>
				{t("hub.limits.reviewPending", "Review pending setups")}
			</a>
		</Hint>
	);
}

function DevicesCell({ facts }: Readonly<{ facts: LimitFacts }>) {
	const { t } = useTranslation("devices");
	const { limits, slots } = facts;
	return (
		<LimitCell
			label={t("hub.limits.devices.label", "Devices per account")}
			figure={<LimitFigure value={limits.devices} />}
			tech="max_devices_per_user"
		>
			{slots ? (
				<>
					<Usage
						used={slots.used}
						max={slots.max}
						note={
							<span className="text-muted-foreground">
								{t(
									"hub.limits.devices.both",
									"registered devices and unused setup packages both count",
								)}
							</span>
						}
					/>
					<SlotsNote slots={slots} />
				</>
			) : (
				<Hint>
					{t(
						"hub.limits.devices.bothHint",
						"Registered devices and unused setup packages both count.",
					)}
				</Hint>
			)}
		</LimitCell>
	);
}

function PendingCell({ facts }: Readonly<{ facts: LimitFacts }>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const { limits, slots, waiting } = facts;
	const max = limits.pending;
	return (
		<LimitCell
			label={t("hub.limits.pending.label", "Unused setup packages")}
			figure={<LimitFigure value={max} />}
			tech="max_pending_enrollments_per_user"
		>
			{slots && max !== undefined ? (
				<Usage
					used={slots.pending}
					max={max}
					note={<WaitingNames waiting={waiting} />}
				/>
			) : null}
			<Hint className="mt-auto pt-0.5">
				<Trans
					t={t}
					i18nKey="hub.limits.pending.hint"
					defaults="A package counts until a device uses it or it expires. Cancel unused ones on <1>Fleet overview</1>."
					components={{ 1: <a {...link(FLEET)} className={LINK} /> }}
				/>
			</Hint>
		</LimitCell>
	);
}

/** Whole hours read as hours ("24 hours"), anything else as minutes. */
const lifetimeOf = (seconds: number) => {
	const hours = seconds % 3600 === 0;
	return { amount: hours ? seconds / 3600 : Math.round(seconds / 60), hours };
};

function LifetimeUnit({ seconds }: Readonly<{ seconds: number }>) {
	const { t } = useTranslation("devices");
	const life = lifetimeOf(seconds);
	return life.hours
		? t("hub.limits.lifetime.hours", {
				count: life.amount,
				defaultValue_one: "hour",
				defaultValue_other: "hours",
			})
		: t("hub.limits.lifetime.minutes", {
				count: life.amount,
				defaultValue_one: "minute",
				defaultValue_other: "minutes",
			});
}

function LifetimeCell({ facts }: Readonly<{ facts: LimitFacts }>) {
	const { t } = useTranslation("devices");
	const seconds = facts.limits.lifetimeS;
	return (
		<LimitCell
			label={t("hub.limits.lifetime.label", "Package lifetime")}
			figure={
				seconds === undefined ? (
					<LimitFigure />
				) : (
					<LimitFigure
						value={lifetimeOf(seconds).amount}
						unit={<LifetimeUnit seconds={seconds} />}
					/>
				)
			}
			tech="enrollment_ttl_seconds"
		>
			<Hint className="mt-auto pt-0.5">
				{t(
					"hub.limits.lifetime.hint",
					"A setup package only works for this long after you create it. It can't be downloaded again: if you lose it, cancel the setup and create a new one.",
				)}
			</Hint>
		</LimitCell>
	);
}

function DayCell({ facts }: Readonly<{ facts: LimitFacts }>) {
	const { t } = useTranslation("devices");
	const { limits, today } = facts;
	const max = limits.perDay;
	const formula = limits.devices !== undefined && limits.pending !== undefined;
	return (
		<LimitCell
			label={t("hub.limits.day.label", "Setup packages per day")}
			figure={<LimitFigure value={max} />}
		>
			{today !== undefined && max !== undefined ? (
				<Usage
					used={today}
					max={max}
					note={
						<span className="text-muted-foreground">
							{t(
								"hub.limits.day.counted",
								"in the last 24 hours, cancelled ones too",
							)}
						</span>
					}
				/>
			) : null}
			{formula ? (
				<Hint className="mt-auto pt-0.5">
					{t(
						"hub.limits.day.hint",
						"Twice (devices per account + unused packages): 2 × ({{devices, number}} + {{pending, number}}), counted over any 24 hours.",
						{ devices: limits.devices, pending: limits.pending },
					)}
				</Hint>
			) : null}
		</LimitCell>
	);
}

function LimitsGrid({ facts }: Readonly<{ facts: LimitFacts }>) {
	const { t } = useTranslation("devices");
	return (
		<>
			{facts.hub.support.state === "off" ? (
				<Hint className="px-4 pt-3">
					{t(
						"hub.limits.off",
						"These limits apply once the hub operator turns device support on.",
					)}
				</Hint>
			) : null}
			<ul className="grid grid-cols-4 gap-px bg-hairline @max-[1080px]/devices:grid-cols-2 @max-[560px]/devices:grid-cols-1">
				<DevicesCell facts={facts} />
				<PendingCell facts={facts} />
				<LifetimeCell facts={facts} />
				<DayCell facts={facts} />
			</ul>
		</>
	);
}

function FullBody({ facts }: Readonly<{ facts: LimitFacts }>) {
	const { t } = useTranslation("devices");
	const state = facts.hub.support.state;
	if (hasLimits(facts.limits) && state !== "checking")
		return <LimitsGrid facts={facts} />;
	return (
		<div className="px-4 py-3">
			{state === "checking" ? (
				<LoadingLimits />
			) : state === "unreachable" ? (
				<StateView
					kind="notloaded"
					title={t(
						"hub.limits.notLoaded",
						"The limits haven't been read: the hub didn't answer.",
					)}
				/>
			) : (
				<NoLimits />
			)}
		</div>
	);
}

function FullLimits() {
	const { t } = useTranslation("devices");
	const facts = useLimitFacts();
	return (
		<Block
			id="limits"
			icon={Gauge}
			title={t("hub.limits.title", "Limits")}
			stamp={facts.stamp}
			flush
			className="scroll-mt-4"
			foot={
				<span>
					{t(
						"hub.limits.foot",
						"Per account on this hub. Only the hub operator can change them.",
					)}{" "}
					{usageNote(t, facts)}
				</span>
			}
		>
			<FullBody facts={facts} />
		</Block>
	);
}

/** Hub limits with your usage (BG3). `compact` is the setup wizard's "Your limits" block. */
export function HubLimitsUsage({
	compact = false,
}: Readonly<{ compact?: boolean }>) {
	return compact ? <CompactLimits /> : <FullLimits />;
}
