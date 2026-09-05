import {
	QueryClient,
	QueryClientProvider,
	useQueryClient,
} from "@tanstack/react-query";
import { useState } from "react";
import { createRoot } from "react-dom/client";
import { AuthContext, type AuthContextProps } from "react-oidc-context";
import { Toaster } from "sonner";
import { ProfilePage } from "../../packages/ui/components/account/account-settings";
import ChangeEmailDialog from "../../packages/ui/components/account/change-email-dialog";
import ChangePasswordDialog from "../../packages/ui/components/account/change-password-dialog";
import { PublicProfilePage } from "../../packages/ui/components/profile/public-profile-page";
import { TooltipProvider } from "../../packages/ui/components/ui/tooltip";
import {
	useBackend,
	useBackendStore,
} from "../../packages/ui/state/backend-state";
import "../../packages/ui/global.css";
import { WorkspaceFixture } from "./workspace-fixture";

const params = new URLSearchParams(location.search);
if (params.has("light")) document.documentElement.classList.remove("dark");
const at = { secs_since_epoch: 1788566400, nanos_since_epoch: 0 };
const user = {
	id: "review-user",
	name: "Alex Morgan",
	preferred_username: "alexm",
	email: "alex@example.invalid",
	description:
		"I build tools that make everyday work easier. Exploring practical ways to connect data, documents, and people.",
	created_at: "2025-03-15",
	tier: "FREE",
};
const apps = ["Knowledge Chat", "Invoice Review", "Team Briefing"].map(
	(name, i) => [
		{
			id: `review-app-${i}`,
			authors: [user.id],
			bits: [],
			boards: [],
			events: [],
			page_ids: [],
			widget_ids: [],
			templates: [],
			status: "Active",
			visibility: "Public",
			execution_mode: "Remote",
			primary_category: "Productivity",
			created_at: at,
			updated_at: at,
			download_count: 24 + i,
			rating_count: 0,
			rating_sum: 0,
		},
		{
			name,
			description: [
				"Find answers across your team's documents.",
				"Extract and review incoming invoices.",
				"Keep everyone up to date on the work that matters.",
			][i],
			tags: ["productivity"],
			created_at: at,
			updated_at: at,
			preview_media: [],
		},
	],
);

function Fixture() {
	const original = useBackend();
	const client = useQueryClient();
	const [passwordOpen, setPasswordOpen] = useState(false);
	const [emailOpen, setEmailOpen] = useState(false);
	const [state] = useState(() => {
		const state = {
			user: { ...user },
			profileWrites: [] as unknown[],
			passwordWrites: 0,
			emailWrites: 0,
			failSave: false,
			failEmail: false,
			failPassword: false,
			holdSave: false,
			releaseSave: undefined as (() => void) | undefined,
			failApps: params.has("apps-error"),
			failLookup: params.has("profile-error"),
		};
		Object.assign(window, {
			profileQa: {
				state,
				refresh: () => client.invalidateQueries(),
			},
		});
		useBackendStore.getState().setBackend({
			...original,
			userState: {
				...original.userState,
				getInfo: async function getInfo() {
					return { ...state.user };
				},
				lookupUser: async function lookupUser() {
					if (state.failLookup) throw new Error("Fixture profile failure");
					return { ...state.user };
				},
				updateUser: async function updateUser(data: object, avatar?: File) {
					if (state.holdSave)
						await new Promise<void>((resolve) => {
							state.releaseSave = resolve;
						});
					if (state.failSave) throw new Error("Fixture save failure");
					state.profileWrites.push(data);
					Object.assign(state.user, data);
					if (avatar)
						Object.assign(state.user, { avatar: URL.createObjectURL(avatar) });
				},
			},
			appState: {
				...original.appState,
				searchApps: async function searchApps() {
					if (state.failApps) throw new Error("Fixture app failure");
					return params.has("empty") ? [] : apps;
				},
			},
		} as unknown as typeof original);
		return state;
	});

	return (
		<div className="flex min-h-screen flex-col bg-background text-foreground">
			<div className="border-b px-4 py-2 text-xs text-muted-foreground">
				Local component verification · fixture data
			</div>
			{params.get("view") === "workspace" ? (
				<WorkspaceFixture />
			) : params.get("view") === "account" ? (
				<ProfilePage
					actions={{
						credentialsReady: true,
						providerManaged: params.has("managed"),
						handleAttributeUpdate: params.has("managed")
							? undefined
							: async (_, value) => {
									state.user.preferred_username = value;
								},
						updateEmail: params.has("managed")
							? undefined
							: async () => setEmailOpen(true),
						changePassword: params.has("managed")
							? undefined
							: async () => setPasswordOpen(true),
						previewProfile: async () => {
							location.href = "/?sub=review-user";
						},
						viewSubscription: async () => {},
					}}
				/>
			) : (
				<PublicProfilePage />
			)}
			<ChangePasswordDialog
				open={passwordOpen}
				onOpenChange={setPasswordOpen}
				onPasswordChange={async () => {
					state.passwordWrites++;
					if (state.failPassword) {
						const error = new Error("Policy rejected");
						error.name = "InvalidPasswordException";
						throw error;
					}
				}}
			/>
			<ChangeEmailDialog
				open={emailOpen}
				onOpenChange={setEmailOpen}
				updateEmail={async () => {
					state.emailWrites++;
					if (state.failEmail) {
						const error = new Error("Network failure");
						error.name = "NetworkError";
						throw error;
					}
					return { needsVerification: true };
				}}
				verifyEmail={async () => {
					state.user.email = "updated@example.invalid";
				}}
				resendCode={async () => {
					state.emailWrites++;
				}}
			/>
			<Toaster />
		</div>
	);
}

const client = new QueryClient({
	defaultOptions: {
		queries: { retry: false, staleTime: 30000, refetchOnWindowFocus: false },
	},
});
const container = document.getElementById("root");
if (!container) throw new Error("Fixture root missing");
createRoot(container).render(
	<QueryClientProvider client={client}>
		<AuthContext.Provider
			value={
				{
					isAuthenticated: !params.has("visitor"),
					user: { profile: { sub: "review-user" } },
				} as AuthContextProps
			}
		>
			<TooltipProvider>
				<Fixture />
			</TooltipProvider>
		</AuthContext.Provider>
	</QueryClientProvider>,
);
