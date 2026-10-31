"use client";

import { OfflineAccessPage } from "@flow-like/flow-like-ui/components/settings/offline-access";
import { Button } from "@flow-like/flow-like-ui/components/ui/button";
import { useTranslation } from "@flow-like/locales";
import { useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { useSearchParams } from "next/navigation";
import { useState, type ReactElement } from "react";
import { toast } from "sonner";
import { nativeSnapshotOwner } from "../../../../lib/native-snapshot-owner";
import NotFound from "../not-found";

export default function Page(): ReactElement {
	const searchParams = useSearchParams();
	const id = searchParams?.get("id") ?? null;
	const queryClient = useQueryClient();
	const { t } = useTranslation("settings");
	const [recovering, setRecovering] = useState(false);
	const recover = async () => {
		const owner = nativeSnapshotOwner();
		if (!owner) return;
		setRecovering(true);
		try {
			const result = await invoke<{
				recoveredScopes: number;
				recoveredReports: number;
				conflicts: string[];
				cancelled: boolean;
			}>("native_auth_recover_background_work", { profileId: owner.profileId });
			if (result.cancelled) return;
			await queryClient.invalidateQueries({ queryKey: ["offline-writes"] });
			if (result.conflicts.length) {
				toast.warning(
					t(
						"offlineAccess.recoveryConflict",
						"Some older work needs review because this profile already has offline data for the same app.",
					),
					{ description: result.conflicts.join("\n") },
				);
			} else {
				toast.success(
					t("offlineAccess.recoveryComplete", {
						defaultValue:
							"Recovered {{scopes}} offline caches and {{reports}} run reports.",
						scopes: result.recoveredScopes,
						reports: result.recoveredReports,
					}),
				);
			}
		} catch (error) {
			toast.error(error instanceof Error ? error.message : String(error));
		} finally {
			setRecovering(false);
		}
	};

	if (!id) return <NotFound />;

	return (
		<OfflineAccessPage
			appId={id}
			recoveryAction={
				<Button
					className="self-start"
					variant="outline"
					disabled={recovering}
					onClick={() => void recover()}
				>
					{t("offlineAccess.recoverOlderWork", "Recover older pending work")}
				</Button>
			}
		/>
	);
}
