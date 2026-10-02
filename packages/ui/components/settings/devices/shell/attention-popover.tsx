"use client";

import { useTranslation } from "@flow-like/locales";
import { useCallback, useMemo } from "react";
import { useInvoke } from "../../../../hooks/use-invoke";
import { countAttention } from "../../../../lib/device-management/model/attention";
import type {
	AttentionItem,
	DevicesRoute,
	DevicesScope,
	FixAction,
	Freshness,
	AttentionAction as ItemAction,
} from "../../../../lib/device-management/model/types";
import { useBackend } from "../../../../state/backend-state";
import type { IAppState } from "../../../../state/backend-state/app-state";
import { attentionCopy, attentionNames } from "../copy/attention-copy";
import { gateCopy } from "../copy/gate-copy";
import { type DevicesT, useAreaTime } from "../primitives/area-context";
import {
	type AttentionEntry,
	AttentionList,
} from "../primitives/attention-list";
import { DvButton } from "../primitives/dv-button";
import type { FreshnessStampProps } from "../primitives/freshness-stamp";
import type { Gate } from "../primitives/gate-notice";
import {
	useAttention,
	useFixAction,
	useOverlayStore,
	usePersonNames,
	useSnoozeAttention,
} from "../workspace";
import type { ChromeNavigate } from "./rail-row";

export interface AttentionPopoverProps {
	scope: DevicesScope;
	route: DevicesRoute;
	onNavigate: ChromeNavigate;
	/** App scope: the app's name ("Needs you in Invoice AI"). */
	appName?: string;
	/** Called after a link or action was chosen so the popover closes. */
	onClose?: () => void;
}

/** The popover shows the five most severe items; the rest is one click away. */
export const ATTENTION_POPOVER_CAP = 5;

export interface AttentionPopoverViewProps {
	/** Every open item, most severe first. */
	items: readonly AttentionEntry[];
	critical: number;
	/** Counted items: Info never counts. */
	total: number;
	appName?: string;
	/** "Show all N on Fleet overview" / "Show all N for <app>". */
	onShowAll?: () => void;
	/** App scope: the account-wide list. */
	onAllDevices?: () => void;
}

function PopoverHead({
	critical,
	total,
	appName,
}: Readonly<
	Pick<AttentionPopoverViewProps, "critical" | "total" | "appName">
>) {
	const { t } = useTranslation("devices");
	return (
		<h3 className="flex flex-wrap items-center gap-x-1.5 px-4 pt-3 pb-2 text-ui font-semibold tracking-normal">
			<span>
				{appName
					? t("chrome.attention.headApp", "Needs you in {{app}}", {
							app: appName,
						})
					: t("chrome.attention.head", "Needs you")}
			</span>
			<span className="font-mono font-medium text-muted-foreground tabular-nums">
				· {total}
			</span>
			{critical > 0 ? (
				<span className="font-medium text-critical">
					·{" "}
					{t("chrome.attention.criticalCount", "{{count, number}} critical", {
						count: critical,
					})}
				</span>
			) : null}
		</h3>
	);
}

function PopoverFoot(props: Readonly<AttentionPopoverViewProps>) {
	const { t } = useTranslation("devices");
	const { total, appName, onShowAll, onAllDevices } = props;
	if (!(total > 0 && onShowAll) && !onAllDevices) return null;
	return (
		<div className="flex flex-wrap gap-2 border-t border-hairline px-4 py-2.5">
			{total > 0 && onShowAll ? (
				<DvButton size="sm" onClick={onShowAll}>
					{appName
						? t(
								"chrome.attention.showAllApp",
								"Show all {{count, number}} for {{app}}",
								{ count: total, app: appName },
							)
						: t(
								"chrome.attention.showAll",
								"Show all {{count, number}} on Fleet overview",
								{ count: total },
							)}
				</DvButton>
			) : null}
			{onAllDevices ? (
				<DvButton size="sm" variant="ghost" onClick={onAllDevices}>
					{t("chrome.attention.allDevices", "All devices' attention")}
				</DvButton>
			) : null}
		</div>
	);
}

/** The attention popover over translated entries. */
export function AttentionPopoverView(
	props: Readonly<AttentionPopoverViewProps>,
) {
	const { t } = useTranslation("devices");
	const { items, critical, total, appName } = props;
	return (
		<div data-chrome="attention-popover" className="flex min-w-0 flex-col">
			<PopoverHead critical={critical} total={total} appName={appName} />
			<AttentionList
				items={items}
				cap={ATTENTION_POPOVER_CAP}
				compact
				base="auto"
				emptyText={
					appName
						? t(
								"chrome.attention.emptyApp",
								"Nothing in {{app}} needs you right now.",
								{ app: appName },
							)
						: undefined
				}
				className="border-t border-hairline"
			/>
			<PopoverFoot {...props} />
		</div>
	);
}

/* Binding: model items → translated entries with their actions. */

/** A model freshness as stamp props (R5). */
export function stampOf(freshness: Freshness): FreshnessStampProps {
	const failure = freshness.error;
	return {
		source: freshness.src,
		age: freshness.age,
		...(freshness.at === undefined ? {} : { observedAt: freshness.at }),
		...(freshness.cadenceS === undefined
			? {}
			: { cadenceSec: freshness.cadenceS }),
		...(freshness.skewS === undefined ? {} : { skewSec: freshness.skewS }),
		...(failure || freshness.dataFrom !== undefined
			? { error: { dataFrom: freshness.dataFrom, retryAt: failure?.retryAt } }
			: {}),
	};
}

type AppList = Awaited<ReturnType<IAppState["getApps"]>>;

async function noApps(): Promise<AppList> {
	return [];
}

/** App id → display name, from the app list the shell already holds. */
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

export interface AttentionEntryOptions {
	onNavigate: ChromeNavigate;
	/** Runs after an action was chosen (a popover closes). */
	onDone?: () => void;
}

const isRoute = (target: FixAction | DevicesRoute): target is DevicesRoute =>
	"screen" in target;

/** Places a fix has no overlay or route for: the nearest screen that explains it. */
function externalRoute(fix: FixAction): DevicesRoute | undefined {
	if (fix.kind === "see_plans") return { screen: "hub" };
	if (fix.kind === "use_desktop") return { screen: "keys" };
	return undefined;
}

/** Runs an attention or gate target: a place is navigated to, a fix is carried out. */
export function useRunAttentionTarget(options: AttentionEntryOptions) {
	const fix = useFixAction();
	const { onNavigate, onDone } = options;
	return useCallback(
		(target: FixAction | DevicesRoute) => {
			onDone?.();
			if (isRoute(target)) {
				onNavigate(target);
				return;
			}
			if (target.kind === "fix_clock" && target.deviceId) {
				useOverlayStore.getState().openDiagnose(target.deviceId);
				return;
			}
			const outcome = fix(target);
			if (outcome.kind === "navigate") onNavigate(outcome.route);
			const fallback =
				outcome.kind === "external" ? externalRoute(target) : undefined;
			if (fallback) onNavigate(fallback);
		},
		[fix, onNavigate, onDone],
	);
}

function gateOf(t: DevicesT, action: ItemAction | undefined): Gate | null {
	const gate = action?.gate;
	if (!gate || gate.ok) return null;
	return { kind: gate.kind, reason: gateCopy(t, gate).inline };
}

/** The params of an item's sentence that hold an account id. */
const PERSON_PARAMS = ["person", "owner"] as const;

function accountIdsOf(items: readonly AttentionItem[]): string[] {
	return items.flatMap((item) =>
		PERSON_PARAMS.flatMap((name) => {
			const value = item.copy.params?.[name];
			return typeof value === "string" && value !== "" ? [value] : [];
		}),
	);
}

/**
 * Attention items as the list primitive takes them: translated sentence with
 * people by their directory name, source stamp, primary action with its gate,
 * snooze for notices. One adapter for the header popover and every screen
 * that lists attention.
 */
export function useAttentionEntries(
	items: readonly AttentionItem[],
	options: AttentionEntryOptions,
): AttentionEntry[] {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const appName = useAppNames();
	const personName = usePersonNames(accountIdsOf(items));
	const run = useRunAttentionTarget(options);
	const snooze = useSnoozeAttention();
	return useMemo(
		() =>
			items.map((item) => {
				const copy = attentionCopy(t, item, { time, appName, personName });
				const { action } = item;
				return {
					id: item.id,
					severity: item.severity,
					sentence: copy.sentence,
					names: attentionNames(item),
					conditionKey: item.key,
					stamp: stampOf(item.source),
					since: item.firstSeenAt,
					...(action && copy.action
						? {
								action: {
									label: copy.action,
									onSelect: () => run(action.target),
									gate: gateOf(t, action),
								},
							}
						: {}),
					...(item.severity === "notice"
						? { onSnooze: () => void snooze(item) }
						: {}),
				};
			}),
		[items, t, time, appName, personName, run, snooze],
	);
}

const FLEET_ATTENTION: DevicesRoute = {
	screen: "fleet",
	view: "devices",
	focus: "attention",
};

/** SPEC §3.2 item 6: the five most severe open items and "Show all N". */
export function AttentionPopover({
	scope,
	onNavigate,
	appName,
	onClose,
}: Readonly<AttentionPopoverProps>) {
	const { t } = useTranslation("devices");
	const appId = scope.kind === "app" ? scope.appId : undefined;
	const items = useAttention(appId ? { appId } : {});
	const counts = useMemo(() => countAttention(items), [items]);
	const entries = useAttentionEntries(items, { onNavigate, onDone: onClose });
	const names = useAppNames();
	const go = (route: DevicesRoute) => {
		onClose?.();
		onNavigate(route);
	};
	const app = appId
		? (appName ?? names(appId) ?? t("chrome.attention.thisApp", "this app"))
		: undefined;
	return (
		<AttentionPopoverView
			items={entries}
			critical={counts.critical}
			total={counts.total}
			appName={app}
			onShowAll={() =>
				go(appId ? { screen: "app-devices", by: "device" } : FLEET_ATTENTION)
			}
			onAllDevices={appId ? () => go(FLEET_ATTENTION) : undefined}
		/>
	);
}
