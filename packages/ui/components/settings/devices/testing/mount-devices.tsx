import { QueryClientProvider } from "@tanstack/react-query";
import { type ReactNode, act } from "react";
import type { AuthContextProps } from "react-oidc-context";
import type {
	AppEventForm,
	AppInput,
} from "../../../../lib/device-management/model/app-plan";
import type { IEventInput } from "../../../../lib/schema/flow/event";
import {
	type IBackendState,
	type IOwnRole,
	useBackendStore,
} from "../../../../state/backend-state";
import type { DevicesHost, NavigationMode } from "../routing/devices-route";
import type {
	DeviceAuth,
	DeviceWorkspaceOverrides,
} from "../workspace/device-workspace-provider";
import type { DeviceSeed } from "./fake-device-api";
import {
	type FakeWorkspace,
	type FakeWorkspaceOptions,
	createFakeWorkspace,
} from "./fake-workspace";

/*
 * Mounts a device screen, shell part or hook probe over the fake workspace
 * (plan §2.4, M-TEST §4.1). Import it like the components under test, after
 * `installDom()`: a static import loads a large module graph before the DOM
 * exists, and timers a previous test file left behind then run without one.
 *
 *   const dom = installDom();
 *   const { mountDevices, cleanupDevices, preloadDevices } = await import(
 *     "../testing/mount-devices"
 *   );
 *   await preloadDevices();
 *   afterEach(async () => { await cleanupDevices(); await dom.cleanup(); });
 *   afterAll(dom.restore);
 *
 *   const { fake, container } = await mountDevices(<Screen />);
 *   expect(fake.api.writes()).toEqual([]);
 *
 * A node that mounts the providers itself:
 *
 *   await mountDevices(
 *     ({ overrides }) => <DevicesArea scope="account" harness={{ overrides }} />,
 *     { providers: false },
 *   );
 */

export interface MountedNavigation {
	mode: NavigationMode;
	href: string;
}

export interface MountDevicesOptions extends FakeWorkspaceOptions {
	/** The fleet; the golden sample by default. */
	seed?: DeviceSeed;
	/** A fake workspace made beforehand; `cleanupDevices()` disposes it too. */
	fake?: FakeWorkspace;
	/** The page that hosts the area and its query string (`MemoryDevicesRoute`). */
	host?: DevicesHost;
	search?: string;
	/** `false`: only the host contexts (query client, sign-in, route); the node mounts its own providers, e.g. `DevicesArea`. */
	providers?: boolean;
	/** Mount the workspace provider without area gates and without the area contexts (Events column). */
	passive?: boolean;
	/** Also mount `AreaOverlays` (unlock, diagnose and plane sheets). */
	overlays?: boolean;
	signedIn?: boolean;
	widthBucket?: "phone" | "narrow" | "medium" | "wide";
	/** The area clock's tick; stopped by default. */
	tickMs?: number | false;
	/** Apps the fake backend and the fake hub know, by id; the sample apps by default. */
	apps?: Readonly<Record<string, AppInput>>;
	/** Backend states merged over the fake backend. */
	backend?: Partial<IBackendState>;
}

/** What a node needs when it mounts the providers itself (`providers: false`). */
export interface MountContext {
	fake: FakeWorkspace;
	/** For `<DevicesArea harness={{ overrides }} />` or `<DeviceWorkspaceProvider overrides={overrides}>`. */
	overrides: DeviceWorkspaceOverrides;
}

export interface MountedDevices {
	container: HTMLElement;
	fake: FakeWorkspace;
	/** What the provider reads instead of the host; pass it to a node that mounts its own provider. */
	overrides: DeviceWorkspaceOverrides;
	/** Every in-area navigation, oldest first. */
	navigations: MountedNavigation[];
	/** How often the host's sign-in was started. */
	signIns(): number;
	rerender(node: ReactNode): Promise<void>;
	/** Flush pending reads and store updates inside `act`. */
	settle(): Promise<void>;
	/** Unmount and dispose the fake workspace. */
	unmount(): Promise<void>;
}

const mounted = new Set<MountedDevices>();

/** Unmount everything `mountDevices` mounted and dispose the fake workspaces; call in `afterEach`. */
export async function cleanupDevices(): Promise<void> {
	for (const entry of [...mounted]) await entry.unmount();
}

function missing(state: string, name: string): never {
	throw new Error(
		`The fake backend has no ${state}.${name}(). Pass it to mountDevices through options.backend.`,
	);
}

function strict<T extends object>(state: string, known: T): T {
	return new Proxy(known, {
		get(target, name, receiver) {
			if (typeof name === "symbol" || name in target || name === "then")
				return Reflect.get(target, name, receiver);
			return () => missing(state, name);
		},
	});
}

function appOf(apps: Readonly<Record<string, AppInput>>, appId: string) {
	const app = apps[appId];
	if (!app) throw new Error(`The fake backend has no app ${appId}.`);
	return app;
}

/** The sample keeps one version of each event: the one every pin names. */
function eventOf(
	apps: Readonly<Record<string, AppInput>>,
	appId: string,
	eventId: string,
) {
	for (const event of appOf(apps, appId).events)
		if (event.id === eventId) return event;
	throw new Error(`The fake backend has no event ${eventId} in ${appId}.`);
}

/** The app record with its own version text and last change, as the real one carries them: the sample's newest version. */
function appRecord({ id, visibility, versions }: AppInput) {
	const newest = versions?.[0];
	const version = newest?.label?.replace(/^v(?=\d)/, "");
	const changed = newest?.builtAt;
	return Object.assign(
		{ id, visibility },
		version ? { version } : {},
		changed
			? { updated_at: { secs_since_epoch: changed, nanos_since_epoch: 0 } }
			: {},
	);
}

/** A form's fields as its record lists them: its file fields as `PathBuf`, the others as text. */
function formInputs({ fields, fileFields }: AppEventForm) {
	const inputs: IEventInput[] = [];
	for (let index = 0; index < fields; index++)
		inputs.push({
			id: `pin_field_${index}`,
			name: `field_${index + 1}`,
			friendly_name: `Field ${index + 1}`,
			description: "",
			data_type: index >= fields - fileFields ? "PathBuf" : "String",
			value_type: "Normal",
			index,
		});
	return inputs;
}

/**
 * An event as the app's store returns it: its config bytes (the sample's
 * `config`, else its schedule), a form's fields as `inputs` and the flow as
 * `board_id`. What the model derives (`flow`, `form`) is left out; `inputs`
 * the sample gives itself win.
 */
export function eventRecord({
	schedule,
	boardId,
	flow: _flow,
	form,
	...event
}: AppInput["events"][number]) {
	return {
		...(form ? { inputs: formInputs(form) } : {}),
		...event,
		board_id: boardId ?? "flow_main",
		config: event.config?.length
			? [...event.config]
			: schedule
				? [...new TextEncoder().encode(JSON.stringify(schedule))]
				: [],
	};
}

/** "flow_review" → "Review flow": the sample's flows are named after their ids. */
export const sampleFlowName = (boardId: string) =>
	`${boardId.replace(/^flow_/, "").replace(/^./, (first) => first.toUpperCase())} flow`;

/** The flows the app's events use, as the summaries a flow list reads: id and name. */
function flowSummaries(app: AppInput) {
	const ids = [
		...new Set(app.events.map((event) => event.boardId ?? "flow_main")),
	];
	return ids.sort().map((id) => ({ id, name: sampleFlowName(id) }));
}

/** The sample viewer owns the sample apps, as the prototype shows them. */
const OWNER_ROLE: IOwnRole = {
	role_id: "role_owner",
	role_name: "Owner",
	permissions: 1,
	is_owner: true,
	can_leave: false,
};

/**
 * The host backend: the fake hub as `apiState`, the profile, the hub's apps
 * with their events, the Owner role on every app, and on the desktop this
 * computer's own triggers (`sinkState`, from `fake.sinks`; the web has none).
 * Another role, or none (`roleState: undefined`), goes in through `extra`.
 */
export function fakeBackend(
	fake: FakeWorkspace,
	apps: Readonly<Record<string, AppInput>> = fake.hub.apps,
	extra: Partial<IBackendState> = {},
): IBackendState {
	fake.hub.apps = { ...apps };
	const backend = {
		apiState: fake.api,
		userState: strict("userState", {
			getProfile: async () => fake.profile,
			getInfo: async () => ({ id: fake.hub.me, dev_mode: false }),
			updateUser: async () => undefined,
		}),
		appState: strict("appState", {
			getApps: async () =>
				Object.values(apps).map((app) => [
					{ id: app.id, visibility: app.visibility },
					{ name: app.name, description: "" },
				]),
			getApp: async (appId: string) => appRecord(appOf(apps, appId)),
			getAppMeta: async (appId: string) => ({
				name: appOf(apps, appId).name,
				description: "",
			}),
		}),
		eventState: strict("eventState", {
			getEvents: async (appId: string) =>
				appOf(apps, appId).events.map(eventRecord),
			getEventAuthoritative: async (appId: string, eventId: string) =>
				eventRecord(eventOf(apps, appId, eventId)),
			// This computer runs only what the test gave it (`localTriggers`).
			isEventSinkActive: async (eventId: string) =>
				fake.sinks.state().isEventSinkActive(eventId),
		}),
		sinkState:
			fake.deps.platform === "desktop" ? fake.sinks.state() : undefined,
		roleState: strict("roleState", {
			getOwnRole: async (_appId: string) => OWNER_ROLE,
		}),
		boardState: strict("boardState", {
			getBoardSummaries: async (appId: string) =>
				flowSummaries(appOf(apps, appId)),
		}),
		capabilities: () => ({
			needsSignIn: false,
			canHostLlamaCPP: false,
			canHostMLX: false,
			canHostEmbeddings: false,
			canExecuteLocally: fake.deps.platform === "desktop",
		}),
		isOffline: async () => false,
		...extra,
	};
	return strict("backend", backend) as unknown as IBackendState;
}

function oidcOf(
	fake: FakeWorkspace,
	signedIn: boolean,
	onSignIn: () => void,
): AuthContextProps {
	return {
		isAuthenticated: signedIn,
		isLoading: false,
		user: signedIn
			? { profile: { sub: fake.scope.account, iss: fake.scope.issuer } }
			: null,
		signinRedirect: async () => onSignIn(),
	} as unknown as AuthContextProps;
}

async function loadProviders() {
	const [client, oidc, provider, area, confirm, route, overlays, flows] =
		await Promise.all([
			import("react-dom/client"),
			import("react-oidc-context"),
			import("../workspace/device-workspace-provider"),
			import("../workspace/area-context"),
			import("../primitives/confirm-sheet"),
			import("../routing/use-devices-route"),
			import("../overlays/area-overlays"),
			import("../workspace/use-latest-flows"),
		]);
	return {
		createRoot: client.createRoot,
		AuthContext: oidc.AuthContext,
		DeviceWorkspaceProvider: provider.DeviceWorkspaceProvider,
		AreaProvider: area.AreaProvider,
		ConfirmProvider: confirm.ConfirmProvider,
		MemoryDevicesRoute: route.MemoryDevicesRoute,
		AreaOverlays: overlays.AreaOverlays,
		latestFlowSeams: flows.latestFlowSeams,
	};
}

/** A local-only app's flows live on this computer: the desktop commands answer from the fake hub's flow state. */
function desktopFlows(fake: FakeWorkspace) {
	return async () => ({
		read: async (appId: string, boardId: string) =>
			fake.hub.flows.state(appId, boardId),
		publish: async (appId: string, boardId: string) =>
			fake.hub.flows.publish(appId, boardId),
	});
}

let providers: ReturnType<typeof loadProviders> | undefined;

/**
 * Loads react-dom and the area providers once (several seconds in a cold
 * process). Await it at the top level of the test file, after `installDom()`,
 * so the first test does not pay for it inside its own timeout.
 */
export function preloadDevices(): Promise<void> {
	providers ??= loadProviders();
	return providers.then(() => undefined);
}

type Providers = Awaited<ReturnType<typeof loadProviders>>;

interface HostFrame {
	fake: FakeWorkspace;
	oidc: AuthContextProps;
	overrides: DeviceWorkspaceOverrides;
	onNavigate(navigation: MountedNavigation): void;
}

/** The area contexts a screen runs under; a passive mount (Events column) has none. */
function areaTree(
	parts: Providers,
	options: MountDevicesOptions,
	children: ReactNode,
): ReactNode {
	if (options.passive) return children;
	const { AreaProvider, ConfirmProvider, AreaOverlays } = parts;
	return (
		<AreaProvider
			widthBucket={options.widthBucket ?? "wide"}
			tickMs={options.tickMs ?? false}
		>
			<ConfirmProvider>
				{children}
				{options.overlays ? <AreaOverlays /> : null}
			</ConfirmProvider>
		</AreaProvider>
	);
}

function hostTree(
	parts: Providers,
	options: MountDevicesOptions,
	frame: HostFrame,
	children: ReactNode,
): ReactNode {
	const { AuthContext, DeviceWorkspaceProvider, MemoryDevicesRoute } = parts;
	return (
		<QueryClientProvider client={frame.fake.queryClient}>
			<AuthContext.Provider value={frame.oidc}>
				<MemoryDevicesRoute
					host={options.host ?? "account"}
					initialSearch={options.search ?? ""}
					onNavigate={frame.onNavigate}
				>
					{options.providers === false ? (
						children
					) : (
						<DeviceWorkspaceProvider
							overrides={frame.overrides}
							passive={options.passive}
						>
							{areaTree(parts, options, children)}
						</DeviceWorkspaceProvider>
					)}
				</MemoryDevicesRoute>
			</AuthContext.Provider>
		</QueryClientProvider>
	);
}

const macrotask = () =>
	act(async () => {
		await new Promise((resolve) => setTimeout(resolve, 0));
	});

/** Runs until no query is fetching (at most 25 rounds), then two rounds more for the updates that follow. */
async function settleQueries(fake: FakeWorkspace): Promise<void> {
	for (let round = 0; round < 25; round++) {
		await macrotask();
		if (!fake.queryClient.isFetching()) break;
	}
	await macrotask();
	await macrotask();
}

export async function mountDevices(
	node: ReactNode | ((context: MountContext) => ReactNode),
	options: MountDevicesOptions = {},
): Promise<MountedDevices> {
	await preloadDevices();
	const parts = await (providers as ReturnType<typeof loadProviders>);
	const fake =
		options.fake ?? (await createFakeWorkspace(options.seed, options));
	const signedIn = options.signedIn !== false;
	let signIns = 0;
	const onSignIn = () => {
		signIns += 1;
	};
	const auth: DeviceAuth = {
		loading: false,
		signedIn,
		issuer: signedIn ? fake.scope.issuer : "",
		account: signedIn ? fake.scope.account : "",
		signIn: onSignIn,
	};
	const overrides: DeviceWorkspaceOverrides = {
		auth,
		profile: fake.profile,
		platform: fake.deps.platform,
		now: fake.clock.now,
		crypto: fake.deps.crypto,
		workspace: () => fake.workspace,
	};
	const previousBackend = useBackendStore.getState().backend;
	useBackendStore
		.getState()
		.setBackend(fakeBackend(fake, options.apps, options.backend));
	const { latestFlowSeams } = parts;
	const previousFlows = latestFlowSeams.commands;
	latestFlowSeams.commands = desktopFlows(fake);

	const navigations: MountedNavigation[] = [];
	const frame: HostFrame = {
		fake,
		oidc: oidcOf(fake, signedIn, onSignIn),
		overrides,
		onNavigate: (navigation) => {
			navigations.push(navigation);
		},
	};
	const container = document.createElement("div");
	document.body.append(container);
	const root = parts.createRoot(container);
	const settle = () => settleQueries(fake);
	let alive = true;
	const entry: MountedDevices = {
		container,
		fake,
		overrides,
		navigations,
		signIns: () => signIns,
		async rerender(next) {
			await act(async () => root.render(hostTree(parts, options, frame, next)));
			await settle();
		},
		settle,
		async unmount() {
			if (!alive) return;
			alive = false;
			mounted.delete(entry);
			await act(async () => root.unmount());
			// Radix sheets and popovers finish unmounting in a 0 ms timer; it must run in this window.
			await macrotask();
			container.remove();
			useBackendStore.setState({ backend: previousBackend });
			latestFlowSeams.commands = previousFlows;
			await fake.dispose();
		},
	};
	mounted.add(entry);
	await entry.rerender(
		typeof node === "function" ? node({ fake, overrides }) : node,
	);
	return entry;
}
