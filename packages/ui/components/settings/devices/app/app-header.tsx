"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import {
	ArrowLeft,
	ChevronDown,
	Copy,
	Ellipsis,
	Layers,
	LayoutList,
	RefreshCw,
	Rocket,
	Settings2,
	Sparkles,
} from "lucide-react";
import { useCallback, useState } from "react";
import type { MatrixRow } from "../../../../lib/device-management/model/app-plan";
import { eligibilityCopy } from "../copy/eligibility-copy";
import {
	type DevicesT,
	useAreaTime,
	useHubFreshness,
} from "../primitives/area-context";
import { DvButton } from "../primitives/dv-button";
import { type Gate, GatedAction } from "../primitives/gate-notice";
import { InlineResult } from "../primitives/inline-result";
import { cx } from "../primitives/tone";
import { copyText } from "../primitives/use-copy";
import { appEventsHref } from "../routing/devices-href";
import { ACCOUNT_SCOPE } from "../routing/devices-route";
import { useRouteLink } from "../routing/use-devices-route";
import { useWidthBucket } from "../workspace";
import {
	APP_LINKS,
	AppMenu,
	LINK,
	LinkButton,
	type MenuEntry,
	NameRef,
	eventIcon,
	useAppPage,
} from "./app-shared";

const servedOn = (row: MatrixRow) =>
	Object.values(row.cells).filter(
		(cell) => cell.state === "served" || cell.state === "staged",
	).length;

/** "Deploy one event…": every event of the newest version, the ones that can run first (APP §2.4). */
function useEventEntries(): () => MenuEntry[] {
	const { t } = useTranslation("devices");
	const { view, data } = useAppPage();
	return useCallback(() => {
		const eligible: MenuEntry[] = view.events.rows.map((row) => {
			const count = servedOn(row);
			return {
				id: `event-${row.eventId}`,
				label: row.name,
				icon: eventIcon(row.eventType, row.eligibility.hosted),
				route: APP_LINKS.deploy({ eventId: row.eventId }),
				note: count
					? t("app.header.eventOn", {
							count,
							defaultValue_one: "on {{count, number}} device",
							defaultValue_other: "on {{count, number}} devices",
						})
					: t("app.header.eventNowhere", "not on a device"),
			};
		});
		const ineligible: MenuEntry[] = view.events.ineligible.map((row) => ({
			id: `event-${row.eventId}`,
			label: row.name,
			icon: eventIcon(row.eventType),
			blocked: row.eligibility.code
				? eligibilityCopy(t, {
						code: row.eligibility.code,
						eventType: row.eventType,
						...(row.eligibility.detail
							? { detail: row.eligibility.detail }
							: {}),
					}).long
				: null,
		}));
		return [
			...eligible,
			...ineligible,
			{
				id: "manage-events",
				label: t("app.header.manageEvents", "Manage events"),
				icon: Settings2,
				href: appEventsHref(data.appId),
				separated: true,
			},
		];
	}, [t, view.events, data.appId]);
}

function useMoreEntries(onRefresh: () => void): () => MenuEntry[] {
	const { t } = useTranslation("devices");
	const { data, view } = useAppPage();
	return useCallback(
		() => [
			{
				id: "refresh",
				label: t("app.header.refresh", "Refresh status"),
				icon: RefreshCw,
				onSelect: onRefresh,
			},
			{
				id: "copy-id",
				label: t("app.header.copyId", "Copy app ID"),
				icon: Copy,
				onSelect: () => void copyText(data.appId),
			},
			{
				id: "fleet",
				label: t("app.header.showInFleet", "Show in Fleet overview"),
				icon: LayoutList,
				route: { screen: "fleet", view: "services", q: view.app.name },
				scope: ACCOUNT_SCOPE,
			},
		],
		[t, data.appId, view.app.name, onRefresh],
	);
}

/** What Refresh status reached: said as it is, never "refreshed" for a read that failed (R9). */
function refreshResult(
	t: DevicesT,
	time: string,
	outcome: { hubFailing: boolean; failed: number },
): string {
	if (outcome.hubFailing)
		return t(
			"devices:app.header.refreshHubFailed",
			"The hub didn't answer at {{time}}: the device list and cloud access are from before.",
			{ time },
		);
	if (outcome.failed)
		return t("devices:app.header.refreshPartly", {
			time,
			count: outcome.failed,
			defaultValue_one:
				"Refreshed at {{time}}, but the status of {{count, number}} device couldn't be read.",
			defaultValue_other:
				"Refreshed at {{time}}, but the status of {{count, number}} devices couldn't be read.",
		});
	return t("devices:app.header.refreshed", "Refreshed at {{time}}", { time });
}

export interface AppHeaderProps {
	/** Why Deploy to devices… can't start now; null when it can. */
	deployGate: Gate | null;
	/** The never-deployed layout carries the page's coral in its empty state. */
	plainDeploy?: boolean;
}

/** APP §2.4: back link, title, the three deploy entries and the More menu. */
export function AppHeader({
	deployGate,
	plainDeploy = false,
}: Readonly<AppHeaderProps>) {
	const { t } = useTranslation("devices");
	const { data, view, openUpdateAll, updateAllGate } = useAppPage();
	const link = useRouteLink();
	const time = useAreaTime();
	const phone = useWidthBucket() === "phone";
	const hub = useHubFreshness();
	const [refreshed, setRefreshed] = useState<{
		at: number;
		failed: number;
	} | null>(null);
	const [refreshing, setRefreshing] = useState(false);
	const { refresh } = data;
	const onRefresh = useCallback(() => {
		setRefreshing(true);
		void refresh()
			.then(
				({ failed }) => failed,
				() => 1,
			)
			.then((failed) => {
				setRefreshing(false);
				setRefreshed({ at: Date.now() / 1000, failed });
			});
	}, [refresh]);
	const eventEntries = useEventEntries();
	const moreEntries = useMoreEntries(onRefresh);

	const more = (
		<AppMenu
			entries={moreEntries}
			trigger={
				<DvButton
					variant="ghost"
					iconOnly
					icon={Ellipsis}
					busy={refreshing}
					data-act="ad-more"
					aria-label={t("app.header.more", "More")}
				/>
			}
		/>
	);
	// On a phone every action spans its cell: side by side where two fit, else one per row.
	const wide = phone ? "w-full" : undefined;
	const updateAll = (
		<GatedAction
			gate={updateAllGate}
			className={phone ? "w-full items-stretch" : undefined}
		>
			<DvButton
				icon={Layers}
				data-act="ad-update-all"
				onClick={openUpdateAll}
				className={wide}
			>
				{t("app.header.updateAll", "Update everywhere…")}
			</DvButton>
		</GatedAction>
	);
	const deployEvent = (
		<AppMenu
			align="start"
			className="min-w-72"
			entries={eventEntries}
			trigger={
				<DvButton icon={Sparkles} data-act="ad-deploy-event" className={wide}>
					{t("app.header.deployEvent", "Deploy one event…")}
					<ChevronDown aria-hidden className="size-3.5 opacity-70" />
				</DvButton>
			}
		/>
	);
	const deploy = (
		<LinkButton
			route={APP_LINKS.deploy()}
			gate={deployGate}
			act="ad-deploy"
			variant={plainDeploy ? "default" : "primary"}
			icon={Rocket}
			className={phone ? "w-full" : undefined}
		>
			{t("app.header.deploy", "Deploy to devices…")}
		</LinkButton>
	);

	return (
		<header data-app-header="" className="flex flex-col gap-3">
			<a
				{...link(
					{ screen: "fleet", view: "devices" },
					{ scope: ACCOUNT_SCOPE },
				)}
				className={cx(
					LINK,
					"inline-flex w-fit items-center gap-1 text-ui underline",
				)}
			>
				<ArrowLeft aria-hidden className="size-3.5" />
				{t("app.header.back", "All devices")}
			</a>
			<div className="flex flex-wrap items-end justify-between gap-x-6 gap-y-3">
				<div className="flex min-w-0 items-start gap-2">
					<div className="min-w-0">
						<h1 className="text-2xl/[30px] font-semibold tracking-[-0.015em]">
							{t("app.header.title", "Devices")}
						</h1>
						<p className="mt-0.5 flex flex-wrap items-center gap-x-1.5 gap-y-1 text-ui text-muted-foreground">
							<span>
								<Trans
									t={t}
									i18nKey="app.header.sub"
									defaults="Where <1/> runs, on devices you can see"
									components={{
										1: (
											<b className="font-semibold text-ink-2">
												{view.app.name}
											</b>
										),
									}}
								/>
							</span>
							<span aria-hidden>·</span>
							<NameRef
								id={data.appId}
								label={t("app.header.appId", "App ID")}
								copyLabel={t("app.header.copyId", "Copy app ID")}
							/>
						</p>
					</div>
					{phone ? <span className="ml-auto">{more}</span> : null}
				</div>
				{phone ? (
					<div className="flex w-full flex-col gap-2">
						{deploy}
						<div className="grid grid-cols-[repeat(auto-fit,minmax(11.5rem,1fr))] items-start gap-2">
							{deployEvent}
							{updateAll}
						</div>
					</div>
				) : (
					<div className="flex flex-wrap items-start gap-2">
						{updateAll}
						{deployEvent}
						{deploy}
						{more}
					</div>
				)}
			</div>
			{refreshed === null ? null : (
				<InlineResult
					tone={hub.failing || refreshed.failed ? "warning" : "good"}
					onDismiss={() => setRefreshed(null)}
				>
					{refreshResult(t, time.clock(refreshed.at), {
						hubFailing: hub.failing,
						failed: refreshed.failed,
					})}
				</InlineResult>
			)}
		</header>
	);
}
