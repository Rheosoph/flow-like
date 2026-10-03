"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	Info,
	LockOpen,
	LogIn,
	type LucideIcon,
	RefreshCw,
	Rocket,
	Server,
	ServerOff,
} from "lucide-react";
import { type ReactNode, useId } from "react";
import { appCopy } from "../copy/app-copy";
import { headlinePartCopy } from "../copy/headline-copy";
import { ModeChip } from "../primitives/app-chips";
import type { DevicesT } from "../primitives/area-context";
import { Banner } from "../primitives/banner";
import { DvButton } from "../primitives/dv-button";
import {
	FreshnessStamp,
	MixedSourcesStamp,
} from "../primitives/freshness-stamp";
import { cx } from "../primitives/tone";
import { ACCOUNT_SCOPE } from "../routing/devices-route";
import { stampOf } from "../shell/attention-popover";
import { hubErrorCopy, useOverlayStore } from "../workspace";
import { blockShort } from "./events-copy";
import {
	type EventsDevicesBlock,
	type EventsDevicesLive,
	type EventsDevicesValue,
	useEventsDevices,
} from "./events-devices";

/* APP §4.2 and §4.5: the "On devices" strip and the one banner that says why Run on a device… is off. */

/** Id of the visible block reason, for `aria-describedby` on every gated control of the page. */
export const EVENTS_BLOCK_ID = "events-devices-block";

interface BannerCopy {
	icon: LucideIcon;
	title: string;
	text: string;
}

function bannerCopy(
	t: DevicesT,
	block: Exclude<EventsDevicesBlock, "blind">,
	facts: { reason: string; noDevices: boolean },
): BannerCopy {
	const { reason, noDevices } = facts;
	const unavailable = t(
		"devices:events.block.unavailable",
		"Run on a device… isn't available",
	);
	const copy = {
		signed_out: {
			icon: LogIn,
			title: unavailable,
			text: t(
				"devices:events.block.signedOut.text",
				"Devices belong to your Flow-Like account. Sign in to see where these events run and to deploy them.",
			),
		},
		token: {
			icon: LogIn,
			title: unavailable,
			text: t(
				"devices:events.block.token.text",
				"Your access token can't list or manage devices. Sign in with full permissions to see where these events run.",
			),
		},
		hub_off: {
			icon: ServerOff,
			title: t(
				"devices:events.block.hubOff.title",
				"Device support is off on this hub",
			),
			text: t(
				"devices:events.block.hubOff.text",
				"This hub doesn't have device support turned on, so this page can't check which devices run these events, and Run on a device… is off. Events keep working in Flow-Like as before. Ask the hub operator to turn device support on.",
			),
		},
		error: {
			icon: ServerOff,
			title: t(
				"devices:events.block.error.title",
				"Devices couldn't be checked",
			),
			text: t(
				"devices:events.block.error.text",
				"{{reason}} This page can't tell which devices run these events right now, and Run on a device… is off. Events keep working in Flow-Like as before.",
				{ reason },
			).trim(),
		},
		no_target: {
			icon: Info,
			title: unavailable,
			text: noDevices
				? t(
						"devices:events.block.noDevices.text",
						"You have no device yet. Set one up in Devices, then run these events on it.",
					)
				: t(
						"devices:events.block.noTarget.text",
						"You have no device that can take a deploy right now: each one is offline, revoked, locked to another app or has no keys on this computer.",
					),
		},
	} satisfies Record<Exclude<EventsDevicesBlock, "blind">, BannerCopy>;
	return copy[block];
}

/** The way out of a block, when there is one: sign in, the hub's status, another try, or the fleet. */
function blockAction(
	t: DevicesT,
	block: EventsDevicesBlock,
	{ problem, signIn, link }: EventsDevicesValue,
): ReactNode {
	if (block === "signed_out" || block === "token")
		return signIn ? (
			<DvButton size="sm" icon={LogIn} onClick={signIn}>
				{t("devices:events.block.signIn", "Sign in")}
			</DvButton>
		) : undefined;
	if (block === "hub_off")
		return (
			<DvButton size="sm" asChild>
				<a {...link({ screen: "hub" }, ACCOUNT_SCOPE)}>
					{t("devices:events.block.openHub", "Open hub status")}
				</a>
			</DvButton>
		);
	if (block === "error")
		return problem?.retry ? (
			<DvButton size="sm" icon={RefreshCw} onClick={problem.retry}>
				{t("devices:events.block.retry", "Retry")}
			</DvButton>
		) : undefined;
	return (
		<DvButton size="sm" asChild>
			<a {...link({ screen: "fleet", view: "devices" }, ACCOUNT_SCOPE)}>
				{t("devices:events.block.openDevices", "Open devices")}
			</a>
		</DvButton>
	);
}

/**
 * The first cause that turns Run on a device… off for every row, once above
 * the sections. A role that can't read flows is told by the page's own
 * "Flow names unavailable" notice, which carries `EVENTS_BLOCK_ID` then.
 */
export function EventsDevicesBanner({
	className,
}: Readonly<{ className?: string }>) {
	const { t } = useTranslation("devices");
	const devices = useEventsDevices();
	const { block, problem, live } = devices;
	if (!block || block === "blind") return null;
	const copy = bannerCopy(t, block, {
		reason: problem?.code ? hubErrorCopy(t, problem.code) : "",
		noDevices: live?.view.layout === "no_devices",
	});
	return (
		<div id={EVENTS_BLOCK_ID} data-events-block={block} className={className}>
			<Banner
				tone="info"
				icon={copy.icon}
				title={copy.title}
				actions={blockAction(t, block, devices)}
			>
				{copy.text}
			</Banner>
		</div>
	);
}

const STRIP =
	"flex min-w-0 flex-wrap items-center gap-x-4 gap-y-2.5 rounded-lg border border-border bg-surface-sunken px-3 py-2.5 text-ui";
const FACT = "text-xs/[17px] text-muted-foreground";
const NUMBER = <b className="font-semibold text-foreground" />;

function StripTitle() {
	const { t } = useTranslation("devices");
	return (
		<span className="inline-flex items-center gap-1.5 font-semibold">
			<Server aria-hidden className="size-4 text-muted-foreground" />
			{t("events.strip.title", "On devices")}
		</span>
	);
}

/** "3 of 5 events can run on a device. 1 of them runs on 1 device you can see." */
function CountLine({
	id,
	live,
}: Readonly<{ id: string; live: EventsDevicesLive }>) {
	const { t } = useTranslation("devices");
	const { view, rows, coverage } = live;
	const eligible = view.events.rows.length;
	const total = eligible + view.events.ineligible.length;
	// "Of them": only events that can run; one that can't and is still served says so in its own cell.
	const served = view.events.rows.filter(
		(row) => rows.get(row.eventId)?.served.length,
	).length;
	const devices = new Set(view.services.map((row) => row.deviceId)).size;
	const unknown = coverage.unknown.length - coverage.never.length;
	return (
		<p id={id} className={FACT}>
			<Trans
				t={t}
				i18nKey="events.strip.canRun"
				count={total}
				values={{ eligible }}
				tOptions={{
					defaultValue_one:
						"<1>{{eligible, number}}</1> of {{count, number}} event can run on a device.",
					defaultValue_other:
						"<1>{{eligible, number}}</1> of {{count, number}} events can run on a device.",
				}}
				components={{ 1: NUMBER }}
			/>
			{eligible === 0 ? null : served ? (
				<>
					{" "}
					<Trans
						t={t}
						i18nKey="events.strip.runsOn"
						count={served}
						values={{
							devices: t("events.strip.devices", {
								count: devices,
								defaultValue_one: "{{count, number}} device",
								defaultValue_other: "{{count, number}} devices",
							}),
						}}
						tOptions={{
							defaultValue_one:
								"<1>{{count, number}}</1> of them runs on {{devices}} you can see.",
							defaultValue_other:
								"<1>{{count, number}}</1> of them run on {{devices}} you can see.",
						}}
						components={{ 1: NUMBER }}
					/>
				</>
			) : (
				<>
					{" "}
					{unknown > 0
						? t(
								"events.strip.noneVisible",
								"None of them is on a device you can see yet.",
							)
						: t("events.strip.none", "None of them is on a device yet.")}
				</>
			)}
			{view.app.localOnly && eligible > 0 ? (
				<>
					{" "}
					{t(
						"events.strip.recheck",
						"Each device checks them again when the copy arrives.",
					)}
				</>
			) : null}
		</p>
	);
}

function UnlockAction({ live }: Readonly<{ live: EventsDevicesLive }>) {
	const { t } = useTranslation("devices");
	const { coverage, names } = live;
	const locked = coverage.locked.filter((id) => !coverage.never.includes(id));
	const [only] = locked;
	if (!only) return null;
	const overlay = useOverlayStore.getState();
	return locked.length === 1 ? (
		<DvButton
			size="xs"
			icon={LockOpen}
			onClick={() => overlay.openUnlock(only)}
		>
			{t("events.strip.unlockOne", "Unlock {{device}}…", {
				device: names.get(only) ?? only,
			})}
		</DvButton>
	) : (
		<DvButton size="xs" icon={LockOpen} onClick={overlay.openUnlockSeveral}>
			{t("events.strip.unlockSeveral", "Unlock {{count, number}}…", {
				count: locked.length,
			})}
		</DvButton>
	);
}

function DeployAction({
	live,
	countId,
}: Readonly<{ live: EventsDevicesLive; countId: string }>) {
	const { t } = useTranslation("devices");
	const { block, link } = useEventsDevices();
	const label = t("events.strip.deploy", "Deploy to devices…");
	const app = live.view.app.name;
	const off = block
		? { by: EVENTS_BLOCK_ID, why: blockShort(t, block) }
		: live.view.events.rows.length
			? null
			: {
					by: countId,
					why: t(
						"events.strip.noneCanRun",
						"None of {{app}}'s events can run on a device",
						{ app },
					),
				};
	if (off)
		return (
			<DvButton
				size="xs"
				icon={Rocket}
				aria-disabled
				aria-describedby={off.by}
				title={off.why}
				data-gated=""
			>
				{label}
			</DvButton>
		);
	return (
		<DvButton size="xs" icon={Rocket} asChild>
			<a
				{...link({ screen: "deploy", deviceIds: [], mode: "new" })}
				title={t(
					"events.strip.deployTitle",
					"Opens deploy with every event of {{app}} that can run on a device; schedules and bots start unticked. You pick one or more devices next.",
					{ app },
				)}
			>
				{label}
			</a>
		</DvButton>
	);
}

/**
 * APP §4.2: how this app runs on devices and how many of its events do, with
 * the coverage sentence App › Devices shows. Rendered only while devices can
 * be read: signed in, device support on, a role that reads flows.
 */
export function OnDevicesStrip({
	className,
}: Readonly<{ className?: string }>) {
	const { t } = useTranslation("devices");
	const { status, live, link, explainMode } = useEventsDevices();
	const countId = useId();
	if (status === "loading")
		return (
			<section
				aria-busy="true"
				data-on-devices="loading"
				className={cx(STRIP, className)}
			>
				<StripTitle />
				<span className={FACT}>
					{t("events.cell.checking", "Checking devices…")}
				</span>
			</section>
		);
	if (status !== "ready" || !live) return null;
	const { view, coverage } = live;
	const copy = appCopy(t);
	const { name, mode } = view.app;
	return (
		<section
			aria-label={t("events.strip.label", "{{app}} on devices", { app: name })}
			data-on-devices={mode}
			className={cx(STRIP, className)}
		>
			<p className="-mb-1.5 flex min-w-0 basis-full flex-wrap items-center gap-x-2.5 gap-y-1 text-ui">
				<StripTitle />
				<ModeChip mode={mode} app={name} />
				<span className="min-w-0 text-ink-2">
					{copy.mode(mode, name).sentence}
				</span>
				<DvButton variant="link" size="xs" onClick={explainMode}>
					{copy.explainButton()}
				</DvButton>
			</p>
			<div className="flex min-w-0 flex-[1_1_420px] flex-col gap-1">
				<CountLine id={countId} live={live} />
				<p className={FACT}>
					{headlinePartCopy(t, {
						code: "app.coverage",
						params: {
							app: name,
							readable: coverage.readable,
							total: coverage.total,
							unknown: coverage.unknown.length - coverage.never.length,
							never: coverage.never.length,
							noAccess: coverage.noAccess.length,
						},
					})}{" "}
					{live.hub.age === "error" ? (
						<FreshnessStamp {...stampOf(live.hub)} />
					) : (
						<MixedSourcesStamp />
					)}
				</p>
			</div>
			<div className="flex min-w-0 flex-wrap items-center gap-1.5">
				<UnlockAction live={live} />
				<DeployAction live={live} countId={countId} />
				<DvButton size="xs" icon={Server} asChild>
					<a {...link({ screen: "app-devices", by: "device" })}>
						{t("events.openInDevices", "Open in Devices")}
					</a>
				</DvButton>
			</div>
		</section>
	);
}
