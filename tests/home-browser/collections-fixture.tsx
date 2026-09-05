import { useQueryClient } from "@tanstack/react-query";
import { useRef, useState } from "react";
import { HomeWidgetContent } from "../../packages/ui/components/home/home-widget-content";
import { HomeWidgetSettings } from "../../packages/ui/components/home/home-widget-settings";
import type { IHomeWidget } from "../../packages/ui/components/home/types";
import {
	useBackend,
	useBackendStore,
} from "../../packages/ui/state/backend-state";

const at = { secs_since_epoch: 1788566400, nanos_since_epoch: 0 };
const artwork = (color: string) =>
	`data:image/svg+xml,${encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" width="800" height="480"><rect width="800" height="480" fill="${color}"/><circle cx="600" cy="110" r="240" fill="white" fill-opacity=".14"/><circle cx="180" cy="420" r="180" fill="white" fill-opacity=".09"/></svg>`)}`;
const apps = [
	"Fixture Knowledge Chat",
	"Fixture Invoice OCR",
	"Fixture Hidden App",
	"Fixture New App",
].map((name, index) => [
	{
		id: `collection-app-${index}`,
		authors: ["Fixture team"],
		bits: [],
		boards: [],
		events: [],
		page_ids: [],
		widget_ids: [],
		templates: [],
		status: "Active",
		app_type: "App",
		visibility: "Public",
		execution_mode: "Remote",
		primary_category: "Productivity",
		created_at: at,
		updated_at: at,
		download_count: 42,
		rating_count: 3,
		rating_sum: 15,
		avg_rating: 5,
	},
	{
		name,
		description:
			"Local test content for the actual library cards, with realistic text and artwork.",
		use_case: "Find answers and finish a task with your team.",
		tags: ["documents", "productivity"],
		created_at: at,
		updated_at: at,
		preview_media: [],
		icon: artwork(["#4d3988", "#236e62", "#5d5468", "#9d4326"][index]),
		thumbnail: artwork(["#4d3988", "#236e62", "#5d5468", "#9d4326"][index]),
	},
]);
const models = ["Fixture reasoning model", "Fixture embedding model"].map(
	(name, index) => ({
		id: `collection-model-${index}`,
		hash: `fixture-model-hash-${index}`,
		hub: "fixture-hub",
		authors: [],
		dependencies: [],
		dependency_tree_hash: "fixture",
		created: "2026-09-05",
		updated: "2026-09-05",
		type: index ? "Embedding" : "Llm",
		parameters: {
			context_length: 128000,
			provider: { provider_name: "openai", params: {} },
		},
		meta: {
			en: {
				name,
				description:
					"A hosted fixture model with native details and working profile controls.",
				icon: artwork(index ? "#236e62" : "#4d3988"),
				tags: [],
				preview_media: [],
				created_at: at,
				updated_at: at,
			},
		},
		size: 0,
	}),
);
function widget(
	id: string,
	type: string,
	config: Record<string, unknown>,
): IHomeWidget {
	return {
		id,
		type,
		title: id,
		size: { columns: 6, rows: 4 },
		appearance: { variant: "card", accent: "neutral" },
		config,
	};
}
export default function CollectionsFixture() {
	const original = useBackend();
	const queryClient = useQueryClient();
	const scenario = useRef("populated");
	const profiles = useRef({
		a: {
			id: "collection-profile-a",
			name: "Fixture A",
			hub: location.origin,
			secure: false,
			bits: [models[0].id],
			apps: [{ app_id: "collection-app-0", favorite: true, pinned: false }],
		},
		b: {
			id: "collection-profile-b",
			name: "Fixture B",
			hub: location.origin,
			secure: false,
			bits: [models[1].id],
			apps: [{ app_id: "collection-app-1", favorite: false, pinned: false }],
		},
	});
	const writes = useRef<{ action: string; profile: string; bit: string }[]>([]);
	const [profileId, setProfileId] = useState<"a" | "b">("a");
	const [rendering, setRendering] = useState("standard");
	const [surface, setSurface] = useState("card");
	const [modelRendering, setModelRendering] = useState("standard");
	const installBackend = (id: "a" | "b") => {
		const current = () => profiles.current[id];
		const available = <T,>(data: T[], source: string) => {
			if (scenario.current === "error")
				throw new Error(`Fixture ${source} is offline`);
			return scenario.current === "empty" ? [] : data;
		};
		const backend = {
			...original,
			profile: current(),
			capabilities: () => ({
				needsSignIn: false,
				canExecuteLocally: false,
				canHostLlamaCPP: false,
				canHostMLX: false,
				canHostEmbeddings: false,
			}),
			appState: {
				...original.appState,
				async getApps() {
					return available(apps.slice(0, 3), "apps");
				},
				async searchApps() {
					return available(apps, "apps");
				},
				async getApp(id: string) {
					return apps.find(([app]) => app.id === id)?.[0];
				},
				async getAppMeta(id: string) {
					return apps.find(([app]) => app.id === id)?.[1];
				},
			},
			userState: {
				...original.userState,
				async getProfile() {
					return structuredClone(current());
				},
				async getSettingsProfile() {
					return { hub_profile: structuredClone(current()) };
				},
				async getInfo() {
					return { tier: "FREE" };
				},
			},
			bitState: {
				...original.bitState,
				async getProfileBits() {
					return available(
						models.filter((model) => current().bits.includes(model.id)),
						"models",
					);
				},
				async searchBits() {
					return available(models, "models");
				},
				async getBit(id: string) {
					return models.find((model) => model.id === id);
				},
				async isBitInstalled() {
					return true;
				},
				async getBitSize() {
					return 0;
				},
				async addBit(
					bit: { id: string },
					profile: { hub_profile: { id: string } },
				) {
					const target = Object.values(profiles.current).find(
						(p) => p.id === profile.hub_profile.id,
					);
					if (!target) throw new Error("Fixture profile does not exist");
					target.bits = [...new Set([...target.bits, bit.id])];
					writes.current.push({
						action: "add",
						profile: target.id,
						bit: bit.id,
					});
				},
				async removeBit(
					bit: { id: string },
					profile: { hub_profile: { id: string } },
				) {
					const target = Object.values(profiles.current).find(
						(p) => p.id === profile.hub_profile.id,
					);
					if (!target) throw new Error("Fixture profile does not exist");
					target.bits = target.bits.filter((id) => id !== bit.id);
					writes.current.push({
						action: "remove",
						profile: target.id,
						bit: bit.id,
					});
				},
			},
		};
		useBackendStore.getState().setBackend(backend as unknown as IBackendState);
	};
	const [ready] = useState(() => {
		installBackend("a");
		queryClient.setQueryData(["getSettingsProfile"], {
			hub_profile: structuredClone(profiles.current.b),
		});
		queryClient.setQueryDefaults(["getSettingsProfile"], {
			staleTime: Number.POSITIVE_INFINITY,
		});
		Object.assign(window, {
			collectionsQa: { writes: writes.current, profiles: profiles.current },
		});
		return true;
	});
	const appWidget = {
		...widget("apps", "app-collection", {
			source: "library",
			limit: 6,
			rendering,
		}),
		appearance: { variant: surface, accent: "neutral" },
	};
	const modelWidget = widget("models", "models", {
		source: "explore",
		limit: 6,
		rendering: modelRendering,
	});
	if (!ready) return null;
	return (
		<main className="min-h-screen bg-background p-4 text-foreground">
			<h1 className="mb-3 text-xl font-semibold">
				Native collections · local fixture
			</h1>
			<div className="mb-5 flex flex-wrap gap-3">
				<label>
					Profile{" "}
					<select
						aria-label="Fixture profile"
						value={profileId}
						onChange={(event) => {
							const id = event.target.value as "a" | "b";
							installBackend(id);
							setProfileId(id);
						}}
					>
						<option value="a">Profile A</option>
						<option value="b">Profile B</option>
					</select>
				</label>
				<label>
					Scenario{" "}
					<select
						aria-label="Fixture scenario"
						onChange={(event) => {
							scenario.current = event.target.value;
							void queryClient.invalidateQueries({ queryKey: ["home"] });
						}}
					>
						<option value="populated">Populated</option>
						<option value="empty">Empty</option>
						<option value="error">Error</option>
					</select>
				</label>
				<label>
					Surface{" "}
					<select
						aria-label="Fixture surface"
						value={surface}
						onChange={(event) => setSurface(event.target.value)}
					>
						<option>card</option>
						<option>borderless</option>
						<option>tinted</option>
					</select>
				</label>
			</div>
			<div className="grid items-start gap-6 grid-cols-[repeat(auto-fit,minmax(min(100%,320px),1fr))]">
				<section className="min-w-0">
					<h2 className="mb-3 text-base font-semibold">App card settings</h2>
					<HomeWidgetSettings
						widget={appWidget}
						onChange={(config) => setRendering(String(config.rendering))}
					/>
				</section>
				<section className="min-w-0">
					<h2 className="mb-3 text-base font-semibold">Model card settings</h2>
					<HomeWidgetSettings
						widget={modelWidget}
						onChange={(config) => setModelRendering(String(config.rendering))}
					/>
				</section>
			</div>
			<div
				key={profileId}
				className="mt-6 grid items-start gap-6 grid-cols-[repeat(auto-fit,minmax(min(100%,320px),1fr))]"
			>
				<section
					data-testid="native-apps"
					className="min-w-0 rounded-xl border p-4"
				>
					<h2 className="mb-3 text-base font-semibold">Profile library</h2>
					<HomeWidgetContent widget={appWidget} />
				</section>
				<section
					data-testid="native-editorial"
					className="min-w-0 rounded-xl border p-4"
				>
					<h2 className="mb-3 text-base font-semibold">Featured apps</h2>
					<HomeWidgetContent
						widget={widget("editorial", "app-collection", {
							source: "manual",
							appIds: ["collection-app-0", "collection-app-3"],
							rendering: "editorial",
						})}
					/>
				</section>
				<section
					data-testid="native-models"
					className="min-w-0 rounded-xl border p-4"
				>
					<h2 className="mb-3 text-base font-semibold">Explore models</h2>
					<HomeWidgetContent widget={modelWidget} />
				</section>
				<section
					data-testid="profile-models"
					className="min-w-0 rounded-xl border p-4"
				>
					<h2 className="mb-3 text-base font-semibold">Profile models</h2>
					<HomeWidgetContent
						widget={widget("profile-models", "models", {
							source: "profile",
							rendering: modelRendering,
						})}
					/>
				</section>
			</div>
		</main>
	);
}
