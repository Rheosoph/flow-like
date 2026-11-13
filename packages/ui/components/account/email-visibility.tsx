"use client";

import { useTranslation } from "@flow-like/locales";
import { useQueryClient } from "@tanstack/react-query";
import { Eye } from "lucide-react";
import { useState } from "react";
import { toast } from "sonner";
import { useHub } from "../../hooks/use-hub";
import { useBackend } from "../../state/backend-state";
import type { IUserInfo } from "../../state/backend-state/user-state";
import { Label } from "../ui/label";
import { Switch } from "../ui/switch";
import { accountError, invalidateAccountIdentity } from "./account-model";

/**
 * Whether other people see the account's email address. It is hidden until the
 * person turns this on. A hub that shows every member's address says so here
 * instead of offering a choice it would not honor.
 */
export function EmailVisibility({
	info,
	disabled = false,
}: Readonly<{ info: IUserInfo; disabled?: boolean }>) {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const client = useQueryClient();
	const { hub } = useHub();
	const [saving, setSaving] = useState(false);
	const hubWide = hub?.lookup?.email === true;

	async function toggle(share_email: boolean) {
		if (saving) return;
		setSaving(true);
		try {
			await backend.userState.updateUser({ share_email });
			client.setQueryData<IUserInfo>(
				[backend.userState.getInfo.name],
				(current) => (current ? { ...current, share_email } : current),
			);
			await invalidateAccountIdentity(client);
			toast.success(
				share_email
					? t(
							"accountEmailNowShared",
							"Other people can now see your email address.",
						)
					: t(
							"accountEmailNowHidden",
							"Your email address is hidden from other people.",
						),
			);
		} catch (cause) {
			toast.error(
				accountError(
					cause,
					t(
						"accountEmailVisibilityFailed",
						"Your email visibility could not be changed. Try again.",
					),
				),
			);
		} finally {
			setSaving(false);
		}
	}

	return (
		<div className="flex items-center justify-between gap-4 py-4">
			<div className="min-w-0 space-y-1">
				<Label
					htmlFor="account-share-email"
					className="flex items-center gap-2 text-sm font-medium"
				>
					<Eye className="size-4 text-muted-foreground" />
					{t("accountShareEmail", "Show my email to other people")}
				</Label>
				<p
					id="account-share-email-help"
					className="text-xs leading-5 text-muted-foreground"
				>
					{hubWide
						? t(
								"accountEmailSharedByHub",
								"This hub shows every member's email address to other members.",
							)
						: t(
								"accountShareEmailHelp",
								"Off by default. When on, other people see your email address on your profile and can find you by it.",
							)}
				</p>
			</div>
			<Switch
				id="account-share-email"
				aria-describedby="account-share-email-help"
				checked={hubWide || info.share_email === true}
				disabled={disabled || saving || hubWide}
				onCheckedChange={(checked) => void toggle(checked)}
			/>
		</div>
	);
}
