"use client";

import type {
	EventPayload,
	QueryResultPayload,
	ResizePayload,
	ThemeState,
	ValueChangedPayload,
} from "@flow-like/widget-sdk";
import { validateSchema } from "@flow-like/widget-sdk";
import i18next from "i18next";
import { TriangleAlert } from "lucide-react";
import { useTheme } from "next-themes";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useInvoke } from "../../../hooks/use-invoke";
import { getApiUrl } from "../../../lib/api-url";
import { isTauri } from "../../../lib/platform";
import { cn } from "../../../lib/utils";
import { useBackend } from "../../../state/backend-state";
import { Card, CardContent } from "../../ui/card";
import { Skeleton } from "../../ui/skeleton";
import { useExecuteAction, useSetElementValue } from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import { resolveEventActions } from "../event-handlers";
import {
	type FlwEnvelope,
	MICRO_WIDGET_DEFAULT_HEIGHT,
	MICRO_WIDGET_READY_TIMEOUT_MS,
	type QueryCorrelator,
	TokenBucket,
	acceptHostEnvelope,
	buildDesktopMicroWidgetSrc,
	buildWebMicroWidgetPath,
	clampWidgetHeight,
	createEnvelope,
	createQueryCorrelator,
	diffMicroWidgetProps,
	generateNonce,
	microWidgetValuesKey,
	readThemeTokens,
	registerMicroWidgetBridge,
	shouldUseHttpSchemeBridge,
} from "../micro-widget-host";
import type {
	Action,
	ActionBinding,
	MicroWidgetInstanceComponent,
	Style,
} from "../types";
import { WidgetInstanceProvider } from "./A2UIWidgetInstance";

type Phase = "loading" | "ready" | "error";

export type MicroWidgetEventRoute =
	| { kind: "actions"; actions: Action[] }
	| { kind: "widget_event" };

/**
 * Named handlers override widget bindings, including an explicit empty list.
 * Existing widget bindings keep their historical dispatch path; only when no
 * binding exists may the formerly executable `actions[0]` act as a fallback.
 */
export function resolveMicroWidgetEventRoute(
	component: MicroWidgetInstanceComponent,
	eventName: string,
): MicroWidgetEventRoute {
	const named = resolveEventActions(
		component.eventHandlers,
		eventName,
		undefined,
	);
	if (named.source !== "none") {
		return { kind: "actions", actions: named.actions };
	}

	if (
		component.actionBindings &&
		Object.prototype.hasOwnProperty.call(component.actionBindings, eventName)
	) {
		return { kind: "widget_event" };
	}

	if (component.actions?.[0]) {
		return { kind: "actions", actions: [component.actions[0]] };
	}

	// Preserve the old no-binding path so diagnostics/toasts remain unchanged.
	return { kind: "widget_event" };
}

function resolveLocale(): string {
	if (i18next.language) return i18next.language;
	if (typeof navigator !== "undefined" && navigator.language) {
		return navigator.language;
	}
	return "en";
}

function MicroWidgetErrorCard({
	widgetId,
	message,
}: {
	widgetId: string;
	message: string;
}) {
	return (
		<Card className="border-destructive/40 bg-destructive/5">
			<CardContent className="flex items-start gap-2 p-4 text-sm">
				<TriangleAlert className="mt-0.5 h-4 w-4 shrink-0 text-destructive" />
				<div className="min-w-0">
					<p className="font-medium text-destructive">
						Widget &quot;{widgetId}&quot; failed to load
					</p>
					<p className="text-muted-foreground break-words">{message}</p>
				</div>
			</CardContent>
		</Card>
	);
}

interface MicroWidgetFrameProps {
	component: MicroWidgetInstanceComponent;
	componentId: string;
	style?: Style;
}

function MicroWidgetFrame({
	component,
	componentId,
	style,
}: MicroWidgetFrameProps) {
	const {
		instanceId,
		packageId,
		widgetId,
		packageVersion,
		bundleHash,
		contract,
		preview,
	} = component;
	const sizing = contract?.sizing;

	const backend = useBackend();
	const { resolvedTheme } = useTheme();
	const setElementValue = useSetElementValue();
	const { executeAction } = useExecuteAction();

	const desktop = isTauri();
	const profile = useInvoke(
		backend.userState.getProfile,
		backend.userState,
		[],
		!desktop,
	);

	const iframeRef = useRef<HTMLIFrameElement>(null);
	const [nonce] = useState(generateNonce);
	const [phase, setPhase] = useState<Phase>("loading");
	const [errorMessage, setErrorMessage] = useState<string>("");
	const [height, setHeight] = useState<number>(
		() => sizing?.defaultHeight ?? MICRO_WIDGET_DEFAULT_HEIGHT,
	);

	const propsRef = useRef<Record<string, unknown>>(component.props ?? {});
	const lastSentPropsRef = useRef<Record<string, unknown>>({});
	const initSentRef = useRef(false);
	const readyRef = useRef(false);
	const correlatorRef = useRef<QueryCorrelator | null>(null);
	const buckets = useMemo(
		() => ({ event: new TokenBucket(), resize: new TokenBucket() }),
		[],
	);

	const src = useMemo(() => {
		if (desktop) {
			if (!bundleHash) return null;
			return buildDesktopMicroWidgetSrc({
				packageId,
				bundleHash,
				widgetId,
				useHttpBridge: shouldUseHttpSchemeBridge(
					typeof navigator !== "undefined" ? navigator.userAgent : "",
				),
			});
		}
		if (profile.isLoading) return null;
		return getApiUrl(
			profile.data ?? null,
			buildWebMicroWidgetPath(packageId, packageVersion, widgetId),
		);
	}, [
		desktop,
		packageId,
		packageVersion,
		widgetId,
		bundleHash,
		profile.isLoading,
		profile.data,
	]);

	const post = useCallback((envelope: FlwEnvelope) => {
		iframeRef.current?.contentWindow?.postMessage(envelope, "*");
	}, []);

	const buildThemeState = useCallback((): ThemeState => {
		const mode = resolvedTheme === "dark" ? "dark" : "light";
		if (typeof document === "undefined") return { mode, tokens: {} };
		const styles = getComputedStyle(document.documentElement);
		return {
			mode,
			tokens: readThemeTokens((name) => styles.getPropertyValue(name)),
		};
	}, [resolvedTheme]);

	const sendInit = useCallback(() => {
		lastSentPropsRef.current = propsRef.current;
		initSentRef.current = true;
		post(
			createEnvelope(
				"init",
				{
					props: propsRef.current,
					theme: buildThemeState(),
					locale: resolveLocale(),
					instanceId,
					capabilities: { preview: preview === true },
				},
				nonce,
				instanceId,
			),
		);
	}, [post, buildThemeState, instanceId, nonce, preview]);

	const handleContractEvent = useCallback(
		async (payload: EventPayload) => {
			if (preview === true) return;
			if (!buckets.event.tryTake()) {
				console.warn(
					`[MicroWidget] rate limit hit, dropped event "${payload.name}" from "${instanceId}"`,
				);
				return;
			}
			const spec = contract?.events?.[payload.name];
			if (!spec) {
				console.warn(
					`[MicroWidget] dropped event "${payload.name}" from "${instanceId}": not declared in the contract`,
				);
				return;
			}
			const validation = validateSchema(
				spec.payloadSchema ?? null,
				payload.payload,
			);
			if (!validation.valid) {
				console.warn(
					`[MicroWidget] dropped event "${payload.name}" from "${instanceId}" with invalid payload: ${validation.errors.join("; ")}`,
				);
				return;
			}
			const eventPayload = payload.payload;
			const baseContext =
				eventPayload &&
				typeof eventPayload === "object" &&
				!Array.isArray(eventPayload)
					? (eventPayload as Record<string, unknown>)
					: {};
			const actionContext = {
				...baseContext,
				actionId: payload.name,
				payload: eventPayload,
			};
			const route = resolveMicroWidgetEventRoute(component, payload.name);
			if (route.kind === "actions") {
				for (const action of route.actions) {
					await executeAction(action, componentId, actionContext);
				}
				return;
			}

			await executeAction(
				{ name: "widget_event", context: actionContext },
				componentId,
			);
		},
		[
			preview,
			buckets,
			contract,
			instanceId,
			executeAction,
			componentId,
			component,
		],
	);

	const handleEnvelope = useCallback(
		(envelope: FlwEnvelope) => {
			switch (envelope.type) {
				case "hello":
					sendInit();
					break;
				case "ready":
					readyRef.current = true;
					setPhase((prev) => (prev === "error" ? prev : "ready"));
					break;
				case "resize": {
					if (sizing?.resizable !== true) break;
					if (!buckets.resize.tryTake()) break;
					const { height: requested } = envelope.payload as ResizePayload;
					setHeight(clampWidgetHeight(requested, sizing));
					break;
				}
				case "event":
					void handleContractEvent(envelope.payload as EventPayload);
					break;
				case "value:changed": {
					const { values } = envelope.payload as ValueChangedPayload;
					setElementValue?.(microWidgetValuesKey(instanceId), values);
					break;
				}
				case "query:result":
					correlatorRef.current?.handleResult(
						envelope.payload as QueryResultPayload,
					);
					break;
				default:
					break;
			}
		},
		[
			sendInit,
			sizing,
			buckets,
			handleContractEvent,
			setElementValue,
			instanceId,
		],
	);

	const handleEnvelopeRef = useRef(handleEnvelope);
	handleEnvelopeRef.current = handleEnvelope;

	useEffect(() => {
		const listener = (event: MessageEvent) => {
			const frame = iframeRef.current;
			if (!frame?.contentWindow || event.source !== frame.contentWindow) return;
			const envelope = acceptHostEnvelope(event.data, instanceId, nonce);
			if (!envelope) return;
			handleEnvelopeRef.current(envelope);
		};
		window.addEventListener("message", listener);
		return () => window.removeEventListener("message", listener);
	}, [instanceId, nonce]);

	// Ready timeout: once the document URL is known, the widget must complete
	// the flw/1 handshake within the window or the surface shows an error card.
	useEffect(() => {
		if (!src) return;
		const timer = setTimeout(() => {
			if (readyRef.current) return;
			setErrorMessage(
				`The widget did not become ready within ${Math.round(MICRO_WIDGET_READY_TIMEOUT_MS / 1000)}s.`,
			);
			setPhase("error");
		}, MICRO_WIDGET_READY_TIMEOUT_MS);
		return () => clearTimeout(timer);
	}, [src]);

	// Query bridge registration (imperative host access via microWidgetQuery).
	useEffect(() => {
		const correlator = createQueryCorrelator((payload) => {
			post(createEnvelope("query", payload, nonce, instanceId));
		});
		correlatorRef.current = correlator;
		const unregister = registerMicroWidgetBridge(instanceId, {
			query: correlator.request,
		});
		return () => {
			unregister();
			correlator.dispose();
			correlatorRef.current = null;
		};
	}, [instanceId, nonce, post]);

	// Props updates: element updates merge into component.props upstream; diff
	// against the last sent snapshot and forward only changed keys. Keyed on the
	// serialized props because a2ui resolve() re-parses literalJson to fresh
	// objects every render.
	const propsKey = useMemo(
		() => JSON.stringify(component.props ?? {}),
		[component.props],
	);
	// biome-ignore lint/correctness/useExhaustiveDependencies: keyed on serialized props content, not object identity
	useEffect(() => {
		const nextProps = component.props ?? {};
		propsRef.current = nextProps;
		if (!initSentRef.current) return;
		const patch = diffMicroWidgetProps(lastSentPropsRef.current, nextProps);
		if (!patch) return;
		lastSentPropsRef.current = nextProps;
		post(createEnvelope("props:update", { props: patch }, nonce, instanceId));
	}, [propsKey]);

	// Theme propagation on resolvedTheme change (buildThemeState identity).
	useEffect(() => {
		if (!initSentRef.current) return;
		post(createEnvelope("theme:change", buildThemeState(), nonce, instanceId));
	}, [buildThemeState, post, nonce, instanceId]);

	const onIframeError = useCallback(() => {
		setErrorMessage("The widget document failed to load.");
		setPhase("error");
	}, []);

	if (desktop && !bundleHash) {
		return (
			<MicroWidgetErrorCard
				widgetId={widgetId}
				message="The widget bundle hash is missing, so the local bundle cannot be resolved."
			/>
		);
	}

	if (phase === "error") {
		return <MicroWidgetErrorCard widgetId={widgetId} message={errorMessage} />;
	}

	return (
		<div
			className={cn("relative w-full overflow-hidden", resolveStyle(style))}
			style={{ ...resolveInlineStyle(style), height }}
			data-widget-instance={instanceId}
			data-widget-id={widgetId}
		>
			{phase !== "ready" && <Skeleton className="absolute inset-0" />}
			{src ? (
				<iframe
					ref={iframeRef}
					src={src}
					title={`Widget ${widgetId}`}
					sandbox="allow-scripts"
					referrerPolicy="no-referrer"
					onLoad={sendInit}
					onError={onIframeError}
					className={cn(
						"h-full w-full border-0",
						phase !== "ready" && "opacity-0",
					)}
				/>
			) : null}
		</div>
	);
}

/**
 * Renders a package-shipped micro widget in a sandboxed opaque-origin iframe
 * (`sandbox="allow-scripts"`, never `allow-same-origin`) and speaks the flw/1
 * host protocol: init/props:update/theme:change/query out, hello/ready/event/
 * query:result/resize/value:changed in. Contract events prefer named handlers,
 * then retain the legacy `widget_event`/component-action fallbacks. Values
 * mirror into the elements payload as `"{instanceId}/values"`.
 */
export function A2UIMicroWidget({
	component,
	componentId,
	style,
}: ComponentProps) {
	const microComponent = component as unknown as MicroWidgetInstanceComponent;
	return (
		<WidgetInstanceProvider
			instanceId={microComponent.instanceId}
			widgetId={microComponent.widgetId}
			actionBindings={
				(microComponent.actionBindings ?? {}) as Record<string, ActionBinding>
			}
		>
			<MicroWidgetFrame
				component={microComponent}
				componentId={componentId}
				style={style ?? microComponent.style}
			/>
		</WidgetInstanceProvider>
	);
}
