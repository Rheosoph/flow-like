"use client";

import {
	PageInterface,
	type PageInterfaceProps,
} from "@flow-like/flow-like-ui/components/interfaces/page-interface";
import type { IPageBootstrap } from "@flow-like/flow-like-ui/state/backend-state/page-state";
import { useLayoutEffect, useSyncExternalStore } from "react";
import { flushSync } from "react-dom";

interface RuntimePageRequest {
	id: number;
	props: PageInterfaceProps;
}

// A generation callback can outlive the component that started it.
let requestedPage: RuntimePageRequest | null = null;
let nextRequestId = 0;
const listeners = new Set<() => void>();
const commitListeners = new Set<() => void>();

function notifyCommitListeners() {
	for (const listener of commitListeners) listener();
}

function subscribe(listener: () => void) {
	listeners.add(listener);
	return () => listeners.delete(listener);
}

function getSnapshot() {
	return requestedPage;
}

function getServerSnapshot() {
	return null;
}

function connectedHosts(request: RuntimePageRequest | null) {
	if (!request || typeof document === "undefined") return [];
	return Array.from(
		document.querySelectorAll<HTMLElement>(
			`[data-flowpilot-intake-runtime="${request.id}"]`,
		),
	).filter((element) => element.isConnected);
}

function publish(request: RuntimePageRequest | null) {
	flushSync(() => {
		requestedPage = request;
		for (const listener of listeners) listener();
	});
	notifyCommitListeners();
}

function waitForCommittedHost(request: RuntimePageRequest): Promise<void> {
	return new Promise((resolve, reject) => {
		const finish = (error?: Error) => {
			clearTimeout(timeout);
			commitListeners.delete(check);
			if (error) reject(error);
			else resolve();
		};
		const check = () => {
			if (requestedPage !== request) {
				finish(new Error("The intake runtime mount request was superseded."));
			} else if (connectedHosts(request).length === 1) {
				finish();
			}
		};
		const timeout = setTimeout(
			() =>
				finish(
					new Error(
						`The intake runtime host did not commit: ${JSON.stringify(describeIntakeRuntimeMount())}`,
					),
				),
			5_000,
		);
		commitListeners.add(check);
		check();
	});
}

export async function mountIntakeRuntimePage(
	appId: string,
	bootstrap: IPageBootstrap,
): Promise<void> {
	const page = bootstrap.page;
	if (!page) throw new Error("Intake bootstrap has no page.");
	const request: RuntimePageRequest = {
		id: ++nextRequestId,
		props: {
			appId,
			event: bootstrap.event,
			page,
			pageRevision: bootstrap.revision ?? undefined,
			pageExecutionRevision: bootstrap.executionRevision ?? undefined,
			pageElementDemand: bootstrap.elementDemand ?? undefined,
			route: bootstrap.canonicalRoute ?? "/intake",
			queryParams: {},
			active: true,
		},
	};
	publish(request);
	await waitForCommittedHost(request);
}

export async function unmountIntakeRuntimePage(): Promise<void> {
	publish(null);
}

export function describeIntakeRuntimeMount(): Record<string, unknown> {
	const hosts = connectedHosts(requestedPage);
	const host = hosts.length === 1 ? hosts[0] : undefined;
	return {
		requestedAppId: requestedPage?.props.appId,
		requestedPageId: requestedPage?.props.page.id,
		hostConnected: Boolean(host),
		connectedHostCount: hosts.length,
		renderedText: host?.innerText?.slice(0, 1000) ?? "",
		renderedElementCount:
			host?.querySelectorAll("[data-a2ui-element-ref]").length ?? 0,
	};
}

export function IntakeRuntimePageHost() {
	const request = useSyncExternalStore(
		subscribe,
		getSnapshot,
		getServerSnapshot,
	);
	useLayoutEffect(() => {
		if (request) notifyCommitListeners();
	}, [request]);
	if (!request) return null;
	return (
		<div
			className="h-[480px] w-full shrink-0 overflow-auto rounded-lg border"
			data-testid="intake-runtime-surface"
			data-flowpilot-intake-runtime={request.id}
		>
			<PageInterface key={request.id} {...request.props} />
		</div>
	);
}
