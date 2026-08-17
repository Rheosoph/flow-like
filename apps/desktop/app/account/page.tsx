"use client";
import { useTranslation } from "@flow-like/locales";
import { Button, useBackend, useHub, useInvoke } from "@flow-like/flow-like-ui";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Amplify } from "aws-amplify";
import {
	type AuthTokens,
	type TokenProvider,
	type UpdateUserAttributesInput,
	decodeJWT,
	fetchAuthSession,
	fetchMFAPreference,
	fetchUserAttributes,
	getCurrentUser,
	updatePassword,
	updateUserAttributes,
} from "aws-amplify/auth";
import { useRouter } from "next/navigation";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { type AuthContextProps, useAuth } from "react-oidc-context";
import { toast } from "sonner";
import { type ProfileActions, ProfilePage } from "./account";
import ChangeEmailDialog from "./change-email";
import ChangePasswordDialog from "./change-password";

class AuthTokenProvider implements TokenProvider {
	constructor(private readonly authContext: AuthContextProps) {}

	async getTokens(options?: {
		forceRefresh?: boolean;
	}): Promise<AuthTokens | null> {
		if (!this.authContext.isAuthenticated || !this.authContext.user) {
			return null;
		}

		if (
			!this.authContext.user.access_token ||
			!this.authContext.user.id_token
		) {
			return null;
		}

		const accessToken = decodeJWT(this.authContext.user?.access_token || "");
		const idToken = decodeJWT(this.authContext.user?.id_token || "");

		return {
			accessToken: accessToken,
			idToken: idToken,
		};
	}
}

const AccountPage: React.FC = () => {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const hub = useHub();
	const auth = useAuth();
	const router = useRouter();
	const [passwordDialogOpen, setPasswordDialogOpen] = useState(false);
	const [emailDialogOpen, setEmailDialogOpen] = useState(false);
	const [cognito, setCognito] = useState(false);
	const [federated, setFederated] = useState(false);
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
	);

	const isPremiumEnabled = hub.hub?.features?.premium ?? false;

	const backendRef = useRef(backend);
	backendRef.current = backend;
	const authRef = useRef(auth);
	authRef.current = auth;
	const hubRef = useRef(hub);
	hubRef.current = hub;

	const updateUserAttribute = useCallback(
		async (attributeKey: string, value: string) => {
			if (!cognito) {
				console.warn(
					"Cognito is not enabled, skipping attribute update ",
					cognito,
				);
				return;
			}
			try {
				const updateInput: UpdateUserAttributesInput = {
					userAttributes: {
						[attributeKey]: value,
					},
				};

				await updateUserAttributes(updateInput);
			} catch (error) {
				console.error(`Failed to update ${attributeKey}:`, error);
				throw error;
			}
		},
		[cognito],
	);

	const handleChangePassword = useCallback(async () => {
		setPasswordDialogOpen(true);
	}, []);

	const handleUpdateEmail = useCallback(async () => {
		setEmailDialogOpen(true);
	}, []);

	const configureAmplify = useCallback(async () => {
		const currentAuth = authRef.current;
		const currentHub = hubRef.current;

		if (!currentAuth.isAuthenticated || !currentAuth.user?.profile) return;
		if (currentHub.hub?.authentication?.openid?.cognito?.user_pool_id) {
			const provider = new AuthTokenProvider(currentAuth);
			Amplify.configure(
				{
					Auth: {
						Cognito: {
							userPoolClientId: currentAuth.settings.client_id,
							userPoolId:
								currentHub.hub.authentication.openid.cognito.user_pool_id,
						},
					},
				},
				{
					Auth: {
						tokenProvider: provider,
					},
				},
			);

			const currentUser = await getCurrentUser();
			const attributes = await fetchUserAttributes();
			const authSession = await fetchAuthSession();
			const mfaPreferences = await fetchMFAPreference();

			console.dir({
				currentUser,
				attributes,
				authSession,
				mfaPreferences,
			});

			const isFederated = Array.isArray(
				authSession.tokens?.idToken?.payload?.identities,
			);
			setFederated(isFederated);
			setCognito(true);
		}
	}, []);

	useEffect(() => {
		if (auth.isAuthenticated && hub.hub) {
			configureAmplify();
		}
	}, [auth.isAuthenticated, hub.hub, configureAmplify]);

	const handlePasswordChange = useCallback(
		async (currentPassword: string, newPassword: string) => {
			try {
				await updatePassword({
					oldPassword: currentPassword,
					newPassword: newPassword,
				});

				setPasswordDialogOpen(false);
				toast.success("Password updated successfully");
			} catch (error) {
				console.error("Failed to update password:", error);
				toast.error("Failed to update password");
				throw error;
			}
		},
		[],
	);

	const handleViewBilling = useCallback(async () => {
		try {
			const billingSession =
				await backendRef.current.userState.getBillingSession();

			await openUrl(billingSession.url);
		} catch (error) {
			console.error("Failed to get billing session:", error);
			toast.error("Failed to open billing portal");
		}
	}, []);

	const handlePreviewProfile = useCallback(async () => {
		router.push(`/profile?sub=${authRef.current.user?.profile?.sub}`);
	}, [router]);

	const handleViewSubscription = useCallback(async () => {
		router.push("/subscription");
	}, [router]);

	const profileActions = useMemo<ProfileActions>(
		() => ({
			updateEmail: cognito && !federated ? handleUpdateEmail : undefined,
			changePassword: cognito && !federated ? handleChangePassword : undefined,
			viewBilling: isPremiumEnabled ? handleViewBilling : undefined,
			viewSubscription: isPremiumEnabled ? handleViewSubscription : undefined,
			previewProfile: handlePreviewProfile,
			handleAttributeUpdate: cognito ? updateUserAttribute : undefined,
		}),
		[
			cognito,
			federated,
			isPremiumEnabled,
			handleUpdateEmail,
			handleChangePassword,
			handleViewBilling,
			handleViewSubscription,
			handlePreviewProfile,
			updateUserAttribute,
		],
	);

	if (!auth.isAuthenticated) {
		return (
			<main className="flex flex-row items-center justify-center w-full flex-1 min-h-0 py-12">
				<div className="text-center p-6 border rounded-lg shadow-lg bg-card">
					<h3>{t('pleaseLogInToViewYourProfile', 'Please log in to view your profile.')}</h3>
					<Button onClick={() => auth.signinRedirect()} className="mt-4">
						{t('logIn', 'Log In')}
					</Button>
				</div>
			</main>
		);
	}

	return (
		<>
			<ProfilePage actions={profileActions} />

			{!federated && (
				<ChangePasswordDialog
					key={auth.user?.profile?.sub + "password"}
					open={passwordDialogOpen}
					onOpenChange={setPasswordDialogOpen}
					onPasswordChange={handlePasswordChange}
				/>
			)}

			{!federated && (
				<ChangeEmailDialog
					open={emailDialogOpen}
					onOpenChange={setEmailDialogOpen}
				/>
			)}
		</>
	);
};

export default AccountPage;
