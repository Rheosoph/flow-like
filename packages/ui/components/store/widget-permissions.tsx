"use client";

import { useTranslation } from "@flow-like/locales";
import { BellOff, Loader2, Radio, ShieldCheck, ShieldOff } from "lucide-react";
import { useCallback, useMemo, useState, useSyncExternalStore } from "react";
import { toast } from "sonner";
import { getErrorMessage } from "../../lib/error-message";
import { widgetSourceHost } from "../../lib/package-capabilities";
import { cn } from "../../lib/utils";
import { useBackend } from "../../state/backend-state";
import {
	type MicroWidgetConsentEntry,
	microWidgetConsentKey,
	subscribeMicroWidgetConsent,
} from "../a2ui/micro-widget-capability-consent";
import {
	isEmptyPolicy,
	policyCapabilities,
	policyHosts,
} from "../a2ui/micro-widget-policy";
import { WidgetSourceLevelBadge } from "../a2ui/micro-widget-purpose-card";
import { Button } from "../ui/button";
import { RelativeTime } from "../ui/relative-time";
import {
	Sheet,
	SheetContent,
	SheetDescription,
	SheetFooter,
	SheetHeader,
	SheetTitle,
} from "../ui/sheet";
import { Tooltip, TooltipContent, TooltipTrigger } from "../ui/tooltip";
import {
	WidgetCapabilityBadges,
	useWidgetPolicyLabels,
} from "./widget-network-access";
import {
	type MicroWidgetConsentQuery,
	type WidgetConsentRuntimeRow,
	type WidgetConsentSourceRow,
	askAgainForWidgetRuntime,
	clearPackageWidgetPermissions,
	listWidgetConsentEntries,
	revokeAppWidgetPermissions,
	revokeWidgetPermissions,
	widgetConsentRuntimeRows,
	widgetConsentSourceRows,
} from "./widget-permissions-actions";

export {
	type MicroWidgetConsentQuery,
	askAgainForWidgetRuntime,
	clearPackageWidgetPermissions,
	listWidgetConsentEntries,
	revokeAppWidgetPermissions,
	revokeWidgetPermissions,
};

const NO_ENTRIES = "[]";

/** Grants that currently apply on this device, kept in step with other tabs. */
export function useMicroWidgetConsentEntries(
	query: MicroWidgetConsentQuery = {},
): MicroWidgetConsentEntry[] {
	const { appId, packageId, widgetId } = query;
	const read = useCallback(
		() =>
			JSON.stringify(listWidgetConsentEntries({ appId, packageId, widgetId })),
		[appId, packageId, widgetId],
	);
	const snapshot = useSyncExternalStore(
		subscribeMicroWidgetConsent,
		read,
		() => NO_ENTRIES,
	);
	return useMemo(
		() => JSON.parse(snapshot) as MicroWidgetConsentEntry[],
		[snapshot],
	);
}

type StoreTranslate = ReturnType<typeof useTranslation<"store">>["t"];

/** Consent is already gone when this shows; only grants issued earlier outlive it. */
function revokeFailedMessage(t: StoreTranslate, error: unknown): string {
	return t(
		"widgetGrantRevokeFailed",
		"Permissions were revoked, but grants already issued to these widgets stay valid until they expire: {{message}}",
		{ message: getErrorMessage(error) },
	);
}

function entryKey(entry: MicroWidgetConsentEntry): string {
	return `${microWidgetConsentKey(entry.target)}:${entry.scope}`;
}

/** A record that only says "stop asking" grants nothing, so it gets no scope line. */
function grantsSomething(entry: MicroWidgetConsentEntry): boolean {
	return !isEmptyPolicy(entry.policy) || entry.runtime.length > 0;
}

function useScopeLabel() {
	const { t } = useTranslation("store");
	return useCallback(
		(entry: MicroWidgetConsentEntry) => {
			if (entry.legacy) {
				return t(
					"widgetPermissionScopeLegacy",
					"Always allowed (from an earlier version)",
				);
			}
			return entry.scope === "app"
				? t("widgetPermissionScopeApp", "Always allowed for this project")
				: t("widgetPermissionScopeSession", "Allowed for this session");
		},
		[t],
	);
}

export interface WidgetPermissionsListProps {
	entries: readonly MicroWidgetConsentEntry[];
	/** Display names by package id; the id is shown when a name is missing. */
	packageNames?: ReadonlyMap<string, string>;
	busy?: boolean;
	onRevoke: (entry: MicroWidgetConsentEntry) => void;
	onAskAgain?: (entry: MicroWidgetConsentEntry) => void;
}

export function WidgetPermissionsList({
	entries,
	packageNames,
	busy = false,
	onRevoke,
	onAskAgain = askAgainForWidgetRuntime,
}: WidgetPermissionsListProps) {
	const { t } = useTranslation("store");
	if (entries.length === 0) {
		return (
			<p
				className="rounded-lg border border-dashed p-6 text-center text-sm text-muted-foreground"
				data-widget-permissions-empty
			>
				{t(
					"widgetPermissionsEmpty",
					"No widget permissions are stored for this project on this device.",
				)}
			</p>
		);
	}
	return (
		<ul className="space-y-3" data-widget-permissions-list>
			{entries.map((entry) => (
				<WidgetPermissionItem
					key={entryKey(entry)}
					entry={entry}
					packageName={packageNames?.get(entry.target.packageId)}
					busy={busy}
					onRevoke={onRevoke}
					onAskAgain={onAskAgain}
				/>
			))}
		</ul>
	);
}

function ConsentAddressRow({
	source,
	directives,
	level,
	at,
}: WidgetConsentSourceRow & { at?: number }) {
	const labels = useWidgetPolicyLabels();
	const runtime = at !== undefined;
	return (
		<li
			className="flex flex-wrap items-center gap-x-2 gap-y-0.5"
			{...(runtime
				? { "data-widget-permission-runtime": source }
				: { "data-widget-permission-source": source })}
		>
			{runtime && (
				<Radio
					aria-hidden="true"
					className="h-3 w-3 shrink-0 text-muted-foreground"
				/>
			)}
			<bdi dir="ltr" translate="no" className="break-all font-mono text-xs">
				{source}
			</bdi>
			<span className="text-xs text-muted-foreground">
				{labels.directives(directives)}
			</span>
			{level && <WidgetSourceLevelBadge level={level} />}
			{runtime && (
				<RelativeTime value={at} className="text-xs text-muted-foreground" />
			)}
		</li>
	);
}

function ConsentSourceList({
	rows,
}: {
	rows: readonly WidgetConsentSourceRow[];
}) {
	if (rows.length === 0) return null;
	return (
		<ul className="space-y-1" data-widget-permission-sources>
			{rows.map((row) => (
				<ConsentAddressRow key={row.source} {...row} />
			))}
		</ul>
	);
}

function ConsentRuntimeList({
	rows,
}: {
	rows: readonly WidgetConsentRuntimeRow[];
}) {
	const { t } = useTranslation("store");
	if (rows.length === 0) return null;
	return (
		<div className="space-y-1" data-widget-permission-runtime-list>
			<p className="text-xs text-muted-foreground">
				{t("widgetPermissionRuntimeEntries", {
					defaultValue_one: "{{count}} address provided while the app ran",
					defaultValue_other: "{{count}} addresses provided while the app ran",
					count: rows.length,
				})}
			</p>
			<ul className="space-y-1">
				{rows.map((row) => (
					<ConsentAddressRow key={row.source} {...row} />
				))}
			</ul>
		</div>
	);
}

function WidgetPermissionItem({
	entry,
	packageName,
	busy,
	onRevoke,
	onAskAgain,
}: {
	entry: MicroWidgetConsentEntry;
	packageName?: string;
	busy: boolean;
	onRevoke: (entry: MicroWidgetConsentEntry) => void;
	onAskAgain: (entry: MicroWidgetConsentEntry) => void;
}) {
	const { t } = useTranslation("store");
	const labels = useWidgetPolicyLabels();
	const scopeLabel = useScopeLabel();
	const { target, policy } = entry;
	const capabilities = useMemo(() => policyCapabilities(policy), [policy]);
	const sources = useMemo(() => widgetConsentSourceRows(entry), [entry]);
	const runtime = useMemo(() => widgetConsentRuntimeRows(entry), [entry]);
	return (
		<li
			className="space-y-2 rounded-lg border p-3"
			data-widget-permission={microWidgetConsentKey(target)}
		>
			<div className="flex items-start justify-between gap-2">
				<div className="min-w-0 space-y-0.5">
					<p className="truncate text-sm font-medium">{target.widgetId}</p>
					<p className="flex min-w-0 flex-wrap items-baseline gap-x-1 text-xs text-muted-foreground">
						<bdi className="max-w-full truncate">
							{packageName ?? target.packageId}
						</bdi>
						<span aria-hidden="true">·</span>
						<span className="wrap-break-word" data-widget-permission-origin>
							{labels.origin(target.source)}
						</span>
					</p>
					{grantsSomething(entry) && (
						<p className="text-xs text-muted-foreground">
							{scopeLabel(entry)}
							{entry.grantedAt > 0 && (
								<>
									{" · "}
									<RelativeTime value={entry.grantedAt} />
								</>
							)}
						</p>
					)}
				</div>
				<Button
					variant="outline"
					size="sm"
					className="shrink-0"
					disabled={busy}
					onClick={() => onRevoke(entry)}
				>
					{t("widgetPermissionRevoke", "Revoke")}
				</Button>
			</div>
			<WidgetCapabilityBadges capabilities={capabilities} />
			<ConsentSourceList rows={sources} />
			<ConsentRuntimeList rows={runtime} />
			{entry.runtimeMuted && (
				<div
					className="flex flex-wrap items-center justify-between gap-2 rounded-md bg-muted/40 px-2 py-1.5"
					data-widget-permission-muted
				>
					<span className="flex items-center gap-1.5 text-xs text-muted-foreground">
						<BellOff aria-hidden="true" className="h-3 w-3 shrink-0" />
						{t(
							"widgetPermissionRuntimeMuted",
							"Not asking about new addresses",
						)}
					</span>
					<Button
						variant="link"
						size="sm"
						className="h-auto min-h-6 p-0 text-xs"
						disabled={busy}
						onClick={() => onAskAgain(entry)}
						data-widget-permission-ask-again
					>
						{t("widgetPermissionAskAgain", "Ask again")}
					</Button>
				</div>
			)}
		</li>
	);
}

export interface WidgetPermissionsSheetProps {
	appId: string;
	entries: readonly MicroWidgetConsentEntry[];
	packageNames?: ReadonlyMap<string, string>;
	open: boolean;
	onOpenChange: (open: boolean) => void;
}

/** Project settings: what this device allows each package widget to do, with revoke. */
export function WidgetPermissionsSheet({
	appId,
	entries,
	packageNames,
	open,
	onOpenChange,
}: WidgetPermissionsSheetProps) {
	const { t } = useTranslation("store");
	const backend = useBackend();
	const [busy, setBusy] = useState(false);

	const run = useCallback(
		async (action: () => Promise<unknown>) => {
			setBusy(true);
			try {
				await action();
				toast.success(
					t("widgetPermissionsRevoked", "Widget permissions revoked"),
				);
			} catch (error) {
				toast.error(revokeFailedMessage(t, error));
			} finally {
				setBusy(false);
			}
		},
		[t],
	);

	const revokeOne = useCallback(
		(entry: MicroWidgetConsentEntry) =>
			run(() => revokeWidgetPermissions([entry], backend.registryState)),
		[run, backend],
	);
	const revokeAll = useCallback(
		() => run(() => revokeAppWidgetPermissions(appId, backend.registryState)),
		[run, backend, appId],
	);

	return (
		<Sheet open={open} onOpenChange={onOpenChange}>
			<SheetContent className="w-full sm:max-w-md">
				<SheetHeader>
					<SheetTitle>
						{t("widgetPermissions", "Widget permissions")}
					</SheetTitle>
					<SheetDescription>
						{t(
							"widgetPermissionsDescription",
							"What package widgets in this project may do on this device. After you revoke a permission, the widget asks again the next time it opens.",
						)}
					</SheetDescription>
				</SheetHeader>
				<div className="min-h-0 flex-1 overflow-y-auto px-4">
					<WidgetPermissionsList
						entries={entries}
						packageNames={packageNames}
						busy={busy}
						onRevoke={revokeOne}
					/>
				</div>
				<SheetFooter>
					<Button
						variant="outline"
						className="text-destructive hover:text-destructive"
						disabled={busy || entries.length === 0}
						onClick={revokeAll}
					>
						{busy ? (
							<Loader2 className="mr-2 h-4 w-4 animate-spin" />
						) : (
							<ShieldOff className="mr-2 h-4 w-4" />
						)}
						{t("widgetPermissionsRevokeAll", "Revoke all")}
					</Button>
				</SheetFooter>
			</SheetContent>
		</Sheet>
	);
}

export function WidgetPermissionsButton({
	count,
	onClick,
}: {
	count: number;
	onClick: () => void;
}) {
	const { t } = useTranslation("store");
	return (
		<Button size="sm" variant="outline" onClick={onClick}>
			<ShieldCheck className="mr-2 h-4 w-4" />
			{t("widgetPermissions", "Widget permissions")}
			{count > 0 && (
				<span className="ml-2 rounded-full bg-muted px-1.5 text-xs tabular-nums text-muted-foreground">
					{count}
				</span>
			)}
		</Button>
	);
}

/** Inspector provenance header: what this device allows the selected widget to do. */
export function WidgetConsentStatus({
	appId,
	packageId,
	widgetId,
}: {
	appId: string | null | undefined;
	packageId: string;
	widgetId: string;
}) {
	const { t } = useTranslation("store");
	const backend = useBackend();
	const labels = useWidgetPolicyLabels();
	const scopeLabel = useScopeLabel();
	const entries = useMicroWidgetConsentEntries({
		appId: appId ?? null,
		packageId,
		widgetId,
	});

	const revoke = useCallback(async () => {
		try {
			await revokeWidgetPermissions(entries, backend.registryState);
		} catch (error) {
			toast.error(revokeFailedMessage(t, error));
		}
	}, [entries, backend, t]);

	if (entries.length === 0) {
		return (
			<p
				className="text-[10px] text-muted-foreground"
				data-widget-consent-status="none"
			>
				{t("widgetConsentNone", "No widget permissions granted on this device")}
			</p>
		);
	}

	const hosts = [
		...new Set(entries.flatMap(({ policy }) => policyHosts(policy))),
	].sort();
	const runtimeHosts = [
		...new Set(
			entries.flatMap(({ runtime }) =>
				runtime.map((entry) => widgetSourceHost(entry.s)),
			),
		),
	].sort();
	const capabilities = [
		...new Set(entries.flatMap(({ policy }) => policyCapabilities(policy))),
	];
	const summary = [
		...new Set(entries.filter(grantsSomething).map(scopeLabel)),
		...(hosts.length > 0
			? [
					t("widgetNetworkAddresses", {
						defaultValue_one: "Network: {{count}} address",
						defaultValue_other: "Network: {{count}} addresses",
						count: hosts.length,
					}),
				]
			: []),
		...(runtimeHosts.length > 0
			? [
					t("widgetPermissionRuntimeEntries", {
						defaultValue_one: "{{count}} address provided while the app ran",
						defaultValue_other:
							"{{count}} addresses provided while the app ran",
						count: runtimeHosts.length,
					}),
				]
			: []),
		...(entries.some((entry) => entry.runtimeMuted)
			? [t("widgetPermissionRuntimeMuted", "Not asking about new addresses")]
			: []),
		...capabilities.map(labels.capability),
	];
	const allHosts = [...new Set([...hosts, ...runtimeHosts])];
	return (
		<div
			className="flex items-start justify-between gap-2 text-[10px]"
			data-widget-consent-status="granted"
		>
			<p
				className="min-w-0 text-muted-foreground"
				title={allHosts.length > 0 ? allHosts.join("\n") : undefined}
			>
				{summary.join(" · ")}
			</p>
			<Button
				variant="link"
				size="sm"
				className="h-auto shrink-0 p-0 text-[10px]"
				onClick={revoke}
			>
				{t("widgetPermissionRevoke", "Revoke")}
			</Button>
		</div>
	);
}

/** Installed packages: forget every widget decision for one package on this device. */
export function ClearWidgetPermissionsButton({
	packageId,
	packageName,
	className,
}: {
	packageId: string;
	packageName: string;
	className?: string;
}) {
	const { t } = useTranslation("settings");
	const backend = useBackend();
	const [busy, setBusy] = useState(false);
	const label = t(
		"clearWidgetPermissions",
		"Clear widget permissions on this device",
	);

	const clear = useCallback(async () => {
		setBusy(true);
		try {
			const count = await clearPackageWidgetPermissions(
				packageId,
				backend.registryState,
			);
			if (count === 0) {
				toast.info(
					t(
						"noWidgetPermissionsStored",
						"No widget permissions were stored for {{name}} on this device",
						{ name: packageName },
					),
				);
			} else {
				toast.success(
					t("widgetPermissionsCleared", {
						defaultValue_one:
							"Cleared {{count}} widget permission for {{name}}",
						defaultValue_other:
							"Cleared {{count}} widget permissions for {{name}}",
						count,
						name: packageName,
					}),
				);
			}
		} catch (error) {
			toast.error(
				t(
					"clearWidgetPermissionsFailed",
					"Widget permissions were cleared, but grants already issued to its widgets stay valid until they expire: {{message}}",
					{ message: getErrorMessage(error) },
				),
			);
		} finally {
			setBusy(false);
		}
	}, [backend, packageId, packageName, t]);

	return (
		<Tooltip>
			<TooltipTrigger asChild>
				<Button
					variant="ghost"
					size="icon"
					className={cn("h-8 w-8", className)}
					aria-label={label}
					disabled={busy}
					onClick={(event) => {
						event.stopPropagation();
						void clear();
					}}
				>
					{busy ? (
						<Loader2 className="h-3.5 w-3.5 animate-spin" />
					) : (
						<ShieldOff className="h-3.5 w-3.5" />
					)}
				</Button>
			</TooltipTrigger>
			<TooltipContent>{label}</TooltipContent>
		</Tooltip>
	);
}
