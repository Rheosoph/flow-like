import { afterAll, afterEach, expect, test } from "bun:test";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import {
	installDomGlobals,
	restoreDomGlobals,
	settle,
	domWindow as window,
} from "../components/bots-tab/testing/dom-harness";
import {
	type QueryStorageBackend,
	createSmartQueryPersister,
} from "../lib/query-persister";
import { STT_LOCAL_PROVIDER } from "../lib/realtime/call-listening";
import { type IBit, IBitTypes } from "../lib/schema/bit/bit";
import { useBackendStore } from "../state/backend-state";
import { type SpeechModelState, useSpeechModelState } from "./use-speech-model";

installDomGlobals();
const { createRoot } = await import("react-dom/client");
const previousBackend = useBackendStore.getState().backend;
const cleanups: Array<() => Promise<void>> = [];

afterEach(async () => {
	for (const cleanup of cleanups.splice(0)) await cleanup();
});
afterAll(() => {
	useBackendStore.setState({ backend: previousBackend });
	restoreDomGlobals();
});

const local = {
	id: "whisper",
	hub: "hub.example",
	type: IBitTypes.Stt,
	parameters: { provider: { provider_name: STT_LOCAL_PROVIDER } },
} as IBit;
const hosted = {
	id: "hosted-whisper",
	type: IBitTypes.Stt,
	parameters: { provider: { provider_name: "hosted:openrouter" } },
} as IBit;
const localChoice = { bitId: "hub.example:whisper", route: "local" };
const installedKey = ["speech-model-installed", "v2", [local.id]];

function memoryStorage(): QueryStorageBackend & { map: Map<string, string> } {
	const map = new Map<string, string>();
	return {
		map,
		get: async (key) => map.get(key),
		set: async (key, value) => {
			map.set(key, value);
		},
		del: async (key) => {
			map.delete(key);
		},
		entries: async () => [...map.entries()],
	};
}

function backend(installed = true) {
	let checks = 0;
	useBackendStore.getState().setBackend({
		capabilities: () => ({ canExecuteLocally: true }),
		audioState: {},
		userState: {
			getSettingsProfile: async () => ({ hub_profile: { id: "profile" } }),
		},
		bitState: {
			getProfileBits: async () => [hosted, local],
			isBitInstalled: async () => {
				checks++;
				return installed;
			},
		},
	} as never);
	return { checks: () => checks };
}

async function mount(
	persister: ReturnType<typeof createSmartQueryPersister>,
	localOnly = false,
) {
	const client = new QueryClient({
		defaultOptions: {
			queries: {
				staleTime: 30_000,
				retry: false,
				persister: persister.persisterFn,
			},
		},
	});
	const host = window.document.createElement("div");
	window.document.body.append(host);
	const errors: unknown[] = [];
	const root = createRoot(host as never, {
		onUncaughtError: (error) => errors.push(error),
	});
	let state: SpeechModelState | undefined;
	function Harness() {
		state = useSpeechModelState(true, undefined, localOnly);
		return null;
	}
	let disposed = false;
	const dispose = async () => {
		if (disposed) return;
		disposed = true;
		await act(async () => root.unmount());
		client.clear();
		host.remove();
	};
	cleanups.push(dispose);
	await act(async () => {
		root.render(
			<QueryClientProvider client={client}>
				<Harness />
			</QueryClientProvider>,
		);
	});
	// Profile, profile bits and installation checks resolve in successive renders.
	for (let i = 0; i < 4; i++) await settle();
	return { client, errors, state: () => state, dispose };
}

test("desktop selects an installed local speech model", async () => {
	const source = backend();
	const mounted = await mount(
		createSmartQueryPersister({ backend: memoryStorage() }),
	);
	expect(mounted.errors).toEqual([]);
	expect(mounted.state()?.choice).toEqual(localChoice);
	expect(mounted.state()?.settled).toBe(true);
	expect(source.checks()).toBe(1);
});

test("persisted installation IDs retain local speech after a reload", async () => {
	const source = backend();
	const storage = memoryStorage();
	const persister = createSmartQueryPersister({ backend: storage });
	const first = await mount(persister);
	expect(first.state()?.choice).toEqual(localChoice);
	expect(first.client.getQueryData(installedKey)).toEqual([local.id]);
	await persister.persistQueryByKey(installedKey, first.client);
	const saved = [...storage.map.values()]
		.map((value) => JSON.parse(value))
		.find((entry) => entry.queryKey[0] === "speech-model-installed");
	expect(saved.state.data).toEqual([local.id]);
	await first.dispose();

	const reloaded = await mount(persister);
	expect(reloaded.errors).toEqual([]);
	expect(reloaded.state()?.choice).toEqual(localChoice);
	expect(reloaded.state()?.settled).toBe(true);
	expect(source.checks()).toBe(1);
});

test("legacy serialized Sets are bypassed and installation is checked again", async () => {
	const source = backend();
	const storage = memoryStorage();
	const persister = createSmartQueryPersister({ backend: storage });
	const legacy = new QueryClient();
	const legacyKey = ["speech-model-installed", [local.id]];
	legacy.setQueryData(legacyKey, new Set([local.id]));
	await persister.persistQueryByKey(legacyKey, legacy);
	legacy.clear();
	expect(JSON.parse([...storage.map.values()][0]).state.data).toEqual({});

	const mounted = await mount(persister);
	expect(mounted.errors).toEqual([]);
	expect(mounted.state()?.choice).toEqual(localChoice);
	expect(mounted.state()?.settled).toBe(true);
	expect(source.checks()).toBe(1);
});

test("local-only speech stays unavailable when the local model is not installed", async () => {
	backend(false);
	const mounted = await mount(
		createSmartQueryPersister({ backend: memoryStorage() }),
		true,
	);
	expect(mounted.errors).toEqual([]);
	expect(mounted.state()?.choice).toBeNull();
	expect(mounted.state()?.settled).toBe(true);
});
