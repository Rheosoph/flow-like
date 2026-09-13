"use client";

import { useEffect, useRef, useState } from "react";
import { useAuth } from "react-oidc-context";
import { useInvoke } from "../../hooks/use-invoke";
import { parseAppRouteTarget } from "../../lib/app-route-url";
import {
	NATIVE_WIDGETS_CHANGED,
	type NativeWidgetPageDefinition,
	nativeWidgetScope,
	readNativeWidgetDefinitions,
} from "../../lib/native-widget";
import {
	NATIVE_WIDGET_PAGE_CAPTURE,
	type NativeWidgetPageCaptureDetail,
	projectNativeWidgetPage,
} from "../../lib/native-widget-page";
import { useBackend, useBackendReady } from "../../state/backend-state";
import type { IPage } from "../../state/backend-state/page-state";
import { useData } from "../a2ui/DataContext";
import type { Surface } from "../a2ui/types";

function querySignature(search: string): string {
	const params = new URLSearchParams(search);
	params.sort();
	return JSON.stringify([...params]);
}

export function matchingNativeWidgetPages(
	scope: string,
	appId: string,
	path: string | undefined,
	search: string,
): NativeWidgetPageDefinition[] {
	let route: ReturnType<typeof parseAppRouteTarget>;
	try {
		route = parseAppRouteTarget(path);
	} catch {
		return [];
	}
	const query = querySignature(search);
	return readNativeWidgetDefinitions(scope).filter(
		(definition): definition is NativeWidgetPageDefinition =>
			definition.kind === "page" &&
			definition.appId === appId &&
			(parseAppRouteTarget(definition.path).path ?? "/") ===
				(route.path ?? "/") &&
			querySignature(
				new URLSearchParams(
					definition.queryParams.map(({ name, value }) => [name, value]),
				).toString(),
			) === query,
	);
}

/** Captures only configured, visible pages after their existing load Event completes. */
export function NativeWidgetPageCapture({
	scope,
	appId,
	page,
	surface,
	path,
	search,
	pageRevision,
	ready,
}: {
	scope: string;
	appId: string;
	page: IPage;
	surface: Surface;
	path?: string;
	search: string;
	pageRevision: string;
	ready: boolean;
}) {
	const { data } = useData();
	const [definitions, setDefinitions] = useState<NativeWidgetPageDefinition[]>(
		[],
	);
	useEffect(() => {
		const changed = () =>
			setDefinitions(matchingNativeWidgetPages(scope, appId, path, search));
		changed();
		window.addEventListener(NATIVE_WIDGETS_CHANGED, changed);
		document.addEventListener("visibilitychange", changed);
		return () => {
			window.removeEventListener(NATIVE_WIDGETS_CHANGED, changed);
			document.removeEventListener("visibilitychange", changed);
		};
	}, [scope, appId, path, search]);

	useEffect(() => {
		if (!ready || document.visibilityState === "hidden") return;
		if (!definitions.length) return;
		const timeout = window.setTimeout(() => {
			if (document.visibilityState === "hidden") return;
			for (const definition of definitions) {
				const detail: NativeWidgetPageCaptureDetail = {
					scope,
					definitionId: definition.id,
					revision: definition.updatedAt,
					result: projectNativeWidgetPage(page, definition, { surface, data }),
					capturedAt: Date.now(),
					pageId: page.id,
					pageRevision,
				};
				window.dispatchEvent(
					new CustomEvent(NATIVE_WIDGET_PAGE_CAPTURE, { detail }),
				);
			}
		}, 500);
		return () => window.clearTimeout(timeout);
	}, [scope, page, surface, pageRevision, ready, data, definitions]);
	return null;
}

type CaptureBridgeProps = {
	appId: string;
	page: IPage;
	surface: Surface;
	path?: string;
	search: string;
	pageRevision: string;
	ready: boolean;
};

export function NativeWidgetPageCaptureBridge(props: CaptureBridgeProps) {
	const backend = useBackend();
	const backendReady = useBackendReady();
	const auth = useAuth();
	const viewerId = auth.isAuthenticated ? auth.user?.profile.sub : undefined;
	const backendScope = nativeWidgetScope(backend, viewerId);
	// A mounted page can still hold the previous workspace's surface after a profile switch.
	const ownerScope = useRef(backendScope);
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
		backendReady && !auth.isLoading,
		[],
		0,
	);
	const identityReady =
		backendReady &&
		!auth.isLoading &&
		profile.isSuccess &&
		profile.isFetchedAfterMount &&
		Boolean(profile.data?.id) &&
		(!auth.isAuthenticated || Boolean(viewerId));
	if (!identityReady) return null;
	const scope = nativeWidgetScope({ profile: profile.data }, viewerId);
	if (scope !== ownerScope.current || scope !== backendScope) return null;
	return <NativeWidgetPageCapture {...props} scope={scope} />;
}
