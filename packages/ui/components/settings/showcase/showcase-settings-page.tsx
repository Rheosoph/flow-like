"use client";

import { useTranslation } from "@flow-like/locales";
import {
	CheckIcon,
	ExternalLinkIcon,
	Loader2Icon,
	TriangleAlertIcon,
} from "lucide-react";
import { useRouter, useSearchParams } from "next/navigation";
import { useCallback, useMemo } from "react";
import { useAppPermissions } from "../../../hooks/use-app-permissions";
import { useInvoke } from "../../../hooks/use-invoke";
import { RolePermissions } from "../../../lib/permission/role-permission";
import type { IAppShowcaseSettings } from "../../../lib/schema/app/showcase";
import { useBackend } from "../../../state/backend-state";
import { Button } from "../../ui/button";
import { Card, CardContent } from "../../ui/card";
import { Separator } from "../../ui/separator";
import { Skeleton } from "../../ui/skeleton";
import { SectionLockedPanel } from "../permission/permission-gate";
import { ShowcaseGroupCard } from "./showcase-group-card";
import type { ShowcaseGroupSpec } from "./showcase-group-spec";
import { useShowcaseGroupSpecs } from "./showcase-group-spec";
import {
	allOff,
	allOn,
	readFlag,
	readList,
	withFlag,
	withListItem,
} from "./showcase-settings-model";
import {
	type ShowcaseSaveState,
	useShowcaseSettings,
} from "./use-showcase-settings";

function SaveIndicator({ state }: Readonly<{ state: ShowcaseSaveState }>) {
	const { t } = useTranslation("store");
	if (state === "idle") return null;
	const Icon =
		state === "saving"
			? Loader2Icon
			: state === "saved"
				? CheckIcon
				: TriangleAlertIcon;
	const label =
		state === "saving"
			? t("showcaseSettingSaving", "Saving")
			: state === "saved"
				? t("showcaseSettingSaved", "Saved")
				: t("showcaseSettingNotSaved", "Not saved");
	return (
		<output className="flex items-center gap-1.5 text-xs text-muted-foreground">
			<Icon
				className={
					state === "saving"
						? "size-3.5 animate-spin"
						: state === "error"
							? "size-3.5 text-destructive"
							: "size-3.5"
				}
			/>
			{label}
		</output>
	);
}

export function ShowcaseSettingsPage() {
	const { t } = useTranslation("store");
	const router = useRouter();
	const searchParams = useSearchParams();
	const appId = searchParams.get("id") ?? "";
	const backend = useBackend();
	const permissions = useAppPermissions(appId);
	const canEdit = permissions.can(RolePermissions.Owner);

	const tables = useInvoke(
		backend.dbState.listTables,
		backend.dbState,
		[appId],
		!!appId && canEdit,
	);
	const events = useInvoke(
		backend.eventState.getEvents,
		backend.eventState,
		[appId],
		!!appId && canEdit,
	);
	const tableNames = tables.data;
	const eventItems = useMemo(
		() =>
			events.data?.map((event) => ({
				id: event.id,
				name: event.name,
				type: event.event_type,
			})),
		[events.data],
	);
	const eventIds = useMemo(
		() => eventItems?.map((event) => event.id),
		[eventItems],
	);

	const { settings, update, saveState, isLoading, error } = useShowcaseSettings(
		canEdit ? appId : "",
		{ tables: tableNames, events: eventIds },
	);
	const specs = useShowcaseGroupSpecs(tableNames ?? [], eventItems ?? []);

	const setFlag = useCallback(
		(path: Parameters<typeof withFlag>[1], value: boolean) =>
			settings && update(withFlag(settings, path, value)),
		[settings, update],
	);
	const toggleItem = useCallback(
		(key: "tables" | "events", item: string, ticked: boolean) =>
			settings && update(withListItem(settings, key, item, ticked)),
		[settings, update],
	);

	if (!appId) return null;

	return (
		<div className="mx-auto flex w-full max-w-3xl flex-col gap-5 p-4 pb-8">
			<div className="flex flex-wrap items-start justify-between gap-3">
				<div>
					<h1 className="text-2xl font-bold tracking-tight">
						{t("showcaseSettingTitle", "Store page")}
					</h1>
					<p className="max-w-prose text-sm text-muted-foreground">
						{t(
							"showcaseSettingIntro",
							"Visitors only see what is switched on here. Everything starts off.",
						)}
					</p>
				</div>
				{canEdit && (
					<div className="flex flex-wrap items-center gap-2">
						<SaveIndicator state={saveState} />
						<Button
							variant="outline"
							size="sm"
							disabled={!settings}
							onClick={() => settings && update(allOff())}
						>
							{t("showcaseSettingAllOff", "All off")}
						</Button>
						<Button
							variant="outline"
							size="sm"
							disabled={!settings || !tableNames || !eventIds}
							onClick={() => update(allOn(tableNames ?? [], eventIds ?? []))}
						>
							{t("showcaseSettingAllOn", "All on")}
						</Button>
						<Button size="sm" onClick={() => router.push(`/store?id=${appId}`)}>
							<ExternalLinkIcon className="size-4" />
							{t("showcaseSettingOpenStorePage", "Open store page")}
						</Button>
					</div>
				)}
			</div>

			{!canEdit ? (
				<SectionLockedPanel
					feature={t("showcaseSettingTitle", "Store page")}
					description={t(
						"showcaseSettingLocked",
						"Only owners of this app can choose what its store page shows.",
					)}
					missing={[RolePermissions.Owner]}
					roleName={permissions.roleName}
				/>
			) : error && !settings ? (
				<Card>
					<CardContent className="flex items-center justify-between gap-3 py-4 text-sm">
						<span className="text-destructive">
							{t(
								"showcaseSettingLoadFailed",
								"Could not load the store page settings.",
							)}
						</span>
					</CardContent>
				</Card>
			) : isLoading || !settings ? (
				<div className="flex flex-col gap-3">
					<Skeleton className="h-24 w-full" />
					<Skeleton className="h-40 w-full" />
					<Skeleton className="h-32 w-full" />
				</div>
			) : (
				<div className="flex flex-col gap-3">
					{specs.map((spec: ShowcaseGroupSpec) => (
						<ShowcaseGroupCard
							key={spec.id}
							spec={spec}
							settings={settings as IAppShowcaseSettings}
							readFlag={readFlag}
							readList={readList}
							onFlag={setFlag}
							onItem={toggleItem}
						/>
					))}
					<Separator />
					<p className="text-xs text-muted-foreground">
						{t(
							"showcaseSettingFooter",
							"Hidden items are never sent to visitors.",
						)}
					</p>
				</div>
			)}
		</div>
	);
}
