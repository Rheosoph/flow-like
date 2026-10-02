"use client";

import { useTranslation } from "@flow-like/locales";
import { PanelLeft, Search, Server } from "lucide-react";
import { Fragment, type ReactNode } from "react";
import type {
	DeployRoute,
	DevicesRoute,
	DevicesScope,
} from "../../../../lib/device-management/model/types";
import {
	Breadcrumb,
	BreadcrumbItem,
	BreadcrumbLink,
	BreadcrumbList,
	BreadcrumbPage,
	BreadcrumbSeparator,
} from "../../../ui/breadcrumb";
import { DvButton } from "../primitives/dv-button";
import { Kbd } from "../primitives/kbd";
import { cx } from "../primitives/tone";
import { deployExitHref } from "../routing/devices-href";
import {
	useDevicesRoute,
	useHostLink,
	useRouteLink,
} from "../routing/use-devices-route";
import { AreaNav } from "./area-nav";

export interface TopbarCrumb {
	label: string;
	/** Absent on the current page. */
	route?: DevicesRoute;
	/** A page outside the area (the app's Events page), instead of `route`. */
	href?: string;
}

export interface AreaTopbarProps {
	/** `null`: the screen has no rail, or the rail is docked next to it. */
	rail: { open: boolean; onToggle(): void } | null;
	/** App page: the path from the app's Devices page to the open object. The account page shows the area nav. */
	crumbs?: readonly TopbarCrumb[];
	onSearch(): void;
	/** Attention, Keys and Activity buttons; absent while no workspace exists. */
	children?: ReactNode;
	/** Hub pill and platform badge. */
	trailing?: ReactNode;
	className?: string;
}

const FLEET: DevicesRoute = { screen: "fleet", view: "devices" };
const APP_HOME: DevicesRoute = { screen: "app-devices", by: "device" };

/** A deploy that started on the Events page goes back there: "Events › Deploy". */
function deployOrigin(
	route: DeployRoute,
	events: string | undefined,
	scope: DevicesScope | undefined,
): TopbarCrumb | undefined {
	if (route.from !== "events" || !events || scope?.kind !== "app")
		return undefined;
	return { label: events, href: deployExitHref(route, scope) };
}

/** The app page's path: "Devices › edge-berlin-01 › invoice-extractor". The last crumb is the current page. */
export function appCrumbs(
	route: DevicesRoute,
	labels: Readonly<{
		devices: string;
		device: string;
		deploy: string;
		events?: string;
	}>,
	scope?: DevicesScope,
) {
	const home: TopbarCrumb = { label: labels.devices, route: APP_HOME };
	if (route.screen === "device") return [home, { label: labels.device }];
	if (route.screen === "deploy") {
		const origin = deployOrigin(route, labels.events, scope) ?? home;
		return [origin, { label: labels.deploy }];
	}
	if (route.screen !== "service") return [home];
	const device: TopbarCrumb = {
		label: labels.device,
		route: { screen: "device", deviceId: route.deviceId, tab: "services" },
	};
	return [home, device, { label: route.serviceId }];
}

function isMac() {
	return (
		typeof navigator !== "undefined" &&
		/Mac|iPhone|iPad/.test(navigator.platform)
	);
}

function RailToggle({
	rail,
}: Readonly<{ rail: NonNullable<AreaTopbarProps["rail"]> }>) {
	const { t } = useTranslation("devices");
	return (
		<DvButton
			variant="ghost"
			iconOnly
			icon={PanelLeft}
			data-shell="rail-toggle"
			aria-expanded={rail.open}
			aria-label={
				rail.open
					? t("shell.topbar.hideDevices", "Hide devices")
					: t("shell.topbar.showDevices", "Show devices")
			}
			className={cx("shrink-0", rail.open && "bg-row-selected")}
			onClick={rail.onToggle}
		/>
	);
}

const NARROW_HIDDEN = "@max-[720px]/devices:hidden";

/** Object names truncate, the home crumb never does; below 720 px the crumbs above the parent hide. */
function TopbarCrumbs({
	crumbs,
}: Readonly<{ crumbs: readonly TopbarCrumb[] }>) {
	const { t } = useTranslation("devices");
	const link = useRouteLink();
	const hostLink = useHostLink();
	const last = crumbs.length - 1;
	const target = (crumb: TopbarCrumb, index: number) => {
		if (index === last) return undefined;
		if (crumb.route) return link(crumb.route);
		return crumb.href ? hostLink(crumb.href) : undefined;
	};
	return (
		<Breadcrumb
			aria-label={t("shell.topbar.breadcrumb", "Breadcrumb")}
			className="min-w-0"
		>
			<BreadcrumbList className="flex-nowrap gap-1 text-[13px]/[18px] sm:gap-1">
				{crumbs.map((crumb, index) => {
					const props = target(crumb, index);
					return (
						<Fragment key={`${index}:${crumb.label}`}>
							{index > 0 ? (
								<BreadcrumbSeparator
									className={cx(
										"shrink-0 text-border-strong [&>svg]:size-3",
										index < last && NARROW_HIDDEN,
									)}
								/>
							) : null}
							<BreadcrumbItem
								className={cx(
									index === 0 || index < last - 1 ? "shrink-0" : "min-w-0",
									index < last - 1 && NARROW_HIDDEN,
								)}
							>
								{props ? (
									<BreadcrumbLink
										{...props}
										className="truncate hover:underline"
									>
										{crumb.label}
									</BreadcrumbLink>
								) : (
									<BreadcrumbPage className="truncate font-semibold">
										{crumb.label}
									</BreadcrumbPage>
								)}
							</BreadcrumbItem>
						</Fragment>
					);
				})}
			</BreadcrumbList>
		</Breadcrumb>
	);
}

function SearchButton({
	label,
	onSearch,
}: Readonly<{ label: string; onSearch(): void }>) {
	return (
		<button
			type="button"
			data-shell="search"
			aria-label={label}
			onClick={onSearch}
			className="@container/cmdk inline-flex h-8 min-w-8.5 flex-[0_30_300px] items-center gap-2 overflow-hidden rounded-lg border border-border bg-card pr-1.5 pl-2.5 text-left text-ui text-muted-foreground hover:border-border-strong focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring @max-[720px]/devices:w-8 @max-[720px]/devices:min-w-8 @max-[720px]/devices:flex-none @max-[720px]/devices:justify-center @max-[720px]/devices:border-transparent @max-[720px]/devices:bg-transparent @max-[720px]/devices:px-0"
		>
			<Search aria-hidden className="size-4 shrink-0" />
			<span className="min-w-0 flex-1 truncate @max-[150px]/cmdk:hidden @max-[720px]/devices:hidden">
				{label}
			</span>
			<Kbd className="shrink-0 @max-[150px]/cmdk:hidden @max-[720px]/devices:hidden">
				{isMac() ? "⌘K" : "Ctrl K"}
			</Kbd>
		</button>
	);
}

/** SPEC §3.2: 48 px, one line at every width; labels hide below 720 px of area width. */
export function AreaTopbar({
	rail,
	crumbs,
	onSearch,
	children,
	trailing,
	className,
}: Readonly<AreaTopbarProps>) {
	const { t } = useTranslation("devices");
	const { host } = useDevicesRoute();
	const link = useRouteLink();
	const inApp = host === "app";
	return (
		<header
			data-shell="topbar"
			className={cx(
				"flex h-12 shrink-0 items-center gap-1 overflow-hidden border-b border-hairline bg-background px-3 whitespace-nowrap @max-[720px]/devices:gap-0.5 @max-[720px]/devices:px-2",
				className,
			)}
		>
			{rail ? <RailToggle rail={rail} /> : null}
			{inApp ? (
				<TopbarCrumbs crumbs={crumbs ?? []} />
			) : (
				<>
					<a
						{...link(FLEET)}
						aria-label={t("shell.topbar.home", "Devices home")}
						className="mr-1.5 ml-0.5 inline-flex size-5.5 shrink-0 items-center justify-center rounded-[5px] bg-foreground text-background focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ring @max-[480px]/devices:hidden"
					>
						<Server aria-hidden className="size-3.5" />
					</a>
					<AreaNav />
				</>
			)}
			<span className="min-w-1 flex-1 @max-[720px]/devices:min-w-0" />
			<SearchButton
				label={
					inApp
						? t(
								"shell.topbar.searchApp",
								"Search this app's devices and services",
							)
						: t("shell.topbar.searchAll", "Search devices, services, actions")
				}
				onSearch={onSearch}
			/>
			{children}
			{trailing}
		</header>
	);
}
