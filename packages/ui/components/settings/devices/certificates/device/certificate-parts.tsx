"use client";

import { useTranslation } from "@flow-like/locales";
import { useQuery } from "@tanstack/react-query";
import { type ReactNode, useEffect, useMemo, useRef } from "react";
import type { CertificateErrorCategory } from "../../../../../lib/device-management/certificate-acme";
import { MAX_CERTIFICATE_PEM_BYTES } from "../../../../../lib/device-management/certificates";
import type {
	FixAction,
	Freshness,
	GateResult,
} from "../../../../../lib/device-management/model/types";
import type { ManagementCall } from "../../../../../lib/device-management/telemetry";
import { ManagementUnconfirmedError } from "../../../../../lib/device-management/transport";
import { useBackend } from "../../../../../state/backend-state";
import { gateCopy } from "../../copy/gate-copy";
import {
	type AreaTime,
	type DevicesT,
	useAreaTime,
} from "../../primitives/area-context";
import { DvButton, type DvButtonProps } from "../../primitives/dv-button";
import { DvInput, Field, utf8Bytes } from "../../primitives/form-fields";
import { FreshnessStamp } from "../../primitives/freshness-stamp";
import {
	type Gate,
	GateNotice,
	GatedAction,
} from "../../primitives/gate-notice";
import { InlineResult } from "../../primitives/inline-result";
import { cx } from "../../primitives/tone";
import { useDevicesRoute } from "../../routing/use-devices-route";
import { stampOf, useRunAttentionTarget } from "../../shell/attention-popover";
import { type DeviceActionContext, useInlineResults } from "../../workspace";
import { LIVE_EVERY_S, MAX_CERTIFICATE_SLOTS } from "./use-device-certificates";

export const MENU_CONTENT =
	"min-w-60 border-border-strong bg-popover shadow-none backdrop-blur-none";
export const MENU_ITEM =
	"items-start text-[13px]/[18px] focus:bg-row-hover focus:text-foreground";
export const LINK =
	"underline decoration-border-strong underline-offset-2 hover:decoration-current";
/**
 * The app's base layer gives every table outer margins and cell borders on all
 * sides, and chips in cells are pills where the design has 4 px corners.
 * Remove once `DvTable` does both itself (requested from W4-SWITCH).
 */
export const TABLE_RESET =
	"my-0 [&_td]:border-x-0 [&_td]:border-b-0 [&_th]:border-x-0 [&_th]:border-t-0 [&_td_[data-slot=badge]]:h-auto [&_td_[data-slot=badge]]:min-h-5.5 [&_td_[data-slot=badge]]:rounded-md [&_td_[data-slot=badge]]:py-0.5 [&_td_[data-slot=badge]]:whitespace-normal [&_td_[data-slot=badge]>span]:whitespace-normal";
/** A chip whose label may take two lines in a narrow column instead of being cut. */
export const CHIP_WRAP =
	"h-auto min-h-5.5 items-start rounded-md py-0.5 whitespace-normal [&>span]:whitespace-normal [&>svg]:mt-0.5";

/* Gates (R7). */

export interface GateView {
	gate: Gate;
	fix?: { label: string; action: FixAction };
}

/**
 * Controls the viewer can never use here (no permission, not the owner, an
 * agent from before the feature) aren't offered at all; the block says so
 * once. Everything else stays visible and disabled with its reason.
 */
const NEVER_HERE = new Set<Gate["kind"]>(["noaccess", "owner", "unsupported"]);

export const neverHere = (result: GateResult) =>
	!result.ok && (result.hide || NEVER_HERE.has(result.kind));

export function gateView(
	t: DevicesT,
	time: Pick<AreaTime, "at" | "locale">,
	result: GateResult,
): GateView | null {
	if (result.ok) return null;
	const copy = gateCopy(t, result, { at: time.at, locale: time.locale });
	return {
		gate: { kind: result.kind, reason: copy.inline },
		...(result.fix && copy.fix
			? { fix: { label: copy.fix, action: result.fix } }
			: {}),
	};
}

/** The fix that belongs to a gate reason ("Unlock…", "Connect live"). */
export function GateFixButton({ view }: Readonly<{ view: GateView | null }>) {
	const { navigate } = useDevicesRoute();
	const run = useRunAttentionTarget({ onNavigate: navigate });
	const fix = view?.fix;
	if (!fix) return null;
	return (
		<DvButton size="xs" onClick={() => run(fix.action)}>
			{fix.label}
		</DvButton>
	);
}

/** A control behind a gate: hidden when it can never apply, else disabled with the one-line reason. */
export function GatedButton({
	result,
	reason,
	children,
	...button
}: Readonly<
	DvButtonProps & {
		result: GateResult;
		/** Replaces the gate's own sentence ("Used by 1 service. …"). */
		reason?: ReactNode;
	}
>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	if (neverHere(result)) return null;
	const view = gateView(t, time, result);
	const control = <DvButton {...button}>{children}</DvButton>;
	if (!view) return control;
	return (
		<GatedAction gate={reason ? { kind: view.gate.kind, reason } : view.gate}>
			{control}
		</GatedAction>
	);
}

/* Results (R9). */

export const certificateResultKey = (deviceId: string, target: string) =>
	`certificates:${deviceId}:${target}`;

export function ActionResults({
	scopeKey,
	className,
}: Readonly<{ scopeKey: string; className?: string }>) {
	const results = useInlineResults(scopeKey);
	if (!results.length) return null;
	return (
		<div className={cx("flex flex-col gap-1.5", className)}>
			{results.map((result) => (
				<InlineResult
					key={result.id}
					tone={result.tone}
					onDismiss={result.dismiss}
				>
					{result.text}
				</InlineResult>
			))}
		</div>
	);
}

/**
 * The certificate calls of the device lib answer every failure with one
 * sentence. This keeps the cause for the action layer: the device's own
 * reason for a refusal, "no reply" for a lost answer, and "not confirmed" for
 * an answer that doesn't match what was sent.
 */
export async function withDeviceReason<T>(
	context: Pick<DeviceActionContext, "request">,
	run: (call: ManagementCall) => Promise<T>,
): Promise<T> {
	let cause: unknown;
	let answered: string | undefined;
	const call: ManagementCall = async (command, operationId) => {
		try {
			const response = await context.request(command, operationId);
			answered = response.operation_id;
			return response;
		} catch (error) {
			cause = error;
			throw error;
		}
	};
	try {
		return await run(call);
	} catch (error) {
		if (cause) throw cause;
		if (answered !== undefined) throw new ManagementUnconfirmedError(answered);
		throw error;
	}
}

/** False once the component is gone, so a late answer changes nothing. */
export function useAlive(): () => boolean {
	const alive = useRef(true);
	useEffect(() => {
		alive.current = true;
		return () => {
			alive.current = false;
		};
	}, []);
	return useMemo(() => () => alive.current, []);
}

/* Stamps (R5). */

/** A live read's source and age; the certificate list says "details read …". */
export function LiveStamp({
	freshness,
	details = false,
}: Readonly<{ freshness: Freshness; details?: boolean }>) {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { at } = freshness;
	const text =
		details && freshness.age === "live" && at !== undefined
			? t("deviceCertificates.stamp.detailsRead", "details read {{ago}}", {
					ago: time.ago(Math.min(at, time.nowS)),
				})
			: undefined;
	return (
		<FreshnessStamp
			{...stampOf(freshness)}
			cadenceSec={LIVE_EVERY_S}
			{...(text ? { text } : {})}
		/>
	);
}

/* Dates and names. */

const dayFormatters = new Map<string, Intl.DateTimeFormat>();

/** "5 Oct", with the year when it differs from now. */
export function dayLabel(locale: string, atS: number, nowS: number): string {
	const withYear =
		new Date(atS * 1000).getFullYear() !== new Date(nowS * 1000).getFullYear();
	const key = `${locale}|${withYear}`;
	let formatter = dayFormatters.get(key);
	if (!formatter) {
		formatter = new Intl.DateTimeFormat(locale, {
			day: "numeric",
			month: "short",
			year: withYear ? "numeric" : undefined,
		});
		dayFormatters.set(key, formatter);
	}
	return formatter.format(atS * 1000);
}

export function useDay(): (atS: number) => string {
	const time = useAreaTime();
	const { locale, nowS } = time;
	const day = Math.floor(nowS / 86_400);
	// biome-ignore lint/correctness/useExhaustiveDependencies: the label only changes with the calendar day, not every second
	return useMemo(
		() => (atS: number) => dayLabel(locale, atS, nowS),
		[locale, day],
	);
}

/** A comma and a no-break space (U+00A0): the comma stays with its name when the list wraps. */
const NAME_SEPARATOR = `,${String.fromCharCode(0xa0)}`;

/** DNS names and IP addresses exactly as the certificate or request carries them. */
export function Names({
	names,
	empty,
}: Readonly<{ names: readonly string[]; empty?: ReactNode }>) {
	if (!names.length)
		return <span className="text-muted-foreground">{empty}</span>;
	return (
		<span data-names="" className="font-mono text-xs">
			{names.map((name, index) => (
				// A line breaks between names; inside one only when it is longer than the column.
				<span key={name} className="inline-block max-w-full wrap-anywhere">
					{name}
					{index < names.length - 1 ? NAME_SEPARATOR : null}
				</span>
			))}
		</span>
	);
}

/** One sentence for a list of names ("a, b and c"). */
export function listOf(locale: string, values: readonly string[]): string {
	return new Intl.ListFormat(locale, { type: "conjunction" }).format(values);
}

const PERSON_STALE_MS = 10 * 60_000;

/** The owner's display name; undefined until it is known. */
export function usePersonName(
	userId: string | undefined,
	enabled = true,
): string | undefined {
	const backend = useBackend();
	const id = userId ?? "";
	const query = useQuery({
		queryKey: ["devices", "person-name", id],
		queryFn: () => backend.userState.lookupUser(id),
		enabled: enabled && id !== "",
		retry: false,
		staleTime: PERSON_STALE_MS,
	});
	const user = query.data as
		| { name?: string; preferred_username?: string; username?: string }
		| null
		| undefined;
	return [user?.name, user?.preferred_username, user?.username].find(Boolean);
}

/** IA §6.2 N2: the renewal sections are read-only for everyone but the owner. */
export function OwnerOnlyNote({
	ownerId,
	className,
}: Readonly<{ ownerId: string; className?: string }>) {
	const { t } = useTranslation("devices");
	const owner = usePersonName(ownerId);
	return (
		<p
			data-owner-only=""
			className={cx("text-xs text-muted-foreground", className)}
		>
			{owner
				? t(
						"deviceCertificates.ownerOnlyNamed",
						"Only the owner, {{owner}}, can set up automatic renewal.",
						{ owner },
					)
				: t(
						"deviceCertificates.ownerOnly",
						"Only the owner can set up automatic renewal.",
					)}
		</p>
	);
}

/* BG27: why renewal failed, by the cause the device reports. */

export function failureCause(
	t: DevicesT,
	category: CertificateErrorCategory,
): string {
	const causes = {
		dns: t(
			"devices:deviceCertificates.cause.dns",
			"The certificate authority couldn't find the name, or the name's DNS records don't allow it to issue.",
		),
		port_bind: t(
			"devices:deviceCertificates.cause.portBind",
			"The device couldn't open the challenge address. Port 80 is taken or the agent may not use it.",
		),
		rate_limited: t(
			"devices:deviceCertificates.cause.rateLimited",
			"The certificate authority limits how often this name can be requested. The device waits and tries again.",
		),
		ca_rejected: t(
			"devices:deviceCertificates.cause.caRejected",
			"The certificate authority refused the request: the answer on port 80 was wrong, or it doesn't accept the name.",
		),
		authority_expired: t(
			"devices:deviceCertificates.cause.authorityExpired",
			"The authority that renews it has expired or expires within one hour.",
		),
		network: t(
			"devices:deviceCertificates.cause.network",
			"The device and the certificate authority couldn't reach each other.",
		),
		internal: t(
			"devices:deviceCertificates.cause.internal",
			"Something failed on the device itself. Its agent logs say what.",
		),
	} satisfies Record<CertificateErrorCategory, string>;
	return causes[category];
}

interface RenewalFailure {
	failures?: number;
	error_category?: CertificateErrorCategory;
	last_error: string | null;
}

/**
 * The last renewal error as one sentence: the cause the device names (BG27),
 * an expired authority, or else the device's own words, quoted as they came.
 */
export function lastErrorText(
	t: DevicesT,
	row: RenewalFailure,
	authorityExpired = false,
): string | null {
	if (row.error_category) return failureCause(t, row.error_category);
	if (authorityExpired) return failureCause(t, "authority_expired");
	if (!row.last_error) return null;
	return t(
		"devices:deviceCertificates.deviceSays",
		"The device says: “{{message}}”",
		{ message: row.last_error },
	);
}

/**
 * BG27: how often renewal failed in a row, and what is still open about the
 * cause. Agents from before it report neither: the fixed sentence says so.
 */
export function failureSummary(
	t: DevicesT,
	row: RenewalFailure,
	detailed: boolean,
): { attempts?: string; note?: string } {
	if (!detailed)
		return row.last_error
			? {
					note: t(
						"devices:deviceCertificates.cause.interim",
						"This device's agent doesn't report how often renewal failed or why. Update the agent to see both.",
					),
				}
			: {};
	const failures = row.failures ?? 0;
	if (failures <= 0) return {};
	return {
		attempts: t("devices:deviceCertificates.failures", {
			count: failures,
			defaultValue_one: "{{count, number}} failed attempt in a row",
			defaultValue_other: "{{count, number}} failed attempts in a row",
		}),
		...(row.error_category
			? {}
			: {
					note: t(
						"devices:deviceCertificates.cause.unknownYet",
						"The cause isn't known yet. It shows after the device's next attempt.",
					),
				}),
	};
}

/* Sheet parts shared by the import, request and Let's Encrypt forms. */

export type LabelProblem = "empty" | "too_long";

/** The device takes labels of 1 to 128 bytes without control characters. */
export function labelProblem(label: string): LabelProblem | null {
	const value = label.trim();
	if (!value) return "empty";
	return utf8Bytes(value) > 128 || /\p{Cc}/u.test(label) ? "too_long" : null;
}

export function LabelField({
	id,
	value,
	onChange,
	showProblem,
}: Readonly<{
	id: string;
	value: string;
	onChange(value: string): void;
	/** After the first submit: say what is wrong with the label. */
	showProblem: boolean;
}>) {
	const { t } = useTranslation("devices");
	const problem = showProblem ? labelProblem(value) : null;
	const texts: Record<LabelProblem, string> = {
		empty: t("deviceCertificates.request.labelEmpty", "Give it a label."),
		too_long: t(
			"deviceCertificates.request.labelTooLong",
			"Use at most 128 bytes and no control characters.",
		),
	};
	return (
		<Field
			id={id}
			label={t("deviceCertificates.request.labelField", "Label")}
			error={problem ? texts[problem] : undefined}
		>
			<DvInput
				value={value}
				maxLength={128}
				autoComplete="off"
				onChange={(event) => onChange(event.target.value)}
			/>
		</Field>
	);
}

/** Why a sheet can't be sent now: the gate, a request that already waits, or no free slot. */
export function SheetBlocker({
	view,
	waitingFor,
	full = false,
}: Readonly<{
	view: GateView | null;
	/** Label of the certificate whose signing request is still waiting. */
	waitingFor?: string;
	full?: boolean;
}>) {
	const { t } = useTranslation("devices");
	if (view)
		return <GateNotice kind={view.gate.kind} title={view.gate.reason} />;
	if (waitingFor !== undefined)
		return (
			<GateNotice
				kind="busy"
				title={t(
					"deviceCertificates.request.waiting",
					"A signing request for {{label}} is already waiting.",
					{ label: waitingFor },
				)}
				text={t(
					"deviceCertificates.request.waitingText",
					"Install its signed certificate or discard it first.",
				)}
			/>
		);
	if (!full) return null;
	return (
		<GateNotice
			kind="busy"
			title={t(
				"deviceCertificates.request.full",
				"All {{max, number}} certificate slots are used.",
				{ max: MAX_CERTIFICATE_SLOTS },
			)}
			text={t(
				"deviceCertificates.request.fullText",
				"Delete a certificate or discard a request first.",
			)}
		/>
	);
}

/** Why a sheet's primary is off although no gate applies; sits beside the button as the sheet's foot note (R7). */
export function FootReason({ children }: Readonly<{ children: ReactNode }>) {
	return (
		<span data-foot-reason="" className="text-warning">
			{children}
		</span>
	);
}

/** The reason while a sheet's confirmation isn't ticked. */
export function TickFirst() {
	const { t } = useTranslation("devices");
	return (
		<FootReason>
			{t("deviceCertificates.tickFirst", "Tick the confirmation above first.")}
		</FootReason>
	);
}

/* Files. */

export function downloadText(name: string, text: string, type: string) {
	const url = URL.createObjectURL(new Blob([text], { type }));
	const link = document.createElement("a");
	link.href = url;
	link.download = name;
	link.click();
	setTimeout(() => URL.revokeObjectURL(url), 1000);
}

const CHAIN_PEM =
	/^\s*-----BEGIN CERTIFICATE-----[\s\S]+-----END CERTIFICATE-----\s*$/u;

export type ChainProblem = "empty" | "too_large" | "not_a_chain" | "has_key";

/** A signed chain goes to the device as it is: PEM certificates only, never a key. */
export function chainProblem(chain: string): ChainProblem | null {
	if (!chain.trim()) return "empty";
	if (utf8Bytes(chain) > MAX_CERTIFICATE_PEM_BYTES) return "too_large";
	if (/PRIVATE KEY/u.test(chain)) return "has_key";
	return CHAIN_PEM.test(chain) ? null : "not_a_chain";
}

export function chainProblemText(t: DevicesT, problem: ChainProblem): string {
	const texts = {
		empty: t(
			"devices:deviceCertificates.chain.empty",
			"Add the signed certificate chain.",
		),
		too_large: t(
			"devices:deviceCertificates.chain.tooLarge",
			"The chain is larger than 12 KiB. Send the certificate and its intermediates only.",
		),
		has_key: t(
			"devices:deviceCertificates.chain.hasKey",
			"This contains a private key. The device keeps its own key: send the signed certificates only.",
		),
		not_a_chain: t(
			"devices:deviceCertificates.chain.notAChain",
			"This isn't a PEM certificate chain. It starts with the signed certificate, followed by its intermediates.",
		),
	} satisfies Record<ChainProblem, string>;
	return texts[problem];
}

export const formatKiB = (locale: string, bytes: number) =>
	new Intl.NumberFormat(locale, { maximumFractionDigits: 1 }).format(
		bytes / 1024,
	);
