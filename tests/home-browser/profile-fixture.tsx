import { useQueryClient } from "@tanstack/react-query";
import { usePathname, useRouter } from "next/navigation";
import { useState } from "react";
import { Toaster } from "sonner";
import { ProfileTemplateEditorPage } from "../../packages/ui/components/profile-templates/profile-template-editor";
import { createProfileTemplate } from "../../packages/ui/components/profile-templates/profile-template-model";
import { ProfileTemplatesPage } from "../../packages/ui/components/profile-templates/profile-templates-page";
import {
	useBackend,
	useBackendStore,
} from "../../packages/ui/state/backend-state";

const storageKey = "profile-admin-fixture-v1";
const at = { secs_since_epoch: 1788566400, nanos_since_epoch: 0 };
const names = ["Knowledge Chat", "Invoice OCR", "Sheet Sync"];
const apps = names.map((name, i) => [
	{
		id: `fixture-app-${i}`,
		authors: [],
		bits: [],
		boards: [],
		events: [],
		page_ids: [],
		widget_ids: [],
		templates: [],
		status: "Active",
		visibility: "Private",
		execution_mode: "Remote",
		primary_category: "Productivity",
		created_at: at,
		updated_at: at,
		download_count: 0,
		rating_count: 0,
		rating_sum: 0,
	},
	{
		name,
		description: "A local fixture app for profile verification.",
		tags: ["productivity"],
		created_at: at,
		updated_at: at,
		preview_media: [],
	},
]);
const bits = [
	"Research language model",
	"Document embeddings",
	"Speech recognition",
].map((name, i) => ({
	id: `fixture-bit-${i}`,
	hub: "fixture.invalid",
	type: ["Llm", "Embedding", "Stt"][i],
	authors: [],
	dependencies: [],
	dependency_tree_hash: "fixture",
	hash: "fixture",
	created: "2026-09-05",
	updated: "2026-09-05",
	parameters: { provider: { provider_name: "Fixture provider" } },
	meta: {
		en: {
			name,
			description: "Local fixture model metadata. No model download occurs.",
			tags: [],
			preview_media: [],
			created_at: at,
			updated_at: at,
		},
	},
}));

export default function ProfileFixture() {
	const original = useBackend();
	const client = useQueryClient();
	const pathname = usePathname();
	const router = useRouter();
	const [fixture] = useState(() => {
		const initial = [
			{
				...createProfileTemplate("fixture.invalid"),
				id: "research",
				name: "Research & discovery",
				description:
					"A focused starting point for exploring sources, finding answers, and sharing what you learn.",
				tags: ["Research", "Knowledge"],
				interests: ["Data analysis"],
				bits: ["fixture.invalid:fixture-bit-0"],
				apps: [{ app_id: "fixture-app-0", pinned: true, favorite: true }],
			},
			{
				...createProfileTemplate("fixture.invalid"),
				id: "operations",
				name: "Operations desk",
				description: "Keep the team's daily work and useful apps close.",
				tags: ["Operations"],
				apps: [{ app_id: "fixture-app-1", pinned: false, favorite: true }],
			},
		];
		const stored = JSON.parse(sessionStorage.getItem(storageKey) || "null");
		// biome-ignore lint/suspicious/noExplicitAny: The fixture supplies a partial backend with inspectable test state.
		const state: any = stored ?? {
			templates: initial,
			writes: [],
			deletes: [],
			uploads: [],
			media: {},
			nextMedia: 1,
		};
		const save = () =>
			sessionStorage.setItem(storageKey, JSON.stringify(state));
		// biome-ignore lint/suspicious/noExplicitAny: The fixture supplies a partial backend with inspectable test state.
		const qa: any = {
			state,
			refetch: () =>
				client.invalidateQueries({ queryKey: ["profile-templates"] }),
			getSaved: () => structuredClone(state.templates),
		};
		// biome-ignore lint/suspicious/noExplicitAny: The fixture supplies a partial backend with inspectable test state.
		(window as any).profileQa = qa;
		const profile = {
			...createProfileTemplate("fixture.invalid"),
			id: "fixture-admin",
			name: "Fixture administrator",
			apps: apps.map(([app]) => ({
				app_id: app.id,
				favorite: false,
				pinned: false,
			})),
		};
		useBackendStore.getState().setBackend({
			...original,
			profile,
			userState: {
				...original.userState,
				getProfile: async () => profile,
				getInfo: async () => ({ permission: 152 }),
				getSettingsProfile: async () => ({ hub_profile: profile }),
			},
			appState: {
				...original.appState,
				getApps: async () => apps,
				searchApps: async (_id: unknown, query: string) =>
					apps.filter(([, meta]) =>
						meta.name.toLowerCase().includes(query.toLowerCase()),
					),
			},
			apiState: {
				...original.apiState,
				get: async (_profile: unknown, path: string) => {
					if (path === "info/profiles") return structuredClone(state.templates);
					if (path.startsWith("admin/profiles/media")) {
						const format =
							new URL(path, location.origin).searchParams.get("format") ??
							"webp";
						const mediaPath = `/profile-fixture-media/${state.nextMedia++}.${format}`;
						return {
							url: `${location.origin}${mediaPath}?signature=fixture`,
							final_url: `${location.origin}${mediaPath}`,
						};
					}
					throw new Error(`Unexpected fixture GET ${path}`);
				},
				// biome-ignore lint/suspicious/noExplicitAny: The fixture supplies a partial backend with inspectable test state.
				post: async (_profile: unknown, path: string, body: any) => {
					if (path !== "bit")
						throw new Error(`Unexpected fixture POST ${path}`);
					qa.lastBitRequest = structuredClone(body);
					return bits.filter(
						(bit) =>
							(!body.query ||
								bit.meta.en.name
									.toLowerCase()
									.includes(body.query.toLowerCase())) &&
							(!body.bit_types?.length || body.bit_types.includes(bit.type)),
					);
				},
				// biome-ignore lint/suspicious/noExplicitAny: The fixture supplies a partial backend with inspectable test state.
				put: async (_profile: unknown, path: string, body: any) => {
					if (!path.startsWith("admin/profiles/"))
						throw new Error(`Unexpected fixture PUT ${path}`);
					qa.saveAttempts = (qa.saveAttempts ?? 0) + 1;
					if (qa.holdSave)
						await new Promise<void>((resolve) => {
							qa.releaseSave = resolve;
						});
					if (qa.failSave)
						throw new Error(
							"Fixture save failed. Your changes are still here.",
						);
					const saved = structuredClone(body);
					state.templates = [
						// biome-ignore lint/suspicious/noExplicitAny: The fixture supplies a partial backend with inspectable test state.
						...state.templates.filter((item: any) => item.id !== saved.id),
						saved,
					];
					state.writes.push({ path, profile: saved });
					save();
					return saved;
				},
				del: async (_profile: unknown, path: string) => {
					if (qa.failDelete)
						throw new Error("Fixture deletion failed. Try again.");
					const id = decodeURIComponent(path.split("/").at(-1) ?? "");
					state.templates = state.templates.filter(
						// biome-ignore lint/suspicious/noExplicitAny: The fixture supplies a partial backend with inspectable test state.
						(item: any) => item.id !== id,
					);
					state.deletes.push(id);
					save();
				},
			},
			// biome-ignore lint/suspicious/noExplicitAny: The fixture supplies a partial backend with inspectable test state.
		} as any);
		return {
			upload: async (url: string, file: Blob) => {
				qa.uploadStarted = true;
				if (qa.holdUpload)
					await new Promise<void>((resolve) => {
						qa.releaseUpload = resolve;
					});
				if (qa.failUpload)
					throw new Error("Fixture image upload failed. Try again.");
				const bytes = new Uint8Array(await file.arrayBuffer());
				const bitmap = await createImageBitmap(file);
				state.uploads.push({
					type: file.type,
					url,
					bytes: file.size,
					width: bitmap.width,
					height: bitmap.height,
					signature: String.fromCharCode(...bytes.slice(0, 4)),
				});
				bitmap.close();
				let binary = "";
				for (const byte of bytes) binary += String.fromCharCode(byte);
				state.media[new URL(url).pathname] = btoa(binary);
				save();
			},
		};
	});
	return (
		// biome-ignore lint/a11y/useKeyWithClickEvents: This ancestor delegates native anchor clicks, including keyboard activation, to the test router.
		<div
			className="min-h-screen bg-background text-foreground"
			onClick={(event) => {
				const link = (event.target as HTMLElement).closest("a");
				if (
					!link ||
					event.defaultPrevented ||
					event.metaKey ||
					event.ctrlKey ||
					event.shiftKey ||
					event.altKey
				)
					return;
				const target = new URL(link.href);
				if (
					target.origin === location.origin &&
					target.pathname.startsWith("/admin/")
				) {
					event.preventDefault();
					router.push(target.pathname + target.search);
				}
			}}
		>
			<div className="border-b bg-card px-5 py-2 text-[11px] text-muted-foreground">
				Local profile verification · fixture data · remote requests blocked
			</div>
			{pathname === "/admin/profiles/add" ? (
				<ProfileTemplateEditorPage uploadMedia={fixture.upload} />
			) : pathname === "/admin/profiles" ? (
				<ProfileTemplatesPage />
			) : (
				<p data-testid="home-destination">
					Default home destination: {location.pathname + location.search}
				</p>
			)}
			<Toaster />
		</div>
	);
}
