"use client";

import { useTranslation } from "@flow-like/locales";
import { ShieldCheck } from "lucide-react";
import { useMemo } from "react";
import {
	type PackageAccess,
	groupAccessTags,
} from "../../../lib/app-package-overview";
import {
	describePackageWidgetNetwork,
	usePackageCapabilities,
} from "../../../lib/package-capabilities";
import type { PackagePinState } from "../../../lib/package-license";
import type { PackageWidgetEntry } from "../../../lib/schema/wasm";
import { cn } from "../../../lib/utils";
import {
	type MicroWidgetCapability,
	policyCapabilities,
} from "../../a2ui/micro-widget-policy";
import { Badge } from "../../ui/badge";
import { Button } from "../../ui/button";
import {
	Table,
	TableBody,
	TableCell,
	TableHead,
	TableHeader,
	TableRow,
} from "../../ui/table";
import { WidgetCapabilityBadges } from "../widget-network-access";
import {
	ELEVATED_CHIP_CLASS,
	PackageStatusBadge,
	Section,
	useAccessLabels,
} from "./parts";

const MAX_VISIBLE_HOSTS = 2;

export interface PackageAccessRow {
	packageId: string;
	name: string;
	access: PackageAccess;
	widgets: readonly PackageWidgetEntry[];
	pinState: PackagePinState;
	timeLeftLabel?: string;
}

export function PackageAccessSection({
	rows,
	grantCount,
	onManageGrants,
}: Readonly<{
	rows: readonly PackageAccessRow[];
	grantCount: number;
	onManageGrants: () => void;
}>) {
	const { t } = useTranslation("store");

	return (
		<Section
			title={t("appPackagesAccess", "Access")}
			meta={t(
				"appPackagesAccessMeta",
				"What each package may do when a flow runs",
			)}
		>
			<div className="overflow-hidden rounded-xl border bg-card shadow-xs">
				<Table className="my-0">
					<TableHeader>
						<TableRow className="hover:bg-transparent">
							<TableHead className="pl-4">
								{t("appPackagesColumnPackage", "Package")}
							</TableHead>
							<TableHead>{t("network", "Network")}</TableHead>
							<TableHead>{t("appPackagesColumnFiles", "Files")}</TableHead>
							<TableHead>{t("appPackagesColumnLimits", "Limits")}</TableHead>
							<TableHead>{t("appPackagesColumnOther", "Other")}</TableHead>
							<TableHead className="pr-4">
								{t("appPackagesWidgets", "Widgets")}
							</TableHead>
						</TableRow>
					</TableHeader>
					<TableBody>
						{rows.map((row) => (
							<AccessRow key={row.packageId} row={row} />
						))}
					</TableBody>
				</Table>
			</div>
			<p className="flex flex-wrap items-center gap-x-2 gap-y-1 text-sm text-muted-foreground">
				<ShieldCheck className="size-4 shrink-0" aria-hidden="true" />
				{grantCount > 0
					? t("appPackagesGrantsOnDevice", {
							defaultValue_one:
								"{{count}} widget permission granted on this device.",
							defaultValue_other:
								"{{count}} widget permissions granted on this device.",
							count: grantCount,
						})
					: t(
							"appPackagesNoGrantsOnDevice",
							"No widget permissions granted on this device.",
						)}
				<Button
					variant="link"
					size="sm"
					className="h-auto p-0"
					onClick={onManageGrants}
				>
					{t("appPackagesManageGrants", "Manage")}
				</Button>
			</p>
		</Section>
	);
}

function Empty() {
	return <span className="text-muted-foreground">—</span>;
}

function None() {
	const { t } = useTranslation("store");
	return (
		<span className="text-muted-foreground">
			{t("appPackagesNone", "None")}
		</span>
	);
}

function AccessRow({ row }: Readonly<{ row: PackageAccessRow }>) {
	const { t } = useTranslation("store");
	const labels = useAccessLabels();
	const groups = useMemo(
		() => groupAccessTags(row.access.tags),
		[row.access.tags],
	);
	const otherTags = useMemo(
		() => [
			...groups.accounts,
			...groups.models,
			...groups.data,
			...groups.runtime,
		],
		[groups],
	);
	const other = usePackageCapabilities(otherTags);
	const muted = row.pinState === "stale" || row.pinState === "expired";
	const memory = labels.memory(row.access.memory);
	const timeout = labels.timeout(row.access.timeout);
	const hosts = row.access.hosts;
	const hiddenHosts = hosts.slice(MAX_VISIBLE_HOSTS);

	return (
		<TableRow className={cn("align-top", muted && "text-muted-foreground")}>
			<TableCell className="pl-4 font-medium whitespace-normal">
				<div className="flex min-w-32 flex-col items-start gap-1">
					<span>{row.name}</span>
					{row.pinState !== "active" && (
						<PackageStatusBadge
							pinState={row.pinState}
							timeLeftLabel={row.timeLeftLabel}
							className="text-[10px]"
						/>
					)}
				</div>
			</TableCell>
			<TableCell className="whitespace-normal">
				{groups.network.length === 0 ? (
					<None />
				) : (
					<div className="flex min-w-40 max-w-56 flex-wrap gap-1">
						{groups.network.includes("net.http") &&
							(hosts.length > 0 ? (
								<>
									{hosts.slice(0, MAX_VISIBLE_HOSTS).map((host) => (
										<Badge
											key={host}
											variant="outline"
											className={cn("font-mono", ELEVATED_CHIP_CLASS)}
										>
											{host}
										</Badge>
									))}
									{hiddenHosts.length > 0 && (
										<Badge
											variant="outline"
											className={ELEVATED_CHIP_CLASS}
											title={hiddenHosts.join(", ")}
										>
											{`+${hiddenHosts.length}`}
										</Badge>
									)}
								</>
							) : (
								<Badge variant="outline" className={ELEVATED_CHIP_CLASS}>
									{t("appPackagesAnyHost", "Any host")}
								</Badge>
							))}
						{groups.network
							.filter((tag) => tag !== "net.http")
							.map((tag) => (
								<Badge
									key={tag}
									variant="outline"
									className={ELEVATED_CHIP_CLASS}
								>
									{labels.tag(tag)}
								</Badge>
							))}
					</div>
				)}
			</TableCell>
			<TableCell className="whitespace-normal">
				{groups.files.length === 0 ? (
					<None />
				) : (
					<div className="flex min-w-36 max-w-56 flex-wrap gap-1">
						{groups.files.map((tag) => (
							<Badge
								key={tag}
								variant="outline"
								className={cn(
									"font-normal",
									tag === "storage.user" && ELEVATED_CHIP_CLASS,
								)}
							>
								{labels.tag(tag)}
							</Badge>
						))}
					</div>
				)}
			</TableCell>
			<TableCell className="whitespace-normal">
				{memory || timeout ? (
					<div className="flex flex-col gap-0.5 whitespace-nowrap text-xs">
						<span>{memory ?? "—"}</span>
						<span className="text-muted-foreground">{timeout ?? "—"}</span>
					</div>
				) : (
					<Empty />
				)}
			</TableCell>
			<TableCell className="whitespace-normal">
				{other.length === 0 ? (
					<Empty />
				) : (
					<div className="flex max-w-56 flex-wrap gap-1">
						{other.map((capability) => (
							<Badge
								key={capability.key}
								variant="outline"
								className={cn(
									"font-normal",
									capability.severity === "elevated" && ELEVATED_CHIP_CLASS,
								)}
								title={
									capability.key === "oauth" && row.access.oauthProviders.length
										? row.access.oauthProviders.join(", ")
										: undefined
								}
							>
								{capability.label}
							</Badge>
						))}
					</div>
				)}
			</TableCell>
			<TableCell className="pr-4 whitespace-normal">
				<WidgetAccessCell widgets={row.widgets} />
			</TableCell>
		</TableRow>
	);
}

function WidgetAccessCell({
	widgets,
}: Readonly<{ widgets: readonly PackageWidgetEntry[] }>) {
	const { t } = useTranslation("store");
	const view = useMemo(() => {
		const hosts = new Set<string>();
		const capabilities = new Set<MicroWidgetCapability>();
		let hasInputs = false;
		for (const widget of widgets) {
			const network = describePackageWidgetNetwork(widget);
			for (const host of network.hosts) hosts.add(host);
			hasInputs ||= network.hasInputs;
			for (const capability of policyCapabilities(
				widget.contract?.capabilities,
			)) {
				capabilities.add(capability);
			}
		}
		return { hosts: hosts.size, hasInputs, capabilities: [...capabilities] };
	}, [widgets]);

	if (widgets.length === 0) return <Empty />;
	const quiet =
		view.hosts === 0 && !view.hasInputs && view.capabilities.length === 0;

	return (
		<div className="flex min-w-36 flex-col gap-1">
			<span>
				{t("appPackagesWidgetCount", {
					defaultValue_one: "{{count}} widget",
					defaultValue_other: "{{count}} widgets",
					count: widgets.length,
				})}
			</span>
			{quiet ? (
				<span className="flex items-center gap-1 text-xs text-muted-foreground">
					<ShieldCheck className="size-3" aria-hidden="true" />
					{t("appPackagesWidgetsNoExtraAccess", "No extra access")}
				</span>
			) : (
				<>
					{view.hosts > 0 && (
						<span className="text-xs text-primary">
							{t("appPackagesWidgetsReachSites", {
								defaultValue_one: "Can reach {{count}} site",
								defaultValue_other: "Can reach {{count}} sites",
								count: view.hosts,
							})}
						</span>
					)}
					{view.hasInputs && (
						<span className="text-xs text-primary">
							{t(
								"appPackagesWidgetsRuntimeSites",
								"Plus addresses the flow passes in",
							)}
						</span>
					)}
					<WidgetCapabilityBadges capabilities={view.capabilities} />
				</>
			)}
		</div>
	);
}
