"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { LayoutGrid } from "lucide-react";
import type { DevicesRoute } from "../../../../lib/device-management/model/types";
import { DvButton } from "../primitives/dv-button";
import { cx } from "../primitives/tone";
import { ACCOUNT_SCOPE } from "../routing/devices-route";
import { useDevicesRoute, useRouteLink } from "../routing/use-devices-route";

const APP_HOME: DevicesRoute = { screen: "app-devices", by: "device" };

/**
 * SPEC §3.3: a device or service opened from an app shows only that app's
 * services, metrics and activity. "Show whole device" leaves to the account area.
 */
export function ContextBar({
	appName,
	canShowWholeDevice,
	className,
}: Readonly<{
	appName: string;
	/** The viewer owns the device or may view its whole status. */
	canShowWholeDevice: boolean;
	className?: string;
}>) {
	const { t } = useTranslation("devices");
	const { route, navigate } = useDevicesRoute();
	const link = useRouteLink();
	if (route.screen !== "device" && route.screen !== "service") return null;
	return (
		<div
			data-shell="context-bar"
			className={cx(
				"flex flex-wrap items-center gap-x-1.5 gap-y-1 rounded-lg border border-border bg-surface-sunken px-3 py-2 text-ui",
				className,
			)}
		>
			<LayoutGrid
				aria-hidden
				className="size-4 shrink-0 text-muted-foreground"
			/>
			<span className="min-w-0">
				<Trans
					t={t}
					i18nKey="shell.context.app"
					defaults="App: <1/>"
					components={{
						1: (
							<a
								{...link(APP_HOME)}
								className="font-semibold underline decoration-border-strong underline-offset-2 hover:decoration-current"
							>
								{appName}
							</a>
						),
					}}
				/>
			</span>
			<span className="text-muted-foreground">
				{t(
					"shell.context.filtered",
					"· Services, metrics and activity are filtered to this app.",
				)}
			</span>
			{canShowWholeDevice ? (
				<DvButton
					variant="link"
					onClick={() => navigate(route, { scope: ACCOUNT_SCOPE })}
				>
					{t("shell.context.showWhole", "Show whole device")}
				</DvButton>
			) : null}
		</div>
	);
}
