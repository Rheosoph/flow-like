import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import { I18nProvider } from "@flow-like/locales";
import { Toaster } from "sonner";
import AdminUsersPage from "../../apps/web/app/admin/users/page";
import {
	type IBackendState,
	useBackend,
	useBackendStore,
} from "../../packages/ui/state/backend-state";
import "../../packages/ui/global.css";

const params = new URLSearchParams(location.search);
const light = params.has("light");
document.documentElement.classList.toggle("dark", !light);
const profile = {
	id: "admin-fixture",
	hub: "https://fixture.invalid",
	name: "Platform team",
};
const seed = [
	["Maya Chen", "maya", "MAX", 83_520_000_000, 18_400_000],
	["Alex Morgan", "alex", "PRO", 22_400_000_000, 4_150_000],
	["Sam Rivera", "sam", "PREMIUM", 4_920_000_000, 1_240_000],
	["Jordan Lee", "jordan", "FREE", 480_000_000, 340_000],
	["Robin Patel", "robin", "MAX", 136_800_000_000, 27_600_000],
];
const users = seed.map(
	([name, id, tier, total_size, total_llm_price], index) => ({
		id: `fixture-${id}`,
		name,
		email: `${id}@example.com`,
		tier,
		total_size,
		total_llm_price,
		status: "ACTIVE",
		permission: 0,
		total_embedding_price: 0,
		created_at: new Date(
			Date.now() - (index + 2) * 86400000 * 14,
		).toISOString(),
		updated_at: new Date().toISOString(),
	}),
);
const calls: string[] = [];
Object.assign(window, { pricingAdminQa: { users, calls } });
function Fixture() {
	const current = useBackend();
	const [backend] = useState(current);
	const [ready, setReady] = useState(false);
	useEffect(() => {
		useBackendStore.getState().setBackend({
			...backend,
			userState: {
				...backend.userState,
				getProfile: async function getProfile() {
					return profile;
				},
			},
			apiState: {
				...backend.apiState,
				get: async (_: unknown, path: string) => {
					calls.push(path);
					const url = new URL(path, "https://fixture.invalid/");
					if (url.pathname !== "/admin/users")
						throw new Error(`Unexpected fixture request: ${path}`);
					const tier = url.searchParams.get("tier");
					const query = url.searchParams.get("query")?.toLowerCase();
					const rows = users.filter(
						(user) =>
							(!tier || user.tier === tier) &&
							(!query ||
								`${user.name} ${user.email}`.toLowerCase().includes(query)),
					);
					return { users: rows, total: rows.length, offset: 0, limit: 25 };
				},
				patch: async () => {
					throw new Error("Read-only screenshot fixture");
				},
			},
		} as unknown as IBackendState);
		setReady(true);
	}, [backend]);
	return (
		<div className="flex h-dvh flex-col bg-background text-foreground">
			<div className="shrink-0 border-b px-4 py-2 text-xs text-muted-foreground">
				Local preview · synthetic example data · admin plan controls
			</div>
			{ready && <AdminUsersPage />}
			<Toaster theme={light ? "light" : "dark"} />
		</div>
	);
}
const client = new QueryClient({
	defaultOptions: { queries: { retry: false, refetchOnWindowFocus: false } },
});
createRoot(document.getElementById("root")!).render(
	<QueryClientProvider client={client}>
		<I18nProvider language="en">
			<Fixture />
		</I18nProvider>
	</QueryClientProvider>,
);
