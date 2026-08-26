"use client";

import { useEffect, useRef } from "react";
import { useAgentActionAccess, useExecuteAction } from "./ActionHandler";
import { useData } from "./DataContext";
import { getComponentEventDefinitions } from "./component-event-manifest";
import { resolveEventActions } from "./event-handlers";
import {
	type LivePageRunRecord,
	type LivePageTriggerResult,
	isLivePageComponentEffectivelyHidden,
	isLivePageValueBearingComponent,
	registerLivePage,
	resolveLivePageComponentId,
	subscribeLivePageRuns,
} from "./live-page-registry";
import type { A2UIServerMessage, BoundValue, Surface } from "./types";

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
	const { resolve } = useData();

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
		resolve,
		executeAction,
		surfaceId,
	});
	latest.current = {
		getSurface,
		getContainer,
		applyServerMessage,
		getElementValues,
		setElementValue,
		resolve,
		executeAction,
		surfaceId,
	};

	useEffect(() => {
		if (!appId || !pageId) return;

		const elementKey = (componentId: string) =>
			`${latest.current.surfaceId ?? pageId}/${componentId}`;
		const resolvesTrue = (component: unknown, field: string) => {
			const value = (component as Record<string, unknown>)[field];
			return Boolean(latest.current.resolve(value as BoundValue));
		};
		const resolvedInputType = (value: BoundValue) =>
			String(latest.current.resolve(value) ?? "")
				.trim()
				.toLowerCase();
		const currentEventContext = (
			componentId: string,
			component: Surface["components"][string]["component"],
			eventName: string,
		): Record<string, unknown> => {
			if (
				eventName !== "change" &&
				eventName !== "input" &&
				eventName !== "submit"
			) {
				return {};
			}
			if (!isLivePageValueBearingComponent(component)) return {};

			const values = latest.current.getElementValues?.() ?? {};
			const key = elementKey(componentId);
			const stored = Object.prototype.hasOwnProperty.call(values, key)
				? values[key]
				: undefined;
			const fields = component as typeof component & Record<string, unknown>;
			if (component.type === "checkbox" || component.type === "switch") {
				const checked =
					stored ?? latest.current.resolve(fields.checked as BoundValue);
				return checked === undefined ? {} : { checked };
			}
			const value =
				stored ?? latest.current.resolve(fields.value as BoundValue);
			return value === undefined ? {} : { value };
		};

		const unregister = registerLivePage({
			appId,
			pageId,
			get eventId() {
				return eventIdRef.current;
			},
			getSurface: () => latest.current.getSurface(),
			getContainer: () => latest.current.getContainer?.() ?? null,
			getElementValues: () => latest.current.getElementValues?.() ?? {},
			resolveBoundValue: (value) => latest.current.resolve(value as BoundValue),
			setElementValue: (requestedId, value) => {
				const surface = latest.current.getSurface();
				if (!surface) {
					throw new Error(`Page '${pageId}' has no rendered surface.`);
				}
				const componentId = resolveLivePageComponentId(
					pageId,
					surface,
					requestedId,
				);
				const component = surface.components?.[componentId]?.component;
				if (!component) {
					throw new Error(
						`Component '${componentId}' does not exist on page '${pageId}'.`,
					);
				}
				if (
					isLivePageComponentEffectivelyHidden(surface, componentId, (value) =>
						latest.current.resolve(value as BoundValue),
					)
				) {
					throw new Error(
						`Component '${componentId}' is hidden by itself or an ancestor.`,
					);
				}
				if (!isLivePageValueBearingComponent(component)) {
					throw new Error(
						`Component '${componentId}' (${component.type}) does not accept set_value.`,
					);
				}
				if (resolvesTrue(component, "disabled")) {
					throw new Error(`Component '${componentId}' is disabled.`);
				}
				if (resolvesTrue(component, "hidden")) {
					throw new Error(`Component '${componentId}' is hidden.`);
				}
				if (
					component.type === "richText" &&
					resolvesTrue(component, "readOnly")
				) {
					throw new Error(`Component '${componentId}' is read-only.`);
				}
				if (
					(component.type === "checkbox" || component.type === "switch") &&
					typeof value !== "boolean"
				) {
					throw new Error(
						`Component '${componentId}' (${component.type}) requires a boolean value.`,
					);
				}
				if (
					component.type === "slider" &&
					(typeof value !== "number" || !Number.isFinite(value))
				) {
					throw new Error(
						`Component '${componentId}' (slider) requires a finite number.`,
					);
				}
				if (
					component.type === "textField" &&
					resolvedInputType(component.inputType as BoundValue) === "number" &&
					!(
						(typeof value === "number" && Number.isFinite(value)) ||
						(typeof value === "string" &&
							value.trim() !== "" &&
							Number.isFinite(Number(value)))
					)
				) {
					throw new Error(
						`Component '${componentId}' (number input) requires a finite number or numeric string.`,
					);
				}
				// Payload half: what the next workflow run receives in _elements/_input_values.
				latest.current.setElementValue?.(elementKey(componentId), value);
				// Visual half: what the rendered input displays.
				latest.current.applyServerMessage({
					type: "upsertElement",
					element_id: elementKey(componentId),
					value:
						component.type === "checkbox" || component.type === "switch"
							? { type: "setChecked", checked: value }
							: { type: "setValue", value },
				} as A2UIServerMessage);
			},
			triggerComponentEvent: async (
				requestedId,
				eventName,
			): Promise<LivePageTriggerResult> => {
				const surface = latest.current.getSurface();
				if (!surface) {
					throw new Error(`Page '${pageId}' has no rendered surface.`);
				}
				const componentId = resolveLivePageComponentId(
					pageId,
					surface,
					requestedId,
				);
				const surfaceComponent = surface?.components?.[componentId];
				if (!surfaceComponent?.component) {
					throw new Error(
						`Component '${componentId}' does not exist on page '${pageId}'.`,
					);
				}
				const component = surfaceComponent.component;
				if (
					isLivePageComponentEffectivelyHidden(surface, componentId, (value) =>
						latest.current.resolve(value as BoundValue),
					)
				) {
					throw new Error(
						`Component '${componentId}' is hidden by itself or an ancestor.`,
					);
				}
				if (resolvesTrue(component, "disabled")) {
					throw new Error(`Component '${componentId}' is disabled.`);
				}
				if (resolvesTrue(component, "hidden")) {
					throw new Error(`Component '${componentId}' is hidden.`);
				}
				const definition = getComponentEventDefinitions(component).find(
					(candidate) => candidate.id === eventName,
				);
				const resolution = resolveEventActions(
					component.eventHandlers,
					eventName,
					component.actions,
					{
						// An unknown event name must never inherit a legacy click or wildcard
						// workflow. Exact custom handlers still resolve before these fallbacks.
						legacyFallback: definition?.legacyFallback ?? false,
						wildcardFallback: definition?.wildcardFallback ?? false,
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
						await latest.current.executeAction(
							action,
							componentId,
							currentEventContext(componentId, component, eventName),
						);
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
