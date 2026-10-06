"use client";

import { useContext, useMemo, useState } from "react";
import { AuthContext, type AuthContextProps } from "react-oidc-context";
import { type IBackendState, useBackend } from "../../../state/backend-state";
import type { FormHostKind, FormWorkbenchProps } from "./contracts";
import {
	formHostKindOf,
	resolveHostCapabilities,
	resolveViewerHabits,
	viewerInputOf,
} from "./session/host";
import { useFormSession } from "./session/use-form-session";
import { useWorkbenchNavigate } from "./shell/navigation";
import { WorkbenchShell } from "./shell/workbench-shell";

interface Identity {
	readonly signedIn: boolean;
	readonly memoryScope: string | null;
}

const SIGNED_OUT: Identity = { signedIn: false, memoryScope: null };

/**
 * PLAN §6: the desktop counts as signed in and keeps memory per profile; the web per OIDC user,
 * session memory when signed out. Hosted and service pages are never signed in.
 */
export function identityOf(
	kind: FormHostKind,
	backend: Pick<IBackendState, "eventState" | "profile">,
	auth: Pick<AuthContextProps, "isAuthenticated" | "user"> | undefined,
): Identity {
	if (kind !== "app") return SIGNED_OUT;
	if (backend.eventState.alwaysRemote !== true)
		return {
			signedIn: true,
			memoryScope: `profile:${backend.profile?.id ?? "local"}`,
		};
	if (!auth?.isAuthenticated) return SIGNED_OUT;
	const sub = auth.user?.profile.sub;
	return { signedIn: true, memoryScope: sub ? `user:${sub}` : null };
}

const viewerOfThisDevice = () =>
	resolveViewerHabits(
		viewerInputOf(typeof navigator === "undefined" ? undefined : navigator),
	);

/** The standard form interface: host props → session → shell. */
export function FormWorkbenchInterface(props: Readonly<FormWorkbenchProps>) {
	const { appId, event, config, toolbarRef, onNavigate } = props;
	const backend = useBackend();
	const auth = useContext(AuthContext);
	const kind = formHostKindOf(props.host, appId);
	const { signedIn, memoryScope } = identityOf(kind, backend, auth);
	const presentation = props.presentation ?? "page";
	const hasToolbar = toolbarRef !== undefined;
	const helperState = backend.helperState;
	const host = useMemo(
		() =>
			resolveHostCapabilities({
				kind,
				presentation,
				helperState,
				hasToolbar,
				signedIn,
				memoryScope,
			}),
		[kind, presentation, helperState, hasToolbar, signedIn, memoryScope],
	);
	const [viewer] = useState(viewerOfThisDevice);
	const navigate = useWorkbenchNavigate(appId, onNavigate);
	const { state, actions } = useFormSession({
		appId,
		event,
		config,
		host,
		viewer,
		navigate,
	});
	return (
		<WorkbenchShell
			state={state}
			actions={actions}
			appId={appId}
			toolbarRef={toolbarRef}
			navigate={navigate}
		/>
	);
}
