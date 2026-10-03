"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { Fragment, type ReactElement, type ReactNode } from "react";
import type { AppServiceRow } from "../../../../lib/device-management/model/app-plan";
import { keysLocked } from "../../../../lib/device-management/model/device-view";
import type { ServiceSummary } from "../../../../lib/device-management/workspace/types";
import { formatMoney } from "../copy/attention-copy";
import {
	type AreaTime,
	type DevicesT,
	useAreaTime,
} from "../primitives/area-context";
import { Headline } from "../primitives/headline";
import { attentionShort } from "../shell/rail-row";
import { type AppViewRead, useResourceSummary } from "../workspace";
import {
	type DevicePage,
	servicesOfApp,
	usePersonName,
} from "./use-device-page";

/*
 * Names go into sentences as component children (`<1/>`), never as
 * interpolated values: <Trans> parses interpolated text as markup, so a
 * display name with "<" in it would be mangled.
 */
function mono(name: string) {
	return <span className="font-mono text-[0.94em]">{name}</span>;
}

function plain(name: string) {
	return <span>{name}</span>;
}

interface Verdict {
	lead: ReactNode;
	rest: ReactNode[];
}

interface VerdictInput {
	t: DevicesT;
	time: AreaTime;
	page: DevicePage;
	/** Readable rows, or the rows kept from before the lock. */
	rows: readonly ServiceSummary[] | null;
	lockedData: boolean;
}

const AS_ASKED = new Set<ServiceSummary["conv"]>([
	"converged",
	"stopped_by_user",
]);

function counted(page: DevicePage, scoped = false) {
	const items = scoped ? page.appAttention : page.attention;
	return items.filter((item) => item.severity !== "info");
}

/** "a, b and c" in mono, for a short list of service names. */
function monoList(time: AreaTime, values: readonly string[]): ReactElement {
	const parts = new Intl.ListFormat(time.locale, {
		type: "conjunction",
	}).formatToParts(values);
	return (
		<>
			{parts.map((part, index) =>
				part.type === "element" ? (
					// biome-ignore lint/suspicious/noArrayIndexKey: list parts are positional
					<Fragment key={index}>{mono(part.value)}</Fragment>
				) : (
					part.value
				),
			)}
		</>
	);
}

/** Sentences about single services: the one switching over and the ones that keep crashing. */
function serviceBits(
	{ t, time, page }: VerdictInput,
	rows: readonly ServiceSummary[],
	skipCrash?: string,
): ReactNode[] {
	const bits: ReactNode[] = [];
	const moving = rows.find((row) => row.conv === "update_in_progress");
	const rollout = moving
		? page.services?.find((row) => row.serviceId === moving.serviceId)?.rollout
		: undefined;
	if (moving && rollout?.created_at !== undefined)
		bits.push(
			<Trans
				t={t}
				i18nKey="devices:device.verdict.switching"
				defaults="<1/> is switching to new settings; it started {{ago}}."
				values={{ ago: time.ago(rollout.created_at, "long") }}
				components={{ 1: mono(moving.serviceId) }}
			/>,
		);
	else if (moving)
		bits.push(
			<Trans
				t={t}
				i18nKey="devices:device.verdict.switchingNoTime"
				defaults="<1/> is switching to new settings."
				components={{ 1: mono(moving.serviceId) }}
			/>,
		);
	for (const row of rows)
		if (row.conv === "crash_looping" && row.serviceId !== skipCrash)
			bits.push(
				<Trans
					t={t}
					i18nKey="devices:device.verdict.crashing"
					defaults="<1/> keeps crashing."
					components={{ 1: mono(row.serviceId) }}
				/>,
			);
	return bits;
}

function revokedVerdict(
	{ t, time, page }: VerdictInput,
	ownerName: string | undefined,
	billing: { used: number; limit: number } | "closed" | "unknown",
): Verdict {
	const { row, keys } = page.view;
	if (page.consentOnly) {
		const lead = ownerName
			? t("devices:device.verdict.revokedBy", "Revoked by {{owner}}.", {
					owner: ownerName,
				})
			: t("devices:device.verdict.revokedByOwner", "Revoked by its owner.");
		const rest =
			billing === "unknown"
				? t(
						"devices:device.verdict.consentOnly",
						"You see it only because you approve or pay for its cloud access.",
					)
				: billing === "closed"
					? t(
							"devices:device.verdict.limitClosed",
							"Your spending limit on it is closed.",
						)
					: t(
							"devices:device.verdict.stillPay",
							"You still pay for its spending limit: {{used}} of {{limit}} used.",
							{
								used: formatMoney(billing.used, time.locale),
								limit: formatMoney(billing.limit, time.locale),
							},
						);
		return { lead, rest: [rest] };
	}
	const rest: ReactNode[] = [
		keys.state === "none"
			? t(
					"devices:device.verdict.revokedNoKeys",
					"The hub refuses this device; no keys for it are stored here.",
				)
			: t(
					"devices:device.verdict.revokedKeys",
					"The hub refuses this device; its keys are still stored on this computer.",
				),
	];
	if (row.last_seen_at !== null)
		rest.push(
			t("devices:device.verdict.lastCheckIn", "Last check-in {{time}}.", {
				time: time.at(row.last_seen_at),
			}),
		);
	return {
		lead: row.revoked_at
			? t("devices:device.verdict.revokedAgo", "Revoked {{ago}}.", {
					ago: time.ago(row.revoked_at, "long"),
				})
			: t("devices:device.verdict.revoked", "Revoked."),
		rest,
	};
}

function neverVerdict({ t, time, page }: VerdictInput): Verdict {
	const onlyHere = page.attention.some(
		(item) => item.key === "keys_not_backed_up_to_account",
	);
	return {
		lead: t(
			"devices:device.verdict.never",
			"Registered {{ago}} and hasn't checked in yet.",
			{ ago: time.ago(page.view.row.registered_at, "long") },
		),
		rest: onlyHere
			? [
					t(
						"devices:device.verdict.keysOnlyHere",
						"Its keys exist only on this computer.",
					),
				]
			: [],
	};
}

function offlineLead({ t, time, page }: VerdictInput): ReactNode {
	const since = page.view.presence.since;
	if (since === undefined)
		return (
			<Trans
				t={t}
				i18nKey="devices:device.verdict.offline"
				defaults="<1/> is offline."
				components={{ 1: mono(page.name) }}
			/>
		);
	return (
		<Trans
			t={t}
			i18nKey="devices:device.verdict.offlineSince"
			defaults="<1/> has been offline since {{time}} ({{ago}})."
			values={{ time: time.at(since), ago: time.ago(since, "long") }}
			components={{ 1: mono(page.name) }}
		/>
	);
}

function offlineVerdict(input: VerdictInput): Verdict {
	const { t, time, page, rows } = input;
	const lead = offlineLead(input);
	const since = page.view.presence.since;
	if (!rows || since === undefined) return { lead, rest: [] };
	const crashing = rows
		.filter((row) => row.conv === "crash_looping")
		.map((row) => row.serviceId);
	const expired = page.attention.some(
		(item) => item.key === "certificate_expired",
	);
	const when = time.at(since);
	if (!crashing.length && !expired)
		return {
			lead,
			rest: [
				t(
					"devices:device.verdict.lastReportedOk",
					"When it last reported ({{time}}), its services ran as requested.",
					{ time: when },
				),
			],
		};
	const rest: ReactNode[] = [];
	if (crashing.length)
		rest.push(
			<Trans
				t={t}
				i18nKey="devices:device.verdict.lastReportedCrash"
				defaults="When it last reported ({{time}}), <1/> kept crashing."
				values={{ time: when }}
				components={{ 1: monoList(time, crashing) }}
			/>,
		);
	if (expired)
		rest.push(
			crashing.length
				? t(
						"devices:device.verdict.certExpiredToo",
						"A certificate had expired too.",
					)
				: t(
						"devices:device.verdict.lastReportedCert",
						"When it last reported ({{time}}), a certificate had expired.",
						{ time: when },
					),
		);
	return { lead, rest };
}

function hubOnlyVerdict(
	{ t, page }: VerdictInput,
	scopedApp?: string,
): Verdict {
	const open = counted(page).length;
	const lead = open
		? t("devices:device.verdict.hubItems", {
				count: open,
				defaultValue_one:
					"{{count, number}} item needs you, as far as the hub knows.",
				defaultValue_other:
					"{{count, number}} items need you, as far as the hub knows.",
			})
		: t(
				"devices:device.verdict.hubHealthy",
				"Healthy as far as the hub knows.",
			);
	if (!keysLocked(page.view.keys))
		return {
			lead,
			rest: [
				t(
					"devices:device.verdict.noKeys",
					"This computer has no keys for it, so its services can't be read here.",
				),
			],
		};
	return {
		lead,
		rest: [
			scopedApp
				? t(
						"devices:device.verdict.unlockApp",
						"Unlock to see your services in {{app}}.",
						{ app: scopedApp },
					)
				: t("devices:device.verdict.unlock", "Unlock to see your services."),
		],
	};
}

function servicesLead(
	{ t, lockedData }: VerdictInput,
	rows: readonly ServiceSummary[],
): string {
	const ok = rows.filter((row) => AS_ASKED.has(row.conv)).length;
	if (!rows.length)
		return lockedData
			? t(
					"devices:device.verdict.noneLocked",
					"No services ran here when last read.",
				)
			: t("devices:device.verdict.none", "No services run here yet.");
	if (ok === rows.length && rows.length === 1)
		return lockedData
			? t(
					"devices:device.verdict.oneLocked",
					"Its service was as you asked when last read.",
				)
			: t("devices:device.verdict.one", "Its service is as you asked.");
	if (ok === rows.length)
		return lockedData
			? t(
					"devices:device.verdict.allLocked",
					"All {{count, number}} services were as you asked when last read.",
					{ count: rows.length },
				)
			: t(
					"devices:device.verdict.all",
					"All {{count, number}} services are as you asked.",
					{ count: rows.length },
				);
	return lockedData
		? t(
				"devices:device.verdict.someLocked",
				"{{ok, number}} of {{count, number}} services were as you asked when last read.",
				{ ok, count: rows.length },
			)
		: t(
				"devices:device.verdict.some",
				"{{ok, number}} of {{count, number}} services are as you asked.",
				{ ok, count: rows.length },
			);
}

function wholeVerdict(input: VerdictInput, scopedApp?: string): Verdict {
	const { t, page, rows } = input;
	if (page.view.presence.kind === "never") return neverVerdict(input);
	if (page.view.presence.kind === "offline") return offlineVerdict(input);
	if (!rows) return hubOnlyVerdict(input, scopedApp);
	const rest = serviceBits(input, rows);
	const open = counted(page);
	const first = open[0];
	if (first)
		rest.push(
			t("devices:device.verdict.needYou", {
				count: open.length,
				urgent: attentionShort(t, first.key).toLowerCase(),
				defaultValue_one:
					"{{count, number}} item needs you; the most urgent: {{urgent}}.",
				defaultValue_other:
					"{{count, number}} items need you; the most urgent: {{urgent}}.",
			}),
		);
	return { lead: servicesLead(input, rows), rest };
}

/* App context (APP §1.10): the verdict answers "does this app run here?". */

interface AppVerdictInput extends VerdictInput {
	app: { id: string; name: string };
	read: AppViewRead | null;
}

/** Where a counted sentence names the device: translators keep the mark, the name is put in its place. */
const DEVICE_MARK = "<1/>";

function elsewhere({ t, page }: AppVerdictInput): ReactNode | null {
	const extra = counted(page).length - counted(page, true).length;
	if (extra <= 0) return null;
	// A plain `t()` with plural forms: <Trans> has one default and would read "1 items".
	const [before, ...after] = t("devices:device.verdict.app.elsewhere", {
		count: extra,
		defaultValue_one: "Elsewhere on <1/>, {{count, number}} item needs you.",
		defaultValue_other: "Elsewhere on <1/>, {{count, number}} items need you.",
	}).split(DEVICE_MARK);
	return (
		<>
			{before}
			{mono(page.name)}
			{after.join(DEVICE_MARK)}
		</>
	);
}

function appNotRunning(input: AppVerdictInput): Verdict {
	const { t, page, app, read, lockedData } = input;
	const view = read?.view;
	const rest: ReactNode[] = [
		view?.app.mode === "offline"
			? t(
					"devices:device.verdict.app.wouldOffline",
					"It's a local-only app, so a service here would get an offline copy from this computer.",
				)
			: t(
					"devices:device.verdict.app.wouldOnline",
					"It's an online app, so a service here would run it online with its data in the cloud.",
				),
	];
	if (view?.app.canReadFlows)
		rest.push(
			t(
				"devices:device.verdict.app.events",
				"{{eligible, number}} of its {{total, number}} events can run on a device.",
				{
					eligible: view.events.rows.length,
					total: view.events.rows.length + view.events.ineligible.length,
				},
			),
		);
	const more = elsewhere(input);
	if (more) rest.push(more);
	const names = { 1: plain(app.name), 2: mono(page.name) };
	return {
		lead: lockedData ? (
			<Trans
				t={t}
				i18nKey="devices:device.verdict.app.notHereLocked"
				defaults="<1/> didn't run on <2/> when last read."
				components={names}
			/>
		) : (
			<Trans
				t={t}
				i18nKey="devices:device.verdict.app.notHere"
				defaults="<1/> doesn't run on <2/> yet."
				components={names}
			/>
		),
		rest,
	};
}

/** The app's only service here isn't as asked: say what it is doing instead. */
function singleLead(
	{ t, app }: AppVerdictInput,
	one: ServiceSummary,
): ReactNode {
	if (one.conv === "update_in_progress")
		return t(
			"devices:device.verdict.app.updating",
			"{{app}} is updating here.",
			{ app: app.name },
		);
	if (one.conv === "crash_looping")
		return (
			<Trans
				t={t}
				i18nKey="devices:device.verdict.app.crashing"
				defaults="<1/> isn't running here: <2/> keeps crashing."
				components={{ 1: plain(app.name), 2: mono(one.serviceId) }}
			/>
		);
	return t(
		"devices:device.verdict.app.notAsAsked",
		"{{app}} isn't running here as you asked yet.",
		{ app: app.name },
	);
}

function allAsAskedLead(
	{ t, app, lockedData }: AppVerdictInput,
	count: number,
): string {
	if (count === 1)
		return lockedData
			? t(
					"devices:device.verdict.app.oneLocked",
					"{{app}} ran here as you asked when last read.",
					{ app: app.name },
				)
			: t("devices:device.verdict.app.one", "{{app}} runs here as you asked.", {
					app: app.name,
				});
	return lockedData
		? t(
				"devices:device.verdict.app.allLocked",
				"All {{count, number}} {{app}} services here were as you asked when last read.",
				{ count, app: app.name },
			)
		: t(
				"devices:device.verdict.app.all",
				"All {{count, number}} {{app}} services here are as you asked.",
				{ count, app: app.name },
			);
}

function appLead(
	input: AppVerdictInput,
	mine: readonly ServiceSummary[],
): ReactNode {
	const { t, app, lockedData } = input;
	const ok = mine.filter((row) => AS_ASKED.has(row.conv)).length;
	const one = mine.length === 1 ? mine[0] : undefined;
	if (one && ok === 0) return singleLead(input, one);
	if (ok === mine.length) return allAsAskedLead(input, mine.length);
	return lockedData
		? t(
				"devices:device.verdict.app.someLocked",
				"{{ok, number}} of {{count, number}} {{app}} services here were as you asked when last read.",
				{ ok, count: mine.length, app: app.name },
			)
		: t(
				"devices:device.verdict.app.some",
				"{{ok, number}} of {{count, number}} {{app}} services here are as you asked.",
				{ ok, count: mine.length, app: app.name },
			);
}

/** "nightly-sync is stopped, as you asked." for each service the viewer stopped. */
function stoppedBits(
	{ t }: AppVerdictInput,
	mine: readonly ServiceSummary[],
): ReactNode[] {
	return mine
		.filter(
			(row) => row.desired === "stopped" && row.conv === "stopped_by_user",
		)
		.map((row) => (
			<Trans
				key={row.serviceId}
				t={t}
				i18nKey="devices:device.verdict.app.stopped"
				defaults="<1/> is stopped, as you asked."
				components={{ 1: mono(row.serviceId) }}
			/>
		));
}

/** "invoice-extractor runs v1.4.0, 1 behind v1.5.0."; an older version without a name is just "an older version". */
function behindSentence(
	t: AppVerdictInput["t"],
	row: AppServiceRow,
	newest: string,
): ReactNode {
	const behind = row.behind ?? 0;
	const label = row.version?.label;
	if (!label)
		return (
			<Trans
				key={row.serviceId}
				t={t}
				i18nKey="devices:device.verdict.app.behindUnnamed"
				defaults="<1/> runs an older version, {{behind, number}} behind <3/>."
				values={{ behind }}
				components={{ 1: mono(row.serviceId), 3: mono(newest) }}
			/>
		);
	return (
		<Trans
			key={row.serviceId}
			t={t}
			i18nKey="devices:device.verdict.app.behind"
			defaults="<1/> runs <2/>, {{behind, number}} behind <3/>."
			values={{ behind }}
			components={{
				1: mono(row.serviceId),
				2: mono(label),
				3: mono(newest),
			}}
		/>
	);
}

/** A service of this device that runs an older version of the app's list (`behind` counts versions, 0 = newest). */
function runsOlder(row: AppServiceRow, deviceId: string): boolean {
	return row.deviceId === deviceId && !!row.version && !!row.behind;
}

/** One sentence for each service of this device that runs an older app version. */
function behindBits({ t, page, read }: AppVerdictInput): ReactNode[] {
	const newest = read?.view?.howRuns.newest?.label;
	if (!read?.view || !newest) return [];
	return read.view.services
		.filter((row) => runsOlder(row, page.deviceId))
		.map((row) => behindSentence(t, row, newest));
}

/** The service the lead sentence already calls crashing, so the rest doesn't repeat it. */
function leadCrashOf(mine: readonly ServiceSummary[]): string | undefined {
	const [one] = mine;
	return mine.length === 1 && one?.conv === "crash_looping"
		? one.serviceId
		: undefined;
}

function appRunning(
	input: AppVerdictInput,
	mine: readonly ServiceSummary[],
): Verdict {
	const { t, page, app } = input;
	const rest = [
		...serviceBits(input, mine, leadCrashOf(mine)),
		...stoppedBits(input, mine),
		...behindBits(input),
	];
	const about = counted(page, true).length;
	if (about)
		rest.push(
			t("devices:device.verdict.app.needYou", {
				count: about,
				app: app.name,
				defaultValue_one: "{{count, number}} item about {{app}} needs you.",
				defaultValue_other: "{{count, number}} items about {{app}} need you.",
			}),
		);
	const more = elsewhere(input);
	if (more) rest.push(more);
	return { lead: appLead(input, mine), rest };
}

function appOffline(
	input: AppVerdictInput,
	mine: readonly ServiceSummary[],
): Verdict {
	const { t, time, page, app } = input;
	const lead = offlineLead(input);
	const since = page.view.presence.since;
	if (since === undefined) return { lead, rest: [] };
	const when = time.at(since);
	const crashing = mine
		.filter((row) => row.conv === "crash_looping")
		.map((row) => row.serviceId);
	if (crashing.length)
		return {
			lead,
			rest: [
				<Trans
					key="crash"
					t={t}
					i18nKey="devices:device.verdict.lastReportedCrash"
					defaults="When it last reported ({{time}}), <1/> kept crashing."
					values={{ time: when }}
					components={{ 1: monoList(time, crashing) }}
				/>,
			],
		};
	return {
		lead,
		rest: [
			mine.length
				? t(
						"devices:device.verdict.app.lastReportedOk",
						"When it last reported ({{time}}), {{app}} ran as you asked.",
						{ time: when, app: app.name },
					)
				: t(
						"devices:device.verdict.app.lastReportedNone",
						"When it last reported ({{time}}), no {{app}} service ran there.",
						{ time: when, app: app.name },
					),
		],
	};
}

function appVerdict(input: AppVerdictInput): Verdict {
	const { t, page, app, rows } = input;
	if (page.view.presence.kind === "never") return neverVerdict(input);
	if (page.view.presence.kind === "offline")
		return rows
			? appOffline(input, servicesOfApp(rows, app.id))
			: offlineVerdict(input);
	if (!rows)
		return {
			lead: t(
				"devices:device.verdict.hubHealthy",
				"Healthy as far as the hub knows.",
			),
			rest: [
				keysLocked(page.view.keys)
					? t(
							"devices:device.verdict.app.unlock",
							"Unlock to see whether {{app}} runs here. The hub can't tell.",
							{ app: app.name },
						)
					: t(
							"devices:device.verdict.app.noKeys",
							"This computer has no keys for it, so you can't see whether {{app}} runs here.",
							{ app: app.name },
						),
			],
		};
	const mine = servicesOfApp(rows, app.id);
	return mine.length ? appRunning(input, mine) : appNotRunning(input);
}

function billingOf(
	summary: ReturnType<typeof useResourceSummary>,
	deviceId: string,
): { used: number; limit: number } | "closed" | "unknown" {
	if (!summary.data) return "unknown";
	const mine = (
		summary.data.devices.find((device) => device.device_id === deviceId)
			?.billing ?? []
	).filter((row) => row.payer_is_me);
	if (!mine.length) return "closed";
	return {
		used: mine.reduce((sum, row) => sum + row.used_micros, 0),
		limit: mine.reduce((sum, row) => sum + row.limit_micros, 0),
	};
}

function Sentences({ parts }: Readonly<{ parts: readonly ReactNode[] }>) {
	return (
		<>
			{parts.map((part, index) => (
				// biome-ignore lint/suspicious/noArrayIndexKey: sentences are positional
				<Fragment key={index}>
					{index ? " " : null}
					{part}
				</Fragment>
			))}
		</>
	);
}

/** SPEC §6.4 N2 verdict; in an app it answers for that app (APP §1.10). */
export function DeviceVerdict({
	page,
	app,
	scopedAppName,
}: Readonly<{
	page: DevicePage;
	app: AppViewRead | null;
	/** The one app a scoped viewer can see, for "Unlock to see your services in {App}". */
	scopedAppName?: string;
}>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const summary = useResourceSummary();
	const ownerName = usePersonName(page.view.row.owner_id, page.consentOnly);
	const input = verdictInput(t, time, page);
	const verdict = page.revoked
		? revokedVerdict(input, ownerName, billingOf(summary, page.deviceId))
		: page.app
			? appVerdict({ ...input, app: page.app, read: app })
			: wholeVerdict(input, scopedAppName);
	return (
		<Headline
			lead={verdict.lead}
			rest={
				verdict.rest.length ? <Sentences parts={verdict.rest} /> : undefined
			}
		/>
	);
}

function verdictInput(
	t: DevicesT,
	time: AreaTime,
	page: DevicePage,
): VerdictInput {
	return {
		t,
		time,
		page,
		rows: page.services ?? page.lockedRows?.services ?? null,
		lockedData: !page.services && !!page.lockedRows,
	};
}
