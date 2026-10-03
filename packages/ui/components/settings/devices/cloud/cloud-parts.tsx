"use client";

import { useTranslation } from "@flow-like/locales";
import { CircleCheck, CircleSlash, OctagonX } from "lucide-react";
import { type ReactNode, useMemo } from "react";
import { useUserIdentity } from "../../../../hooks/use-user-lookup";
import type {
	DevicesRoute,
	DevicesScope,
	GateResult,
} from "../../../../lib/device-management/model/types";
import { formatEuroMicros } from "../../../../lib/device-resources";
import { UnknownPerson, identityName } from "../access/person-name";
import { enumLabel } from "../copy/enum-labels";
import { gateCopy } from "../copy/gate-copy";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { DvButton, type DvButtonProps } from "../primitives/dv-button";
import type { Gate } from "../primitives/gate-notice";
import { KvRow } from "../primitives/key-value-list";
import { PersonChip } from "../primitives/person-chip";
import { StatusChip } from "../primitives/status-chip";
import { useRouteLink } from "../routing/use-devices-route";
import { useAttentionState } from "../workspace";
import { type ApprovalState, type CloudApproval, endOf } from "./cloud-model";
import { useAppNames, useModelNames } from "./use-cloud";

/** A table's first-column name; as a link it is underlined only on hover. */
export const NAME_TEXT = "block truncate font-mono text-[12.5px] font-semibold";
export const NAME_LINK = `${NAME_TEXT} no-underline hover:underline`;

/**
 * Facts in a side column: label and value stay side by side until the area
 * itself is phone-sized (the shared list stacks by its own width).
 */
export function SideFacts({ children }: Readonly<{ children: ReactNode }>) {
	return (
		<dl className="m-0 grid min-w-0 grid-cols-[minmax(112px,max-content)_minmax(0,1fr)] gap-x-4 gap-y-1.5 text-ui @max-[520px]/devices:grid-cols-1 @max-[520px]/devices:gap-y-0.5">
			{children}
		</dl>
	);
}

export function SideFact({
	label,
	children,
}: Readonly<{ label: ReactNode; children: ReactNode }>) {
	return (
		<div className="contents">
			<dt className="text-muted-foreground @max-[520px]/devices:mt-1.5">
				{label}
			</dt>
			<dd className="m-0 min-w-0 wrap-anywhere">{children}</dd>
		</div>
	);
}

/** Exact amount (never rounded): limits in consent and confirm sentences. */
export const money = formatEuroMicros;

/** Display amount in the viewer's locale, rounded to cents like the spend meter. */
export function useMoney(): (micros: number) => string {
	const { i18n } = useTranslation("devices");
	const locale = i18n?.language ?? "en";
	return useMemo(() => {
		const format = new Intl.NumberFormat(locale, {
			style: "currency",
			currency: "EUR",
		});
		return (micros: number) => format.format(micros / 1_000_000);
	}, [locale]);
}

/** A failed gate as the disabled control shows it; `reason` replaces the generic sentence. */
export function useGateOf(): (
	result: GateResult,
	reason?: ReactNode,
) => Gate | null {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	return (result, reason) =>
		result.ok
			? null
			: {
					kind: result.kind,
					reason: reason ?? gateCopy(t, result, time).inline,
				};
}

/** A person by account id: "You" for the viewer, the directory name otherwise; never the account id. */
export function CloudPerson({
	userId,
	fullName = false,
}: Readonly<{ userId: string; fullName?: boolean }>) {
	const { t } = useTranslation("devices");
	const { input } = useAttentionState();
	const identity = useUserIdentity(userId);
	const you = userId === input.me;
	const name = identityName(identity, userId);
	if (!name && !you)
		return (
			<UnknownPerson
				pending={identity.isPending}
				unreachable={!identity.isResolved}
			/>
		);
	return (
		<PersonChip
			name={name ?? t("cloud.spend.you", "You")}
			you={you}
			fullName={fullName}
			{...(identity.subtitle ? { email: identity.subtitle } : {})}
			{...(identity.avatarUrl ? { avatarUrl: identity.avatarUrl } : {})}
		/>
	);
}

/** The display name of an account for sentences ("Only Mira Novak can …"). */
export function usePersonName(userId: string | undefined): string | undefined {
	return identityName(useUserIdentity(userId), userId);
}

/** Who pays: by name when the device's list says so, else "You" or "Someone else". */
export function Payer({ approval }: Readonly<{ approval: CloudApproval }>) {
	const { t } = useTranslation("devices");
	const limit = approval.limit;
	if (!limit) return <span className="text-muted-foreground">–</span>;
	if (limit.payerId) return <CloudPerson userId={limit.payerId} />;
	return limit.payerIsMe ? (
		<PersonChip name={t("cloud.spend.you", "You")} you />
	) : (
		<span>{t("cloud.spend.someoneElse", "Someone else")}</span>
	);
}

/** An in-area link that looks like text. */
export function RouteLink({
	route,
	scope,
	className,
	children,
}: Readonly<{
	route: DevicesRoute;
	scope?: DevicesScope;
	className?: string;
	children: ReactNode;
}>) {
	const link = useRouteLink();
	return (
		<a
			{...link(route, scope ? { scope } : undefined)}
			className={
				className ??
				"underline decoration-border-strong underline-offset-2 hover:decoration-foreground"
			}
		>
			{children}
		</a>
	);
}

/** A button that navigates inside the area (a real link: opens in a new tab with a modifier). */
export function RouteButton({
	route,
	scope,
	children,
	...button
}: Readonly<
	Omit<DvButtonProps, "asChild" | "onClick"> & {
		route: DevicesRoute;
		scope?: DevicesScope;
	}
>) {
	const link = useRouteLink();
	return (
		<DvButton {...button} asChild>
			<a {...link(route, scope ? { scope } : undefined)}>{children}</a>
		</DvButton>
	);
}

/** "App {name}", linked to App › Devices (APP §6.2); a local-only app has no page on the hub side. */
export function AppRef({
	appId,
	projectId,
}: Readonly<{ appId: string | null; projectId?: string }>) {
	const { t } = useTranslation("devices");
	const names = useAppNames();
	const id = appId ?? projectId;
	if (!id) return null;
	const name = names(id);
	const label = name
		? enumLabel(t, "scopeKind", "project", { name })
		: appId
			? t("cloud.app.unnamed", "An app you can't open")
			: t("cloud.app.localOnly", "A local-only app");
	if (!name) return <span className="text-muted-foreground">{label}</span>;
	return (
		<RouteLink
			route={{ screen: "app-devices", by: "device" }}
			scope={{ kind: "app", appId: id }}
		>
			{label}
		</RouteLink>
	);
}

export function ModelList({ ids }: Readonly<{ ids: readonly string[] }>) {
	const { t } = useTranslation("devices");
	const name = useModelNames(ids);
	if (!ids.length)
		return (
			<span className="text-muted-foreground">
				{t("cloud.models.none", "No models")}
			</span>
		);
	return <span data-models="">{ids.map(name).join(", ")}</span>;
}

const STATE_LOOK = {
	active: { tone: "good", icon: CircleCheck },
	revoked: { tone: "critical", icon: OctagonX },
	expired: { tone: "paused", icon: CircleSlash },
} as const;

export function ApprovalChip({ state }: Readonly<{ state: ApprovalState }>) {
	const { t } = useTranslation("devices");
	const look = STATE_LOOK[state];
	return (
		<StatusChip tone={look.tone} icon={look.icon}>
			{enumLabel(t, "approvalStatus", state)}
		</StatusChip>
	);
}

export interface ExpiryCopy {
	/** The date line. */
	main: string;
	/** The date alone, for a row whose label already says "Ended". */
	plain?: string;
	/** "in 29 d", or why it ended. */
	sub?: string;
	/** BG32: why it ends before the approved date, or the older-hub note. */
	note?: string;
	tone?: "warning";
}

type EndLimit = NonNullable<CloudApproval["effective"]>["limit"] | undefined;

/** What ends an approval before its approved date. */
function earlyReason(
	t: DevicesT,
	limit: EndLimit,
	device: string,
): string | undefined {
	if (limit === "sharing_grant")
		return t(
			"devices:cloud.expiry.sharing",
			"The approver's permission to deploy on {{device}} ends then.",
			{ device },
		);
	if (limit === "access_rules")
		return t(
			"devices:cloud.expiry.rules",
			"The access rules of {{device}} expire then. Renewing them extends it.",
			{ device },
		);
	return undefined;
}

/** Why an approval that no longer runs ended. */
function endedReason(t: DevicesT, limit: EndLimit, device: string): string {
	if (limit === "sharing_grant")
		return t(
			"devices:cloud.expiry.endedSharing",
			"The approver's permission to deploy on {{device}} ended.",
			{ device },
		);
	if (limit === "access_rules")
		return t(
			"devices:cloud.expiry.endedRules",
			"The access rules of {{device}} expired or can't be checked.",
			{ device },
		);
	return t("devices:cloud.expiry.endedOwn", "It reached its end date.");
}

/** BG32: the real end with what limits it; older hubs show the approved date and say so. */
export function useExpiryCopy(): (
	approval: CloudApproval,
	deviceName: string,
) => ExpiryCopy {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	return (approval, device) => {
		const approved = time.at(approval.expiresAt);
		const end = endOf(approval);
		const limit = approval.effective?.limit;
		if (approval.state === "revoked")
			return {
				main: t("cloud.expiry.revoked", "Doesn't apply: revoked"),
				sub: t("cloud.expiry.wouldHave", "It was approved until {{date}}.", {
					date: approved,
				}),
			};
		if (approval.state === "expired")
			return {
				main: t("cloud.expiry.ended", "Ended {{date}}", { date: time.at(end) }),
				plain: time.at(end),
				sub: endedReason(t, limit, device),
				tone: "warning",
			};
		if (!approval.effective)
			return {
				main: approved,
				sub: time.ago(approval.expiresAt),
				note: t(
					"cloud.expiry.nominal",
					"The approved date. It ends earlier if the approver's access to {{device}} ends first; this hub doesn't report that.",
					{ device },
				),
			};
		const why =
			end < approval.expiresAt ? earlyReason(t, limit, device) : undefined;
		if (!why) return { main: time.at(end), sub: time.ago(end) };
		return {
			main: time.at(end),
			sub: time.ago(end),
			note: t(
				"cloud.expiry.early",
				"Before the approved date ({{date}}): {{why}}",
				{ date: approved, why },
			),
			tone: "warning",
		};
	};
}

/** The "Ends" row of an approval's facts. */
export function EndRow({
	approval,
	deviceName,
}: Readonly<{ approval: CloudApproval; deviceName: string }>) {
	const { t } = useTranslation("devices");
	const end = useExpiryCopy()(approval, deviceName);
	return (
		<KvRow
			label={
				approval.state === "expired"
					? t("cloud.kv.ended", "Ended")
					: t("cloud.kv.ends", "Ends")
			}
		>
			<span data-expiry={approval.effective ? "effective" : "approved"}>
				{end.plain ?? end.main}
				{end.sub ? (
					<span className="text-muted-foreground"> · {end.sub}</span>
				) : null}
			</span>
			{end.note ? (
				<span
					className={`mt-0.5 block text-xs ${end.tone === "warning" ? "text-warning" : "text-muted-foreground"}`}
				>
					{end.note}
				</span>
			) : null}
		</KvRow>
	);
}
