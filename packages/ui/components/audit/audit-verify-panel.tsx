"use client";

import { useTranslation } from "@flow-like/locales";
import { Loader2, ShieldCheck } from "lucide-react";
import { useId, useState } from "react";
import { apiErrorMessage } from "../../lib/api-error";
import type { IProfile } from "../../lib/schema/profile/profile";
import { Button } from "../ui/button";
import {
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "../ui/card";
import { Label } from "../ui/label";
import { RelativeTime } from "../ui/relative-time";
import { Switch } from "../ui/switch";
import { AuditChainReportView } from "./audit-report";
import { useAuditVerify } from "./use-audit";

export interface AuditVerifyPanelProps {
	profile: IProfile | undefined;
	chainId: string;
	/** Platform admins, and owners on their app's chains, may ignore the cached progress and re-check every seal. */
	allowFull?: boolean;
}

export function AuditVerifyPanel({
	profile,
	chainId,
	allowFull = false,
}: Readonly<AuditVerifyPanelProps>) {
	const { t } = useTranslation("audit");
	const fullSwitchId = useId();
	const [full, setFull] = useState(false);
	const verification = useAuditVerify(profile, chainId, allowFull && full);

	return (
		<Card>
			<CardHeader className="pb-3">
				<CardTitle className="flex items-center gap-2 text-base">
					<ShieldCheck className="h-4 w-4 text-primary" />
					{t("integrity", "Integrity")}
				</CardTitle>
				<CardDescription>
					{t(
						"integrityDescription",
						"Recomputes every record hash, seal link and epoch signature of this chain.",
					)}
				</CardDescription>
			</CardHeader>
			<CardContent className="space-y-3">
				<div className="flex flex-wrap items-center gap-3">
					<Button
						size="sm"
						variant="outline"
						disabled={!profile || verification.isFetching}
						onClick={() => verification.refetch()}
					>
						{verification.isFetching ? (
							<Loader2 className="mr-2 h-3.5 w-3.5 animate-spin" />
						) : (
							<ShieldCheck className="mr-2 h-3.5 w-3.5" />
						)}
						{verification.isFetching
							? t("verifying", "Verifying…")
							: t("verifyChain", "Verify chain")}
					</Button>
					{allowFull && (
						<div className="flex items-center gap-2">
							<Switch
								id={fullSwitchId}
								checked={full}
								onCheckedChange={setFull}
							/>
							<Label
								htmlFor={fullSwitchId}
								className="text-xs text-muted-foreground"
							>
								{t("fullCheck", "Re-check from the first seal")}
							</Label>
						</div>
					)}
				</div>
				{verification.isError ? (
					<p role="alert" className="text-sm text-destructive">
						{apiErrorMessage(
							verification.error,
							t("verifyFailed", "Could not verify this chain. Try again."),
						)}
					</p>
				) : verification.data ? (
					<div aria-live="polite" className="space-y-2">
						<AuditChainReportView report={verification.data} />
						<p className="text-xs text-muted-foreground">
							{t("checked", "Checked")}{" "}
							<RelativeTime value={verification.dataUpdatedAt} />
						</p>
					</div>
				) : null}
			</CardContent>
		</Card>
	);
}
