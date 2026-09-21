"use client";

import { useTranslation } from "@flow-like/locales";
import { useSearchParams } from "next/navigation";
import { useState } from "react";
import { useAppPermissions } from "../../../hooks/use-app-permissions";
import { useInvoke } from "../../../hooks/use-invoke";
import { RolePermissions } from "../../../lib/permission/role-permission";
import type { IProfile } from "../../../lib/schema/profile/profile";
import { useBackend } from "../../../state/backend-state";
import { AuditHeadCard } from "../../audit/audit-head-card";
import { AuditRecordList } from "../../audit/audit-record-list";
import { AuditVerifyPanel } from "../../audit/audit-verify-panel";
import { activityChainOf } from "../../audit/types";
import { useAuditRecords } from "../../audit/use-audit";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "../../ui/tabs";
import { SectionLockedPanel } from "../permission/permission-gate";
import { AuditExportCard } from "./audit-export-card";

function ChainView({
	profile,
	chainId,
	emptyLabel,
}: Readonly<{
	profile: IProfile | undefined;
	chainId: string;
	emptyLabel: string;
}>) {
	return (
		<div className="grid gap-4 xl:grid-cols-[minmax(0,1fr)_360px]">
			<div className="min-w-0">
				<AuditRecordList
					profile={profile}
					chainId={chainId}
					emptyLabel={emptyLabel}
				/>
			</div>
			<div className="min-w-0 space-y-4">
				<AuditVerifyPanel profile={profile} chainId={chainId} allowFull />
				<AuditHeadCard profile={profile} chainId={chainId} />
			</div>
		</div>
	);
}

export function AuditTrailPage() {
	const { t } = useTranslation("audit");
	const searchParams = useSearchParams();
	const appId = searchParams.get("id") ?? "";
	const backend = useBackend();
	const permissions = useAppPermissions(appId);
	/** Every `/audit/*` read of an app chain and every export route is Owner-only. */
	const canRead = permissions.can(RolePermissions.Owner);
	const settingsProfile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);
	const profile = canRead ? settingsProfile.data?.hub_profile : undefined;
	const activityChain = activityChainOf(appId);
	const activityProbe = useAuditRecords(profile, appId ? activityChain : "");
	const hasActivity = (activityProbe.data?.pages[0]?.records.length ?? 0) > 0;
	const [tab, setTab] = useState("records");

	if (!appId) return null;

	return (
		<div className="mx-auto flex w-full max-w-6xl flex-col gap-5 p-4 pb-8">
			<div>
				<h1 className="text-2xl font-bold tracking-tight">
					{t("auditTrail", "Audit trail")}
				</h1>
				<p className="max-w-prose text-sm text-muted-foreground">
					{t(
						"appPageDescription",
						"Every administrative change to this app, sealed into a tamper-evident chain. Records appear at once and are sealed within minutes.",
					)}
				</p>
			</div>

			{canRead ? (
				<Tabs value={tab} onValueChange={setTab}>
					<TabsList>
						<TabsTrigger value="records">{t("records", "Records")}</TabsTrigger>
						{hasActivity && (
							<TabsTrigger value="activity">
								{t("classActivity", "Activity")}
							</TabsTrigger>
						)}
						<TabsTrigger value="export">
							{t("exportTitle", "Export")}
						</TabsTrigger>
					</TabsList>
					<TabsContent value="records" className="mt-4">
						<ChainView
							profile={profile}
							chainId={appId}
							emptyLabel={t(
								"noAppRecords",
								"Nothing has been recorded for this app yet.",
							)}
						/>
					</TabsContent>
					{hasActivity && (
						<TabsContent value="activity" className="mt-4">
							<ChainView
								profile={profile}
								chainId={activityChain}
								emptyLabel={t(
									"noActivityRecords",
									"No activity records. Activity is kept for a shorter window than evidence.",
								)}
							/>
						</TabsContent>
					)}
					<TabsContent value="export" className="mt-4">
						<AuditExportCard profile={profile} appId={appId} />
					</TabsContent>
				</Tabs>
			) : (
				<SectionLockedPanel
					feature={t("auditTrail", "Audit trail")}
					description={t(
						"lockedDescription",
						"Only owners of this app can read its audit trail.",
					)}
					missing={[RolePermissions.Owner]}
					roleName={permissions.roleName}
				/>
			)}
		</div>
	);
}
