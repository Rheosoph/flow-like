"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CircleCheck,
	Info,
	LoaderCircle,
	type LucideIcon,
	OctagonX,
	TriangleAlert,
} from "lucide-react";
import {
	type ComponentProps,
	Fragment,
	type ReactNode,
	useCallback,
	useMemo,
} from "react";
import { useInvoke } from "../../../../hooks/use-invoke";
import { LIVE_TIMING } from "../../../../lib/device-management/workspace/live";
import type { LiveState } from "../../../../lib/device-management/workspace/types";
import { useBackend } from "../../../../state/backend-state";
import type { IAppState } from "../../../../state/backend-state/app-state";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import { CheckField, Field, SecretInput } from "../primitives/form-fields";
import type { KeyChipState } from "../primitives/status-chip";
import { TONE_SURFACE, TONE_TEXT, cx } from "../primitives/tone";
import type { KeyRow } from "./keys-model";

/* Small presentational pieces shared by the N9 screen, its sheets and the device Keys tab. */

/**
 * The app's base layer gives every `table` a margin and every cell a full
 * border; inside a block only the row hairlines of the table primitive remain.
 */
export const KEYS_TABLE_CLASS =
	"my-0 [&_td]:border-x-0 [&_td]:border-b-0 [&_th]:border-x-0 [&_th]:border-t-0";

const dayFormatters = new Map<string, Intl.DateTimeFormat>();

/** "2 Jun" (the year only when it differs from now). */
export function dayLabel(atS: number, nowMs: number, locale: string): string {
	const date = new Date(atS * 1000);
	const withYear = date.getFullYear() !== new Date(nowMs).getFullYear();
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
	return formatter.format(date);
}

/** A day with the full moment on hover (R16). */
export function DayOf({ atS }: Readonly<{ atS: number }>) {
	const time = useAreaTime();
	return (
		<time dateTime={new Date(atS * 1000).toISOString()} title={time.abs(atS)}>
			{dayLabel(atS, time.now, time.locale)}
		</time>
	);
}

export function Mono({ className, ...props }: ComponentProps<"span">) {
	return (
		<span className={cx("font-mono text-[0.92em]", className)} {...props} />
	);
}

/** Names in mono, joined the way the language joins a list. */
export function MonoNames({ names }: Readonly<{ names: readonly string[] }>) {
	const { i18n } = useTranslation("devices");
	const parts = new Intl.ListFormat(i18n?.language ?? "en", {
		type: "conjunction",
	}).formatToParts(names);
	return (
		<>
			{parts.map((part, index) =>
				part.type === "element" ? (
					// biome-ignore lint/suspicious/noArrayIndexKey: a fixed list, rendered once per value
					<Mono key={index}>{part.value}</Mono>
				) : (
					// biome-ignore lint/suspicious/noArrayIndexKey: list separators have no identity
					<Fragment key={index}>{part.value}</Fragment>
				),
			)}
		</>
	);
}

/** `.kx-note`: a short explanation inside a sheet, info or warning. */
export function Note({
	tone = "info",
	icon,
	children,
}: Readonly<{
	tone?: "info" | "warning" | "good";
	icon?: LucideIcon;
	children: ReactNode;
}>) {
	const Icon =
		icon ??
		(tone === "warning" ? TriangleAlert : tone === "good" ? CircleCheck : Info);
	return (
		<div
			data-note={tone}
			className={cx(
				"flex items-start gap-2 rounded-lg border px-2.5 py-2 text-ui text-ink-2",
				TONE_SURFACE[tone],
			)}
		>
			<Icon
				aria-hidden
				className={cx("mt-px size-4 shrink-0", TONE_TEXT[tone])}
			/>
			<span className="min-w-0">{children}</span>
		</div>
	);
}

export interface GuideStep {
	id: string;
	title: ReactNode;
	hint?: ReactNode;
	/** Buttons or a chip under the step. */
	actions?: ReactNode;
}

/** `.guide`: a real sequence, so the numbers are shown. */
export function GuideSteps({
	steps,
	dense = false,
	id,
}: Readonly<{ steps: readonly GuideStep[]; dense?: boolean; id?: string }>) {
	return (
		<ol
			id={id}
			data-guide=""
			className={cx(
				"m-0 flex list-none flex-col p-0",
				dense ? "gap-2.5" : "gap-3.5",
			)}
		>
			{steps.map((step, index) => (
				<li
					key={step.id}
					className="grid grid-cols-[26px_minmax(0,1fr)] gap-2.5"
				>
					<span
						aria-hidden
						className="inline-flex size-6 items-center justify-center rounded-full border border-border-strong font-mono text-xs font-medium text-ink-2"
					>
						{index + 1}
					</span>
					<div className="flex min-w-0 flex-col gap-1.5 pt-0.5">
						<p className="text-sm">{step.title}</p>
						{step.hint ? (
							<p className="text-xs text-muted-foreground">{step.hint}</p>
						) : null}
						{step.actions ? (
							<div className="flex flex-wrap items-center gap-2">
								{step.actions}
							</div>
						) : null}
					</div>
				</li>
			))}
		</ol>
	);
}

/** The explanation line under a fact (`.kv dd .hint`). */
export function KvHint({ children }: Readonly<{ children: ReactNode }>) {
	return <p className="mt-0.5 text-xs text-muted-foreground">{children}</p>;
}

/** `.kx-rs-list`: the bordered list of devices or files inside a sheet. */
export function SheetList({ children }: Readonly<{ children: ReactNode }>) {
	return (
		<ul className="flex list-none flex-col rounded-lg border border-border bg-card p-0">
			{children}
		</ul>
	);
}

export function SheetListItem({
	head,
	status,
	children,
	...props
}: Readonly<
	{ head: ReactNode; status?: ReactNode; children?: ReactNode } & Omit<
		ComponentProps<"li">,
		"children"
	>
>) {
	return (
		<li
			className="flex min-w-0 flex-col gap-2 border-t border-hairline px-3 py-2.5 first:border-t-0"
			{...props}
		>
			<div className="flex min-w-0 flex-wrap items-center justify-between gap-x-3 gap-y-1">
				{head}
				{status}
			</div>
			{children}
		</li>
	);
}

/** "Use one password for all selected devices" with its field (restore and download). */
export function SamePassword({
	id,
	same,
	onSame,
	password,
	onPassword,
	hint,
}: Readonly<{
	id: string;
	same: boolean;
	onSame(same: boolean): void;
	password: string;
	onPassword(password: string): void;
	hint?: ReactNode;
}>) {
	const { t } = useTranslation("devices");
	return (
		<>
			<CheckField id={id} checked={same} onCheckedChange={onSame}>
				{t(
					"keys.samePassword.check",
					"Use one password for all selected devices",
				)}
			</CheckField>
			{same ? (
				<Field
					id={`${id}-password`}
					label={t("keys.samePassword.label", "Device password")}
					hint={hint}
				>
					<SecretInput value={password} onValueChange={onPassword} />
				</Field>
			) : null}
		</>
	);
}

export type RowStatusTone = "muted" | "info" | "good" | "warning" | "critical";

const STATUS_LOOK: Record<
	RowStatusTone,
	{ cls: string; icon?: LucideIcon; spin?: boolean }
> = {
	muted: { cls: "text-muted-foreground" },
	info: { cls: "text-info", icon: LoaderCircle, spin: true },
	good: { cls: "text-good", icon: CircleCheck },
	warning: { cls: "text-warning", icon: TriangleAlert },
	critical: { cls: "text-critical", icon: OctagonX },
};

/** `.kx-rs-st`: the per-row state on the right of a sheet list row. */
export function RowStatus({
	tone,
	children,
}: Readonly<{ tone: RowStatusTone; children: ReactNode }>) {
	const look = STATUS_LOOK[tone];
	const Icon = look.icon;
	return (
		<span
			data-row-status={tone}
			className={cx("inline-flex items-center gap-1 text-xs", look.cls)}
		>
			{Icon ? (
				<Icon
					aria-hidden
					className={cx("size-3.5 shrink-0", look.spin && "animate-spin")}
				/>
			) : null}
			<span className="min-w-0">{children}</span>
		</span>
	);
}

/** "account v3"; without a version to name, what is known instead: none, or not compared yet. */
export function accountVersionLabel(
	t: DevicesT,
	hubRevision: number | undefined,
): string {
	if (hubRevision === undefined)
		return t("devices:keys.backup.accountUnchecked", "account: not checked");
	return hubRevision > 0
		? t("devices:keys.backup.account", "account v{{version}}", {
				version: hubRevision,
			})
		: t("devices:keys.tab.backup.none", "account: none");
}

/** "Owner keys" / "Shared-access keys" (IA §6.6.1). */
export function keyKindLabel(t: DevicesT, role: "owner" | "shared"): string {
	return role === "shared"
		? t("devices:keys.kind.shared", "Shared-access keys")
		: t("devices:keys.kind.owner", "Owner keys");
}

interface KeyChipLook {
	state: KeyChipState;
	transport?: "direct" | "relayed";
	renewsAt?: number;
}

const LIVE_QUIET: readonly LiveState["kind"][] = ["idle", "failed"];

/** An open key session with its live connection on top. */
function liveChip(live: LiveState): KeyChipLook {
	if (live.kind !== "live" && live.kind !== "renewing")
		return {
			state: LIVE_QUIET.includes(live.kind) ? "unlocked" : "reconnecting",
		};
	if (live.transport === "websocket")
		return { state: "live", transport: "relayed" };
	return {
		state: "live",
		transport: "direct",
		renewsAt: live.expiresAt - LIVE_TIMING.renewBeforeS,
	};
}

/** The key chip of a row: its key session, with the live connection on top of open keys. */
export function keyChipOf(row: Pick<KeyRow, "session" | "live">): KeyChipLook {
	const state = row.session ? row.session.state : "locked";
	return state === "unlocked" && row.live ? liveChip(row.live) : { state };
}

type AppList = Awaited<ReturnType<IAppState["getApps"]>>;

async function noApps(): Promise<AppList> {
	return [];
}

/** App id → display name, for scopes that name an app. */
export function useAppNames(): (appId: string) => string | undefined {
	const backend = useBackend();
	const getApps = backend.appState?.getApps ?? noApps;
	const apps = useInvoke(getApps, backend.appState, []);
	const names = useMemo(() => {
		const found = new Map<string, string>();
		for (const [app, meta] of apps.data ?? [])
			if (meta?.name) found.set(app.id, meta.name);
		return found;
	}, [apps.data]);
	return useCallback((appId: string) => names.get(appId), [names]);
}

export function useNames(): (names: readonly string[]) => string {
	const { i18n } = useTranslation("devices");
	const locale = i18n?.language ?? "en";
	return (names) =>
		new Intl.ListFormat(locale, { type: "conjunction" }).format(names);
}
