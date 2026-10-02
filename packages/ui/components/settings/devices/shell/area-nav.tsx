"use client";

import { useTranslation } from "@flow-like/locales";
import { ChevronDown } from "lucide-react";
import type { DevicesRoute } from "../../../../lib/device-management/model/types";
import {
	DropdownMenu,
	DropdownMenuContent,
	DropdownMenuItem,
	DropdownMenuTrigger,
} from "../../../ui/dropdown-menu";
import { DvButton } from "../primitives/dv-button";
import { cx } from "../primitives/tone";
import { useDevicesRoute, useRouteLink } from "../routing/use-devices-route";

export const AREA_SECTIONS = [
	"devices",
	"access",
	"certificates",
	"keys",
	"hub",
] as const;
export type AreaSection = (typeof AREA_SECTIONS)[number];

export const SECTION_ROUTE: Record<AreaSection, DevicesRoute> = {
	devices: { screen: "fleet", view: "devices" },
	access: { screen: "access" },
	certificates: { screen: "certificates" },
	keys: { screen: "keys" },
	hub: { screen: "hub" },
};

/** Fleet, device, service, app page and both wizards belong to "Devices". */
export function areaSection(route: DevicesRoute): AreaSection {
	switch (route.screen) {
		case "access":
		case "certificates":
		case "keys":
		case "hub":
			return route.screen;
		default:
			return "devices";
	}
}

/** Menu surfaces of the shell: opaque, no shadow, and a neutral focus instead of the coral accent (R2, R13). */
export const MENU_CONTENT_CLASS =
	"border-border-strong bg-popover shadow-none backdrop-blur-none";
export const MENU_ITEM_CLASS =
	"text-[13px]/[18px] focus:bg-row-hover focus:text-foreground";

const LINK_CLASS =
	"inline-flex h-8 items-center rounded-lg px-2.5 text-ui font-medium whitespace-nowrap text-muted-foreground hover:bg-row-hover hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring aria-[current=page]:bg-row-selected aria-[current=page]:text-foreground";

export function useSectionLabels(): Record<AreaSection, string> {
	const { t } = useTranslation("devices");
	return {
		devices: t("shell.nav.devices", "Devices"),
		access: t("shell.nav.access", "Access"),
		certificates: t("shell.nav.certificates", "Certificates"),
		keys: t("shell.nav.keys", "Keys & recovery"),
		hub: t("shell.nav.hub", "Hub status"),
	};
}

/** SPEC §3.2 item 3: the five sections as links; one menu button below 1080 px of area width. */
export function AreaNav({ className }: Readonly<{ className?: string }>) {
	const { t } = useTranslation("devices");
	const { route, navigate } = useDevicesRoute();
	const link = useRouteLink();
	const labels = useSectionLabels();
	const current = areaSection(route);
	return (
		<nav
			aria-label={t("shell.nav.label", "Devices area")}
			className={cx("flex min-w-0 items-center", className)}
		>
			<div
				data-nav="links"
				className="flex items-center gap-0.5 @max-[1080px]/devices:hidden"
			>
				{AREA_SECTIONS.map((section) => (
					<a
						key={section}
						{...link(SECTION_ROUTE[section])}
						aria-current={section === current ? "page" : undefined}
						className={LINK_CLASS}
					>
						{labels[section]}
					</a>
				))}
			</div>
			<div
				data-nav="menu"
				className="hidden min-w-0 @max-[1080px]/devices:flex"
			>
				<DropdownMenu modal={false}>
					<DropdownMenuTrigger asChild>
						<DvButton
							variant="ghost"
							aria-label={t("shell.nav.switch", "{{section}}: switch area", {
								section: labels[current],
							})}
							title={labels[current]}
							className="min-w-0 shrink @max-[720px]/devices:px-1.5 @max-[720px]/devices:has-[>svg]:px-1.5"
						>
							<span className="truncate">{labels[current]}</span>
							<ChevronDown aria-hidden className="size-3.5 shrink-0" />
						</DvButton>
					</DropdownMenuTrigger>
					<DropdownMenuContent align="start" className={MENU_CONTENT_CLASS}>
						{AREA_SECTIONS.map((section) => (
							<DropdownMenuItem
								key={section}
								aria-current={section === current ? "page" : undefined}
								className={cx(
									MENU_ITEM_CLASS,
									section === current && "font-semibold",
								)}
								onSelect={() => navigate(SECTION_ROUTE[section])}
							>
								{labels[section]}
							</DropdownMenuItem>
						))}
					</DropdownMenuContent>
				</DropdownMenu>
			</div>
		</nav>
	);
}
