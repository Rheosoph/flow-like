"use client";

import { useTranslation } from "@flow-like/locales";
import { ChevronDown, LayoutGrid, Server } from "lucide-react";
import { type ComponentType, useMemo, useState } from "react";
import { useInvoke } from "../../../../hooks/use-invoke";
import { useSearch } from "../../../../hooks/use-search-index";
import type { DevicesRoute } from "../../../../lib/device-management/model/types";
import { useBackend } from "../../../../state/backend-state";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuTrigger,
} from "../../../ui/dropdown-menu";
import { DvInput } from "../primitives/form-fields";
import { cx } from "../primitives/tone";
import { ACCOUNT_SCOPE } from "../routing/devices-route";
import { useDevicesRoute, useRouteLink } from "../routing/use-devices-route";
import { useAppPlacements, useCoverage } from "../workspace";
import { MENU_CONTENT_CLASS, MENU_ITEM_CLASS } from "./area-nav";

export interface ScopeApp {
	id: string;
	name: string;
}

export interface ScopeSwitchViewProps {
	/** `undefined` while the app list loads. */
	apps: readonly ScopeApp[] | undefined;
	/** The app list could not be read. */
	failed?: boolean;
	/** One line under an app's name; mounted only while the menu is open. */
	AppNote?: ComponentType<{ appId: string }>;
	className?: string;
}

/** Rows shown before the filter is needed (R11). */
export const SCOPE_MENU_CAP = 8;

const FLEET: DevicesRoute = { screen: "fleet", view: "devices" };
const APP_HOME: DevicesRoute = { screen: "app-devices", by: "device" };

const SEGMENT_CLASS =
	"inline-flex h-6.5 min-w-0 items-center gap-1.5 rounded-sm px-2.5 text-ui font-medium whitespace-nowrap text-ink-2 hover:bg-row-hover hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring aria-[current=page]:bg-foreground aria-[current=page]:text-background";

function AppMenu({
	apps,
	failed,
	currentAppId,
	AppNote,
}: Readonly<
	Pick<ScopeSwitchViewProps, "apps" | "failed" | "AppNote"> & {
		currentAppId?: string;
	}
>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const [query, setQuery] = useState("");
	const matches = useSearch(apps, query, { fields: ["name", "id"] });
	if (!apps)
		return (
			<p className="px-2 py-1.5 text-ui text-muted-foreground">
				{failed
					? t("shell.scope.appsFailed", "Your apps couldn't be loaded.")
					: t("shell.scope.appsLoading", "Loading your apps…")}
			</p>
		);
	if (apps.length === 0)
		return (
			<p className="px-2 py-1.5 text-ui text-muted-foreground">
				{t("shell.scope.noApps", "You have no apps yet.")}
			</p>
		);
	const shown = matches.slice(0, SCOPE_MENU_CAP);
	const hidden = matches.length - shown.length;
	return (
		<>
			{apps.length > SCOPE_MENU_CAP ? (
				<DvInput
					type="search"
					value={query}
					autoComplete="off"
					aria-label={t("shell.scope.filter", "Filter apps")}
					placeholder={t("shell.scope.filterPlaceholder", "Filter apps")}
					onChange={(event) => setQuery(event.target.value)}
					onKeyDown={(event) => event.stopPropagation()}
					className="mb-1 h-8"
				/>
			) : null}
			{shown.map((app) => (
				<DropdownMenuItem
					key={app.id}
					asChild
					className={cx(MENU_ITEM_CLASS, "items-start")}
				>
					<a
						{...link(APP_HOME, { scope: { kind: "app", appId: app.id } })}
						aria-current={app.id === currentAppId ? "page" : undefined}
					>
						<LayoutGrid aria-hidden className="mt-0.5 size-4" />
						<span className="flex min-w-0 flex-col">
							<span className="truncate font-medium">{app.name}</span>
							{AppNote ? (
								<span className="text-xs text-muted-foreground">
									<AppNote appId={app.id} />
								</span>
							) : null}
						</span>
					</a>
				</DropdownMenuItem>
			))}
			{matches.length === 0 ? (
				<p className="px-2 py-1.5 text-ui text-muted-foreground">
					{t("shell.scope.noMatch", "No app matches that.")}
				</p>
			) : null}
			{hidden > 0 ? (
				<p className="px-2 py-1.5 text-xs text-muted-foreground">
					{t(
						"shell.scope.more",
						"{{count, number}} more. Type to find an app.",
						{ count: hidden },
					)}
				</p>
			) : null}
		</>
	);
}

/** SPEC §3.3: "All devices" or one app's devices. In an app, "All devices" leaves to the account area. */
export function ScopeSwitchView({
	apps,
	failed,
	AppNote,
	className,
}: Readonly<ScopeSwitchViewProps>) {
	const { t } = useTranslation("devices");
	const { scope } = useDevicesRoute();
	const link = useRouteLink();
	const currentAppId = scope.kind === "app" ? scope.appId : undefined;
	const currentApp = apps?.find((app) => app.id === currentAppId);
	return (
		<fieldset
			aria-label={t("shell.scope.label", "Scope")}
			data-shell="scope-switch"
			className={cx(
				"m-0 inline-flex max-w-full min-w-0 gap-0.5 rounded-lg border border-border bg-muted p-0.5 align-middle",
				className,
			)}
		>
			<a
				{...link(FLEET, { scope: ACCOUNT_SCOPE })}
				aria-current={currentAppId ? undefined : "page"}
				className={SEGMENT_CLASS}
			>
				<Server aria-hidden className="size-3.5 shrink-0" />
				{t("shell.scope.all", "All devices")}
			</a>
			<DropdownMenu modal={false}>
				<DropdownMenuTrigger asChild>
					<button
						type="button"
						aria-current={currentAppId ? "page" : undefined}
						className={SEGMENT_CLASS}
					>
						<LayoutGrid aria-hidden className="size-3.5 shrink-0" />
						<span className="truncate">
							{currentAppId
								? (currentApp?.name ?? t("shell.scope.thisApp", "This app"))
								: t("shell.scope.app", "App")}
						</span>
						<ChevronDown aria-hidden className="size-3.5 shrink-0" />
					</button>
				</DropdownMenuTrigger>
				<DropdownMenuContent
					align="end"
					className={cx(MENU_CONTENT_CLASS, "w-[min(320px,calc(100vw-32px))]")}
				>
					<AppMenu
						apps={apps}
						failed={failed}
						currentAppId={currentAppId}
						AppNote={AppNote}
					/>
				</DropdownMenuContent>
			</DropdownMenu>
		</fieldset>
	);
}

/** "on 3 devices you can see": the hub's placement list plus what readable devices report (APP §6.2). */
function AppDeviceCount({ appId }: Readonly<{ appId: string }>) {
	const { t } = useTranslation("devices");
	const cover = useCoverage(appId);
	const placements = useAppPlacements(appId);
	const hub = placements.data;
	const count = useMemo(
		() =>
			new Set([
				...cover.deployed,
				...(hub?.placements.map((placement) => placement.device_id) ?? []),
			]).size,
		[cover.deployed, hub],
	);
	if (!hub && placements.loading)
		return t("shell.scope.counting", "Checking devices…");
	if (hub)
		return count === 0
			? t("shell.scope.onNone", "Not on a device you can see")
			: t("shell.scope.onDevices", "On {{count, number}} devices you can see", {
					count,
				});
	return count === 0
		? t(
				"shell.scope.onNoneReadable",
				"Not on any of the {{readable, number}} devices you can read",
				{ readable: cover.readable },
			)
		: t(
				"shell.scope.onReadable",
				"On {{count, number}} of the {{readable, number}} devices you can read",
				{ count, readable: cover.readable },
			);
}

/** The scope switch of the Fleet overview and App › Devices page headers. */
export function ScopeSwitch({ className }: Readonly<{ className?: string }>) {
	const backend = useBackend();
	const apps = useInvoke(backend.appState.getApps, backend.appState, []);
	const list = useMemo<ScopeApp[] | undefined>(
		() =>
			apps.data
				?.map(([app, meta]) => ({ id: app.id, name: meta?.name ?? app.id }))
				.sort((a, b) => a.name.localeCompare(b.name)),
		[apps.data],
	);
	return (
		<ScopeSwitchView
			apps={list}
			failed={apps.isError}
			AppNote={AppDeviceCount}
			className={className}
		/>
	);
}
