"use client";

import { useEffect, useRef } from "react";
import { useAgentActionAccess, useExecuteAction } from "./ActionHandler";
import { getComponentEventDefinitions } from "./component-event-manifest";
import { resolveEventActions } from "./event-handlers";
import {
	type LivePageRunRecord,
	type LivePageTriggerResult,
	registerLivePage,
	subscribeLivePageRuns,
} from "./live-page-registry";
import type { A2UIServerMessage, Surface } from "./types";

interface LivePageAgentBridgeProps {
	appId: string;
	pageId: string;
	eventId?: string;
	getSurface: () => Surface | null;
	/** This instance's rendered page container ([data-page-id] node) for captures. */
	getContainer?: () => HTMLElement | null;
	/** The page's surface reducer entry point — used to mirror value writes visually. */
	applyServerMessage: (message: A2UIServerMessage) => void;
	loading: boolean;
}

/**
 * Invisible component mounted inside a page's ActionProvider (via A2UIRenderer's agentBridge
 * slot). Registers a LivePageHandle so FlowPilot's interact_app_page tool can drive this page
 * through the exact action pipeline user interactions use.
 */
export function LivePageAgentBridge({
	appId,
	pageId,
	eventId,
	getSurface,
	getContainer,
	applyServerMessage,
	loading,
}: LivePageAgentBridgeProps) {
	const { surfaceId, getElementValues, setElementValue } =
		useAgentActionAccess();
	const { executeAction } = useExecuteAction();

	const loadingRef = useRef(loading);
	loadingRef.current = loading;
	const eventIdRef = useRef(eventId);
	eventIdRef.current = eventId;

	// The handle must always act on the latest render's callbacks without re-registering.
	const latest = useRef({
		getSurface,
		getContainer,
		applyServerMessage,
		getElementValues,
		setElementValue,
		executeAction,
		surfaceId,
	});
	latest.current = {
		getSurface,
		getContainer,
		applyServerMessage,
		getElementValues,
		setElementValue,
		executeAction,
		surfaceId,
	};

	useEffect(() => {
		if (!appId || !pageId) return;

		const elementKey = (componentId: string) =>
			`${latest.current.surfaceId ?? pageId}/${componentId}`;

		const unregister = registerLivePage({
			appId,
			pageId,
			get eventId() {
				return eventIdRef.current;
			},
			getSurface: () => latest.current.getSurface(),
			getContainer: () => latest.current.getContainer?.() ?? null,
			getElementValues: () => latest.current.getElementValues?.() ?? {},
			setElementValue: (componentId, value) => {
				// Payload half: what the next workflow run receives in _elements/_input_values.
				latest.current.setElementValue?.(elementKey(componentId), value);
				// Visual half: what the rendered input displays.
				latest.current.applyServerMessage({
					type: "upsertElement",
					element_id: elementKey(componentId),
					value: { type: "setValue", value },
				} as A2UIServerMessage);
			},
			triggerComponentEvent: async (
				componentId,
				eventName,
			): Promise<LivePageTriggerResult> => {
				const surface = latest.current.getSurface();
				const surfaceComponent = surface?.components?.[componentId];
				if (!surfaceComponent?.component) {
					throw new Error(
						`Component '${componentId}' does not exist on page '${pageId}'.`,
					);
				}
				const component = surfaceComponent.component;
				const definition = getComponentEventDefinitions(component).find(
					(candidate) => candidate.id === eventName,
				);
				const resolution = resolveEventActions(
					component.eventHandlers,
					eventName,
					component.actions,
					{
						legacyFallback: definition?.legacyFallback,
						wildcardFallback: definition?.wildcardFallback,
					},
				);
				if (resolution.actions.length === 0) {
					return {
						triggered: false,
						source: resolution.source,
						actionCount: 0,
						runs: [],
					};
				}
				const runs: LivePageRunRecord[] = [];
				const unsubscribe = subscribeLivePageRuns(
					latest.current.surfaceId ?? pageId,
					(record) => {
						// The bus is surface-keyed and shared by every actor on this page (a user
						// clicking during the await, a second live instance). Collect only runs
						// started by the component THIS trigger fired.
						if (record.componentId !== componentId) return;
						runs.push(record);
					},
				);
				try {
					for (const action of resolution.actions) {
						await latest.current.executeAction(action, componentId, {});
					}
				} finally {
					unsubscribe();
				}
				return {
					triggered: true,
					source: resolution.source,
					actionCount: resolution.actions.length,
					runs,
				};
			},
			isLoading: () => loadingRef.current,
		});

		return unregister;
	}, [appId, pageId]);

	return null;
}
