"use client";

import { useTranslation } from "@flow-like/locales";
import { useQueryClient } from "@tanstack/react-query";
import {
	type ReactNode,
	createContext,
	useCallback,
	useContext,
	useEffect,
	useMemo,
	useRef,
	useSyncExternalStore,
} from "react";
import { AuthContext, type AuthContextProps } from "react-oidc-context";
import { useInvoke } from "../../../../hooks/use-invoke";
import { getApiOrigin } from "../../../../lib/api-url";
import { loadDeviceCrypto } from "../../../../lib/device-management/crypto";
import {
	type DeviceAccountScope,
	accountStorageKey,
} from "../../../../lib/device-management/storage";
import type { DeviceCrypto } from "../../../../lib/device-management/types";
import {
	type WorkspaceSwitch,
	dismissWorkspaceSwitch,
	disposeAllDeviceWorkspaces,
	disposeDeviceWorkspace,
	getDeviceWorkspace,
	lastWorkspaceSwitch,
	subscribeWorkspaceSwitch,
} from "../../../../lib/device-management/workspace/registry";
import type {
	DeviceWorkspace,
	WorkspaceDeps,
} from "../../../../lib/device-management/workspace/types";
import { isTauri } from "../../../../lib/platform";
import {
	useAuthStatusStore,
	useBackend,
	useBackendReady,
} from "../../../../state/backend-state";
import type { IProfile } from "../../../../types";
import { DvButton } from "../primitives/dv-button";
import { StateView } from "../primitives/state-view";
import { AttentionProvider } from "./use-attention";

export interface DeviceAuth {
	loading: boolean;
	signedIn: boolean;
	issuer: string;
	account: string;
	/** Starts the host's sign-in flow, when it offers one. */
	signIn?: () => void;
}

/** Replaces what the provider reads from the host (tests, previews). */
export interface DeviceWorkspaceOverrides {
	auth?: DeviceAuth;
	profile?: IProfile;
	platform?: "desktop" | "web";
	/** Epoch milliseconds. */
	now?: () => number;
	crypto?: () => Promise<DeviceCrypto>;
	/** Replaces the module registry, e.g. `createDeviceWorkspace` over fakes. Must return the same workspace for the same scope. */
	workspace?: (deps: WorkspaceDeps) => DeviceWorkspace;
}

/** Why there is no workspace yet: the states that come before any hub read. */
export type WorkspaceGate =
	| { kind: "loading" }
	| { kind: "signed_out"; signIn?: () => void }
	| { kind: "profile_unavailable"; retry: () => void };

export interface DeviceWorkspaceProviderProps {
	children: ReactNode;
	/**
	 * No area gates and no fleet-wide hub reads (Events column): without a
	 * workspace `fallback` renders in place of the children.
	 */
	passive?: boolean;
	fallback?: ReactNode;
	/**
	 * Replaces the built-in gate block of a non-passive provider; the area
	 * shell passes its own so its top bar and status bar stay. Children are
	 * never rendered without a workspace, so they can call `useDeviceWorkspace()`.
	 */
	renderGate?: (gate: WorkspaceGate) => ReactNode;
	overrides?: DeviceWorkspaceOverrides;
}

interface WorkspaceBinding {
	workspace: DeviceWorkspace;
	scope: DeviceAccountScope;
	auth: DeviceAuth;
	passive: boolean;
}

const WorkspaceContext = createContext<WorkspaceBinding | null>(null);

function authOf(oidc: AuthContextProps | undefined): DeviceAuth {
	const account = oidc?.user?.profile.sub ?? "";
	return {
		loading: oidc?.isLoading ?? false,
		signedIn: Boolean(oidc?.isAuthenticated && account),
		issuer: oidc?.user?.profile.iss ?? "",
		account,
		...(oidc
			? {
					signIn: () =>
						void oidc.signinRedirect({
							url_state:
								typeof window === "undefined"
									? undefined
									: window.location.pathname + window.location.search,
						}),
				}
			: {}),
	};
}

let signOutWatch: (() => void) | undefined;

/** A sign-out anywhere in the app locks every key session, also while the area is not mounted. */
function watchSignOut() {
	signOutWatch ??= useAuthStatusStore.subscribe((state) => {
		if (state.signedIn === false) void disposeAllDeviceWorkspaces();
	});
}

function closeWorkspace(workspace: DeviceWorkspace) {
	void disposeDeviceWorkspace(workspace.scopeKey);
	void workspace.dispose();
}

function WorkspaceGateView({ gate }: Readonly<{ gate: WorkspaceGate }>) {
	const { t } = useTranslation("devices");
	if (gate.kind === "loading") return <StateView kind="loading" rows={4} />;
	if (gate.kind === "signed_out")
		return (
			<StateView
				kind="gate"
				gate="hub"
				title={t("action.gate.signIn", "Sign in to manage devices")}
				actions={
					gate.signIn ? (
						<DvButton variant="primary" onClick={gate.signIn}>
							{t("action.gate.signInButton", "Sign in")}
						</DvButton>
					) : undefined
				}
			/>
		);
	return (
		<StateView
			kind="error"
			title={t("action.gate.profile", "Your profile couldn't be loaded")}
			text={t(
				"action.gate.profileText",
				"Devices are listed per profile and hub. Check your connection and try again.",
			)}
			actions={
				<DvButton onClick={gate.retry}>
					{t("action.gate.retry", "Retry")}
				</DvButton>
			}
		/>
	);
}

interface HostAccount {
	auth: DeviceAuth;
	profile: IProfile | undefined;
	scope: DeviceAccountScope | undefined;
	/** The host has said who is signed in (the backend is up, or a test stated it). */
	signedOut: boolean;
	profileFailed: boolean;
	retryProfile(): void;
}

/** Who is signed in and on which profile and hub: the account scope (`profileId` falls back to "default"). */
function useHostAccount(overrides: DeviceWorkspaceOverrides | undefined) {
	const backend = useBackend();
	const ready = useBackendReady();
	const oidc = useContext(AuthContext);
	const override = overrides?.auth;
	const auth = useMemo(() => override ?? authOf(oidc), [override, oidc]);
	const profileQuery = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
		ready && auth.signedIn && !overrides?.profile,
		[auth.issuer, auth.account],
	);
	const profile = overrides?.profile ?? profileQuery.data;
	const apiOrigin = profile ? getApiOrigin(profile) : "";
	const profileId = profile?.id ?? "default";
	const { signedIn, issuer, account } = auth;
	const scope = useMemo<DeviceAccountScope | undefined>(
		() =>
			signedIn && apiOrigin
				? { issuer, account, apiOrigin, profileId }
				: undefined,
		[signedIn, issuer, account, apiOrigin, profileId],
	);
	const hostReady = ready || override !== undefined;
	const { isError, refetch } = profileQuery;
	const result: HostAccount = {
		auth,
		profile,
		scope,
		signedOut: hostReady && !auth.loading && !signedIn,
		profileFailed: isError || (signedIn && !!profile && !scope),
		retryProfile: () => void refetch(),
	};
	return result;
}

const hostPlatform = () => (isTauri() ? "desktop" : "web");

/** What a workspace is built from; `undefined` until the account scope is known. */
function useWorkspaceDeps(
	{ scope, profile }: HostAccount,
	overrides: DeviceWorkspaceOverrides = {},
) {
	const { apiState } = useBackend();
	const queryClient = useQueryClient();
	const { crypto = loadDeviceCrypto, platform, now } = overrides;
	return useMemo(() => {
		if (!scope || !profile) return undefined;
		const deps: WorkspaceDeps = {
			api: apiState,
			profile,
			scope,
			queryClient,
			crypto,
			platform: platform ?? hostPlatform(),
			now,
		};
		return deps;
	}, [scope, profile, apiState, queryClient, crypto, platform, now]);
}

/** Another scope, or a sign-out, closes what was open; while the session is still loading it stays. */
function useCloseReplaced(
	workspace: DeviceWorkspace | undefined,
	signedOut: boolean,
) {
	const last = useRef<DeviceWorkspace | undefined>(undefined);
	useEffect(() => {
		watchSignOut();
		if (!workspace && !signedOut) return;
		const previous = last.current;
		if (previous && previous !== workspace) closeWorkspace(previous);
		last.current = workspace;
	}, [workspace, signedOut]);
}

/** One workspace per scope key; a new backend object or a refreshed profile only rebinds it. */
function useScopeWorkspace(
	account: HostAccount,
	overrides: DeviceWorkspaceOverrides | undefined,
) {
	const deps = useWorkspaceDeps(account, overrides);
	const scopeKey = deps && accountStorageKey(deps.scope);
	const latestDeps = useRef(deps);
	latestDeps.current = deps;
	const create = overrides?.workspace ?? getDeviceWorkspace;
	// biome-ignore lint/correctness/useExhaustiveDependencies: one workspace per scope key; api and profile changes rebind below
	const workspace = useMemo(() => {
		const current = latestDeps.current;
		return current && create(current);
	}, [scopeKey, create]);

	const boundDeps = useRef(deps);
	useEffect(() => {
		if (deps && boundDeps.current !== deps) create(deps);
		boundDeps.current = deps;
	}, [deps, create]);

	useCloseReplaced(workspace, account.signedOut);
	return workspace;
}

const gateOf = (account: HostAccount): WorkspaceGate => {
	const { signIn } = account.auth;
	if (account.signedOut)
		return signIn ? { kind: "signed_out", signIn } : { kind: "signed_out" };
	return account.profileFailed
		? { kind: "profile_unavailable", retry: account.retryProfile }
		: { kind: "loading" };
};

/**
 * Binds the account scope's workspace to React (M-DATA §3.1). The workspace
 * lives in the module registry, so unmounting keeps key sessions; another
 * account, hub or profile, or a sign-out, locks and disposes it.
 */
export function DeviceWorkspaceProvider({
	children,
	passive = false,
	fallback = null,
	renderGate,
	overrides,
}: Readonly<DeviceWorkspaceProviderProps>) {
	const account = useHostAccount(overrides);
	const workspace = useScopeWorkspace(account, overrides);
	const { scope, auth } = account;

	const binding = useMemo<WorkspaceBinding | null>(
		() => (workspace && scope ? { workspace, scope, auth, passive } : null),
		[workspace, scope, auth, passive],
	);

	if (binding)
		return (
			<WorkspaceContext.Provider value={binding}>
				<AttentionProvider workspace={binding.workspace} passive={passive}>
					{children}
				</AttentionProvider>
			</WorkspaceContext.Provider>
		);
	if (passive) return <>{fallback}</>;
	const gate = gateOf(account);
	return (
		<>{renderGate ? renderGate(gate) : <WorkspaceGateView gate={gate} />}</>
	);
}

function useBinding(): WorkspaceBinding {
	const binding = useContext(WorkspaceContext);
	if (!binding)
		throw new Error(
			"useDeviceWorkspace() needs a DeviceWorkspaceProvider with a signed-in account above it.",
		);
	return binding;
}

export function useDeviceWorkspace(): DeviceWorkspace {
	return useBinding().workspace;
}

/** `undefined` outside a provider and while there is no signed-in scope. */
export function useOptionalDeviceWorkspace(): DeviceWorkspace | undefined {
	return useContext(WorkspaceContext)?.workspace;
}

export function useDeviceScope(): DeviceAccountScope | undefined {
	return useContext(WorkspaceContext)?.scope;
}

/** The signed-in account behind the workspace (G1) and the host's sign-in action. */
export function useDeviceAuth(): DeviceAuth {
	return useBinding().auth;
}

export function useWorkspacePassive(): boolean {
	return useContext(WorkspaceContext)?.passive ?? false;
}

const UNSET = Symbol("unset");

/**
 * A manager snapshot in React: `read` may build a new object per call, so the
 * last value is kept while `equal` holds. Handles stay inside the managers;
 * only what `read` returns reaches a component.
 */
export function useManagerValue<T>(
	subscribe: (listener: () => void) => () => void,
	read: () => T,
	equal: (previous: T, next: T) => boolean = Object.is,
): T {
	const last = useRef<T | typeof UNSET>(UNSET);
	const snapshot = useCallback(() => {
		const next = read();
		const previous = last.current;
		if (previous !== UNSET && equal(previous, next)) return previous;
		last.current = next;
		return next;
	}, [read, equal]);
	return useSyncExternalStore(subscribe, snapshot, snapshot);
}

export function shallowEqual<T extends object>(previous: T, next: T): boolean {
	if (previous === next) return true;
	const keys = Object.keys(previous) as (keyof T)[];
	return (
		keys.length === Object.keys(next).length &&
		keys.every((key) => Object.is(previous[key], next[key]))
	);
}

export interface ScopeSwitchNotice {
	/**
	 * The latest scope change that closed a workspace (`to` is absent after a
	 * sign-out; `lockedSessions` counts the key sessions it locked), also when
	 * it happened on another page. `undefined` once dismissed.
	 */
	notice: WorkspaceSwitch | undefined;
	dismiss(): void;
}

/** S02: "Keys locked because you switched …" after another account, hub or profile took over. */
export function useScopeSwitchNotice(): ScopeSwitchNotice {
	const notice = useSyncExternalStore(
		subscribeWorkspaceSwitch,
		lastWorkspaceSwitch,
		lastWorkspaceSwitch,
	);
	return useMemo(() => ({ notice, dismiss: dismissWorkspaceSwitch }), [notice]);
}
