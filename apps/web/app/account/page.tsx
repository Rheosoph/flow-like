"use client";

import { Button, useBackend, useHub } from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import { updatePassword, updateUserAttributes } from "aws-amplify/auth";
import { useRouter } from "next/navigation";
import { useState } from "react";
import { useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { currentRelativeUrl } from "../../lib/return-url";
import { type ProfileActions, ProfilePage } from "./account";
import ChangeEmailDialog from "./change-email";
import ChangePasswordDialog from "./change-password";

export default function AccountPage() {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const hub = useHub();
	const auth = useAuth();
	const router = useRouter();
	const [passwordOpen, setPasswordOpen] = useState(false);
	const [emailOpen, setEmailOpen] = useState(false);
	const poolId = hub.hub?.authentication?.openid?.cognito?.user_pool_id;
	const sub = auth.user?.profile.sub;
	const identities = auth.user?.profile.identities;
	const federated = Array.isArray(identities) && identities.length > 0;
	const ready = Boolean(hub.hub);
	const canManageCredentials = ready && Boolean(poolId) && !federated;

	async function viewBilling() {
		try {
			const popup = window.open("about:blank", "_blank");
			if (!popup) throw new Error("Allow pop-ups to open the billing portal.");
			popup.opener = null;
			try {
				const session = await backend.userState.getBillingSession();
				popup.location.href = session.url;
			} catch (error) {
				popup.close();
				throw error;
			}
		} catch {
			toast.error(
				t(
					"accountBillingFailed",
					"The billing portal could not be opened. Allow pop-ups and try again.",
				),
			);
		}
	}

	const premium = hub.hub?.features?.premium ?? false;
	const actions: ProfileActions = {
		credentialsReady: ready,
		providerManaged: ready && (!poolId || federated),
		updateEmail: canManageCredentials
			? async () => setEmailOpen(true)
			: undefined,
		changePassword: canManageCredentials
			? async () => setPasswordOpen(true)
			: undefined,
		handleAttributeUpdate: canManageCredentials
			? async (attribute, value) => {
					await updateUserAttributes({
						userAttributes: { [attribute]: value },
					});
				}
			: undefined,
		previewProfile: sub
			? async () => router.push(`/profile?sub=${encodeURIComponent(sub)}`)
			: undefined,
		viewBilling: premium ? viewBilling : undefined,
		viewSubscription: premium
			? async () => router.push("/subscription")
			: undefined,
	};

	if (auth.isLoading)
		return (
			<main className="p-8" aria-live="polite">
				{t("accountLoading", "Loading account settings...")}
			</main>
		);
	if (!auth.isAuthenticated)
		return (
			<main className="flex flex-1 items-center justify-center p-8">
				<div className="max-w-sm space-y-4 text-center">
					<h1 className="text-xl font-semibold">
						{t("accountSignInRequired", "Sign in to manage your account")}
					</h1>
					<Button
						onClick={() =>
							auth.signinRedirect({ url_state: currentRelativeUrl() })
						}
					>
						{t("logIn", "Log in")}
					</Button>
				</div>
			</main>
		);

	return (
		<>
			<ProfilePage key={sub} actions={actions} />
			{canManageCredentials && (
				<>
					<ChangePasswordDialog
						key={`${sub}:password`}
						open={passwordOpen}
						onOpenChange={setPasswordOpen}
						onPasswordChange={(currentPassword, newPassword) =>
							updatePassword({ oldPassword: currentPassword, newPassword })
						}
					/>
					<ChangeEmailDialog
						key={`${sub}:email`}
						open={emailOpen}
						onOpenChange={setEmailOpen}
					/>
				</>
			)}
		</>
	);
}
