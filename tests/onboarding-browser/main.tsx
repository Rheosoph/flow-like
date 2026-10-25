import { createI18n } from "@flow-like/locales";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useState } from "react";
import { createRoot } from "react-dom/client";
import { AuthContext, type AuthContextProps } from "react-oidc-context";
import { Toaster } from "sonner";
import OnboardingLayout from "../../apps/desktop/app/onboarding/layout";
import Onboarding from "../../apps/desktop/app/onboarding/page";
import { TooltipProvider } from "../../packages/ui/components/ui/tooltip";
import {
	type IBackendState,
	useBackendStore,
} from "../../packages/ui/state/backend-state";
import { qa } from "./tauri";
import "./style.css";

createI18n({ language: "en" });
const queryClient = new QueryClient({
	defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } },
});
useBackendStore.getState().setBackend({
	capabilities: () => ({
		needsSignIn: false,
		canExecuteLocally: true,
		canHostEmbeddings: true,
		canHostLlamaCPP: true,
		canHostMLX: true,
	}),
	bitState: {
		getBitSize: async (bit: { id: string; size?: number }) => {
			if (qa.holdSizes)
				return new Promise<number>((resolve) => {
					qa.sizeRequests.push({ bitId: bit.id, resolve });
				});
			return bit.size ?? 0;
		},
	},
	userState: { getProfile: async () => undefined },
} as unknown as IBackendState);

const auth = {
	settings: {
		authority: "https://fixture.invalid",
		client_id: "fixture-client",
	},
	isAuthenticated: new URLSearchParams(location.search).has("authenticated"),
	isLoading: false,
	user: new URLSearchParams(location.search).has("authenticated")
		? { access_token: "fixture-token" }
		: undefined,
	signinRedirect: async () => {
		qa.signIns++;
		if (qa.failSignIn) throw new Error("Fixture sign-in failed");
	},
};

function Harness() {
	const [active, setActive] = useState(true);
	qa.unmount = () => setActive(false);
	return active ? <Onboarding /> : <p>Onboarding unmounted</p>;
}

const root = document.getElementById("root");
if (!root) throw new Error("Preview root is missing");
function App() {
	const [authState, setAuthState] = useState(auth);
	qa.setSignedIn = (signedIn) =>
		setAuthState({
			...auth,
			isAuthenticated: signedIn,
			user: signedIn ? { access_token: "fixture-token" } : undefined,
		});
	return (
		<AuthContext.Provider value={authState as unknown as AuthContextProps}>
			<QueryClientProvider client={queryClient}>
				<TooltipProvider>
					<OnboardingLayout>
						<Harness />
					</OnboardingLayout>
					<Toaster />
				</TooltipProvider>
			</QueryClientProvider>
		</AuthContext.Provider>
	);
}
createRoot(root).render(<App />);
