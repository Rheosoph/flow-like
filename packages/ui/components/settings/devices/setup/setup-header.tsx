"use client";

import { Trans, useTranslation } from "@flow-like/locales";
import { Hourglass } from "lucide-react";
import type { DevicesRoute } from "../../../../lib/device-management/model/types";
import {
	Breadcrumb,
	BreadcrumbItem,
	BreadcrumbLink,
	BreadcrumbList,
	BreadcrumbPage,
	BreadcrumbSeparator,
} from "../../../ui/breadcrumb";
import { useAreaTime } from "../primitives/area-context";
import { DvButton } from "../primitives/dv-button";
import { WizardTitleRow } from "../primitives/wizard";
import { usePendingSetups } from "../workspace";
import { useSetup } from "./setup-context";
import { Mono } from "./setup-parts";

/** Where Exit, the crumb and "Pending setups" lead: the fleet lists the pending setups. */
export const SETUP_EXIT: DevicesRoute = { screen: "fleet", view: "devices" };

function Sub() {
	const { t } = useTranslation("devices");
	const time = useAreaTime();
	const { draft, step, host } = useSetup();
	const name = <Mono>{draft.name}</Mono>;
	if (draft.resumed && draft.created)
		return (
			<Trans
				t={t}
				i18nKey="setup.sub.resumed"
				defaults="Resuming <1/> · created {{when}} · {{host}}"
				values={{ when: time.at(draft.created.createdAt), host }}
				components={{ 1: name }}
			/>
		);
	if (draft.name && step > 1)
		return (
			<Trans
				t={t}
				i18nKey="setup.sub.named"
				defaults="<1/> · on {{host}}"
				values={{ host }}
				components={{ 1: name }}
			/>
		);
	return <>{t("setup.sub.plain", "On {{host}}", { host })}</>;
}

/** SPEC §4.22 wizard chrome: crumbs, the exit left of the title, and the way to the pending setups. */
export function SetupHeader() {
	const { t } = useTranslation("devices");
	const { leave, leaveLink } = useSetup();
	const pending = usePendingSetups().filter(
		(setup) => setup.state !== "cancelled",
	).length;
	const exit = leaveLink(SETUP_EXIT);
	return (
		<header className="flex flex-col gap-3">
			<Breadcrumb aria-label={t("setup.crumb.label", "Breadcrumb")}>
				<BreadcrumbList className="gap-1 text-xs sm:gap-1">
					<BreadcrumbItem>
						<BreadcrumbLink {...exit} className="hover:underline">
							{t("setup.crumb.devices", "Devices")}
						</BreadcrumbLink>
					</BreadcrumbItem>
					<BreadcrumbSeparator className="[&>svg]:size-3" />
					<BreadcrumbItem>
						<BreadcrumbPage className="text-ink-2">
							{t("setup.title", "Set up a device")}
						</BreadcrumbPage>
					</BreadcrumbItem>
				</BreadcrumbList>
			</Breadcrumb>
			<div className="flex flex-wrap items-end justify-between gap-x-6 gap-y-3">
				<WizardTitleRow
					exitLabel={t("setup.exit", "Exit setup")}
					exitTitle={t(
						"setup.exitTitle",
						"Your choices stay in this window, so you can come back and continue.",
					)}
					onExit={() => leave(SETUP_EXIT)}
					title={t("setup.title", "Set up a device")}
					sub={<Sub />}
					className="flex-[1_1_420px]"
				/>
				<DvButton size="sm" variant="ghost" icon={Hourglass} asChild>
					<a {...exit}>
						{t("setup.pending.title", "Pending setups")}
						<span className="font-mono text-xs text-muted-foreground tabular-nums">
							{pending}
						</span>
					</a>
				</DvButton>
			</div>
		</header>
	);
}
