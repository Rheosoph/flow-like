"use client";

import type { WidgetMediaState } from "@flow-like/widget-sdk";
import {
	createMicroWidgetMedia,
	publicWidgetProps,
	readPublicMediaGrants,
} from "../micro-widget-media";

import { i18n as i18next, useTranslation } from "@flow-like/locales";
import type {
	EventPayload,
	InitCapabilities,
	QueryResultPayload,
	ResizePayload,
	ThemeState,
	ValueChangedPayload,
} from "@flow-like/widget-sdk";
import { validateSchema } from "@flow-like/widget-sdk";
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
import {
	useActionContext,
	useExecuteAction,
	useSetElementValue,
} from "../ActionHandler";
import type { ComponentProps } from "../ComponentRegistry";
import { resolveInlineStyle, resolveStyle } from "../StyleResolver";
import { resolveEventActions } from "../event-handlers";
import {
	MicroWidgetBlockedCard,
	MicroWidgetQueuedCard,
	MicroWidgetReloadAction,
	MicroWidgetUnsupportedCard,
} from "../micro-widget-capability-dialog";
import { MicroWidgetConsentDialog } from "../micro-widget-consent-dialog";
import {
	microWidgetConsentQueueKey,
	useMicroWidgetConsentQueue,
} from "../micro-widget-consent-queue";
import {
	type WidgetConsentSummary,
	summarizeWidgetConsent,
} from "../micro-widget-consent-view";
import {
	type FlwEnvelope,
	MICRO_WIDGET_DEFAULT_HEIGHT,
	MICRO_WIDGET_EVENT_BURST,
	MICRO_WIDGET_EVENT_RATE_PER_SECOND,
	MICRO_WIDGET_READY_TIMEOUT_MS,
	type QueryCorrelator,
	TokenBucket,
	acceptHostEnvelope,
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
import { createMicroWidgetMicrophone } from "../micro-widget-microphone";
import { policyHostCount } from "../micro-widget-policy";
import { useMicroWidgetReloader } from "../micro-widget-reload";
import { MicroWidgetNoticeSlot } from "../micro-widget-runtime-banner";
import type {
	Action,
	ActionBinding,
	MicroWidgetInstanceComponent,
	Style,
} from "../types";
import {
	type MicroWidgetGrantState,
	microWidgetFrameSrc,
	useMicroWidgetGrant,
} from "../use-micro-widget-grant";
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

type MicroWidgetContractEvent = NonNullable<
	NonNullable<MicroWidgetInstanceComponent["contract"]>["events"]
>[string];

export type MicroWidgetContractEventResolution =
	| { kind: "dropped"; reason: "undeclared" }
	| { kind: "dropped"; reason: "invalid_payload"; errors: string[] }
	| {
			kind: "dispatch";
			route: MicroWidgetEventRoute;
			context: Record<string, unknown>;
	  };

function isRecord(value: unknown): value is Record<string, unknown> {
	return typeof value === "object" && value !== null && !Array.isArray(value);
}

/**
 * Widgets may only start work through events their contract declares as own
 * keys. A plain property read would accept inherited members such as
 * `constructor` or `toString`, skip payload validation (no schema) and let the
 * event reach a page `*` handler.
 */
export function readDeclaredMicroWidgetEvent(
	contract: MicroWidgetInstanceComponent["contract"],
	eventName: string,
): MicroWidgetContractEvent | null {
	const events = contract?.events;
	if (
		typeof eventName !== "string" ||
		!isRecord(events) ||
		!Object.prototype.hasOwnProperty.call(events, eventName)
	) {
		return null;
	}
	const spec = events[eventName];
	return isRecord(spec) ? spec : null;
}

/**
 * Gate an iframe event on the contract, validate its payload, and resolve the
 * page-authored route it may start. Anything not declared is dropped before
 * named, wildcard, binding or legacy routing is consulted.
 */
export function resolveMicroWidgetContractEvent(
	component: MicroWidgetInstanceComponent,
	event: EventPayload,
): MicroWidgetContractEventResolution {
	const spec = readDeclaredMicroWidgetEvent(component.contract, event.name);
	if (!spec) return { kind: "dropped", reason: "undeclared" };
	const schema = Object.prototype.hasOwnProperty.call(spec, "payloadSchema")
		? (spec.payloadSchema ?? null)
		: null;
	const validation = validateSchema(schema, event.payload);
	if (!validation.valid) {
		return {
			kind: "dropped",
			reason: "invalid_payload",
			errors: validation.errors,
		};
	}
	const payload = event.payload;
	return {
		kind: "dispatch",
		route: resolveMicroWidgetEventRoute(component, event.name),
		context: {
			...(isRecord(payload) ? payload : {}),
			actionId: event.name,
			payload,
		},
	};
}

type ExecuteAction = ReturnType<typeof useExecuteAction>["executeAction"];

/**
 * Start the page-authored route of a dispatched contract event. The payload is
 * data: every action it starts runs with the micro widget origin, so the
 * payload cannot change the route, URL, app, event or feedback it targets.
 */
export async function dispatchMicroWidgetContractEvent(
	resolution: Extract<MicroWidgetContractEventResolution, { kind: "dispatch" }>,
	componentId: string,
	executeAction: ExecuteAction,
): Promise<void> {
	const { route, context } = resolution;
	if (route.kind === "actions") {
		for (const action of route.actions) {
			await executeAction(action, componentId, context, {
				origin: "micro_widget",
			});
		}
		return;
	}

	await executeAction(
		{ name: "widget_event", context },
		componentId,
		undefined,
		{ origin: "micro_widget" },
	);
}

function resolveLocale(): string {
	if (i18next.language) return i18next.language;
	if (typeof navigator !== "undefined" && navigator.language) {
		return navigator.language;
	}
	return "en";
}

type Translate = ReturnType<typeof useTranslation>["t"];

function grantErrorMessage(
	state: Extract<MicroWidgetGrantState, { status: "error" }>,
	t: Translate,
): string {
	return state.reason === "policy_unstable"
		? t(
				"widgetPolicyChangedRepeatedly",
				"The widget's permissions changed while they were being granted. Reload the page to try again.",
			)
		: t(
				"widgetGrantFailed",
				"The widget's permissions could not be granted: {{detail}}",
				{ detail: state.detail },
			);
}

function mediaStateLabel(media: WidgetMediaState, t: Translate): string {
	switch (media.state) {
		case "error":
			return media.error ?? t("widgetAudioError", "Playback failed");
		case "loading":
			return t("loading", "Loading…");
		case "playing":
			return t("widgetAudioPlaying", "Playing");
		case "paused":
			return t("widgetAudioPaused", "Paused");
		case "idle":
			return "";
	}
}

function MicroWidgetErrorCard({
	elementRef,
	widgetId,
	message,
	onReload,
}: {
	widgetId: string;
	message: string;
	elementRef?: ComponentProps["elementRef"];
	/** Offered by editors when a different build of the widget is installed. */
	onReload?: () => void;
}) {
	const { t } = useTranslation("common");
	return (
		<Card ref={elementRef} className="border-destructive/40 bg-destructive/5">
			<CardContent className="flex items-start gap-2 p-4 text-sm">
				<TriangleAlert className="mt-0.5 h-4 w-4 shrink-0 text-destructive" />
				<div className="flex min-w-0 flex-col gap-1">
					<p className="font-medium text-destructive">
						{t(
							"widgetQuotwidgetidquotFailedToLoad",
							'Widget "{{widgetId}}" failed to load',
							{ widgetId },
						)}
					</p>
					<p className="text-muted-foreground break-words">{message}</p>
					{onReload && <MicroWidgetReloadAction onReload={onReload} />}
				</div>
			</CardContent>
		</Card>
	);
}

interface MicroWidgetFrameProps {
	elementRef?: ComponentProps["elementRef"];
	component: MicroWidgetInstanceComponent;
	componentId: string;
	style?: Style;
}

function MicroWidgetFrame({
	elementRef,
	component,
	componentId,
	style,
}: MicroWidgetFrameProps) {
	const { t } = useTranslation("common");
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
	const { appId } = useActionContext();

	const desktop = isTauri();
	// Consent, capabilities and hosts come from the backend's descriptor of the
	// installed bundle or published version; the page contract is only read to
	// pick the fallback for backends that cannot describe widgets.
	const grant = useMicroWidgetGrant({
		packageId,
		packageVersion,
		bundleHash,
		widgetId,
		preview: preview === true,
		appId,
		contract,
		props: component.props,
		enabled: !desktop || Boolean(bundleHash),
	});
	const grantState = grant.state;
	const frame = grantState.status === "ready" ? grantState.frame : null;
	const framePolicy = frame?.policy;
	const prompt = grant.prompt;
	const queue = useMicroWidgetConsentQueue(
		useMemo(
			() => (prompt ? microWidgetConsentQueueKey(prompt, appId) : null),
			[prompt, appId],
		),
	);
	const { jumpToFront } = queue;
	const { review: reviewGrant } = grant;
	const review = useCallback(() => {
		jumpToFront();
		reviewGrant();
	}, [jumpToFront, reviewGrant]);
	// The blocked card describes the prompt the viewer refused; the grant state only keeps its subject.
	const refusedSummary = useRef<WidgetConsentSummary | null>(null);
	useEffect(() => {
		if (prompt) refusedSummary.current = summarizeWidgetConsent(prompt);
	}, [prompt]);
	const containerRef = useRef<HTMLDivElement | null>(null);
	const reviewButtonRef = useRef<HTMLButtonElement>(null);
	const setContainer = useCallback(
		(element: HTMLDivElement | null) => {
			containerRef.current = element;
			elementRef?.(element);
		},
		[elementRef],
	);
	const restoreFocus = useCallback(() => {
		(reviewButtonRef.current ?? containerRef.current)?.focus();
	}, []);

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
	const [recording, setRecording] = useState(false);
	const [mediaState, setMediaState] = useState<WidgetMediaState>({
		state: "idle",
	});
	const mediaRef = useRef<ReturnType<typeof createMicroWidgetMedia> | null>(
		null,
	);
	const mediaGranted = framePolicy?.media === true;
	const mediaGrants = mediaGranted
		? readPublicMediaGrants(component.props?.publicMediaGrants)
		: [];
	const mediaGrantsRef = useRef(mediaGrants);
	mediaGrantsRef.current = mediaGrants;
	const microphoneRef = useRef<ReturnType<
		typeof createMicroWidgetMicrophone
	> | null>(null);
	const hostCapabilities: InitCapabilities = {
		preview: preview === true,
		media: mediaGranted,
		mediaIds: mediaGrants.map((mediaGrant) => mediaGrant.id),
		microphone: framePolicy?.microphone === true,
	};
	const capabilitiesRef = useRef(hostCapabilities);
	capabilitiesRef.current = hostCapabilities;
	const lastSentPropsRef = useRef<Record<string, unknown>>({});
	const initSentRef = useRef(false);
	const readyRef = useRef(false);
	const correlatorRef = useRef<QueryCorrelator | null>(null);
	const buckets = useMemo(
		() => ({
			event: new TokenBucket(
				MICRO_WIDGET_EVENT_BURST,
				MICRO_WIDGET_EVENT_RATE_PER_SECOND,
			),
			resize: new TokenBucket(),
		}),
		[],
	);

	// No document is fetched until the grant flow settles; the ready timer waits with it.
	const frameSource = useMemo((): { src: string | null; error?: string } => {
		if (!frame) return { src: null };
		try {
			return {
				src: microWidgetFrameSrc(frame, {
					desktop,
					useHttpBridge: shouldUseHttpSchemeBridge(
						typeof navigator !== "undefined" ? navigator.userAgent : "",
					),
					apiUrl: profile.isLoading
						? null
						: (path) => getApiUrl(profile.data ?? null, path),
				}),
			};
		} catch (error) {
			return {
				src: null,
				error: error instanceof Error ? error.message : String(error),
			};
		}
	}, [frame, desktop, profile.isLoading, profile.data]);
	const src = frameSource.src;

	// The frame is the host-authored wrapper; it relays envelopes to the widget.
	const post = useCallback(
		(envelope: FlwEnvelope, transfer: Transferable[] = []) => {
			iframeRef.current?.contentWindow?.postMessage(envelope, "*", transfer);
		},
		[],
	);

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
					props: publicWidgetProps(propsRef.current),
					theme: buildThemeState(),
					locale: resolveLocale(),
					instanceId,
					capabilities: capabilitiesRef.current,
				},
				nonce,
				instanceId,
			),
		);
	}, [post, buildThemeState, instanceId, nonce, preview]);

	/**
	 * A host move — the inline page runtime relocating its portal host between a card slot and
	 * its parking slot — disconnects this iframe, and a disconnected iframe has its browsing
	 * context discarded and reloaded. React state survives that, so without resetting here the
	 * skeleton stays hidden over a blank frame, the ready timeout never re-arms, and every
	 * query issued before the move hangs until it times out.
	 */
	const onGrantFrameLoad = grant.onFrameLoad;
	const onGrantFrameHello = grant.onFrameHello;
	const handleFrameLoad = useCallback(() => {
		onGrantFrameLoad();
		if (initSentRef.current) {
			readyRef.current = false;
			setPhase("loading");
			correlatorRef.current?.dispose();
			microphoneRef.current?.revoke();
			mediaRef.current?.stop();
		}
		sendInit();
	}, [sendInit, onGrantFrameLoad]);

	const handleContractEvent = useCallback(
		async (payload: EventPayload) => {
			if (preview === true) return;
			if (!buckets.event.tryTake()) {
				console.warn(
					`[MicroWidget] rate limit hit, dropped event "${payload.name}" from "${instanceId}"`,
				);
				return;
			}
			const resolution = resolveMicroWidgetContractEvent(component, payload);
			if (resolution.kind === "dropped") {
				console.warn(
					resolution.reason === "undeclared"
						? `[MicroWidget] dropped event "${payload.name}" from "${instanceId}": not declared in the contract`
						: `[MicroWidget] dropped event "${payload.name}" from "${instanceId}" with invalid payload: ${resolution.errors.join("; ")}`,
				);
				return;
			}
			await dispatchMicroWidgetContractEvent(
				resolution,
				componentId,
				executeAction,
			);
		},
		[preview, buckets, instanceId, executeAction, componentId, component],
	);

	const handleEnvelope = useCallback(
		(envelope: FlwEnvelope) => {
			microphoneRef.current?.handle(envelope);
			mediaRef.current?.handle(envelope);
			switch (envelope.type) {
				case "hello":
					onGrantFrameHello();
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
			onGrantFrameHello,
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
	// Keyed on the phase as well as the URL so a reloaded sandbox is held to the
	// same deadline as the first load.
	useEffect(() => {
		if (!src || phase !== "loading") return;
		const timer = setTimeout(() => {
			if (readyRef.current) return;
			setErrorMessage(
				t(
					"theWidgetDidNotBecomeReadyWithinVals",
					"The widget did not become ready within {{val}}s.",
					{ val: Math.round(MICRO_WIDGET_READY_TIMEOUT_MS / 1000) },
				),
			);
			setPhase("error");
		}, MICRO_WIDGET_READY_TIMEOUT_MS);
		return () => clearTimeout(timer);
	}, [src, phase, t]);

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

	useEffect(() => {
		const microphone = createMicroWidgetMicrophone({
			enabled: () => capabilitiesRef.current.microphone === true,
			beforeCapture: () => mediaRef.current?.pause(),
			result: (payload, transfer) =>
				post(
					createEnvelope("microphone:result", payload, nonce, instanceId),
					transfer,
				),
			recording: setRecording,
		});
		const media = createMicroWidgetMedia({
			grants: () => mediaGrantsRef.current,
			state: (state) => {
				setMediaState(state);
				post(createEnvelope("media:state", state, nonce, instanceId));
			},
			result: (result) =>
				post(createEnvelope("media:result", result, nonce, instanceId)),
		});
		mediaRef.current = media;
		microphoneRef.current = microphone;
		return () => {
			microphone.dispose();
			media.dispose();
			mediaRef.current = null;
			microphoneRef.current = null;
		};
	}, [instanceId, nonce, post, src]);

	const mediaGrantsKey = JSON.stringify(mediaGrants);
	const capabilitiesKey = JSON.stringify(hostCapabilities);
	// biome-ignore lint/correctness/useExhaustiveDependencies: serialized capability changes revoke outstanding work
	useEffect(() => {
		if (!capabilitiesRef.current.microphone) microphoneRef.current?.revoke();
		mediaRef.current?.revoke();
		if (initSentRef.current)
			post(
				createEnvelope(
					"capabilities:update",
					capabilitiesRef.current,
					nonce,
					instanceId,
				),
			);
	}, [mediaGrantsKey, capabilitiesKey, instanceId, nonce, post]);

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
		const patch = diffMicroWidgetProps(
			publicWidgetProps(lastSentPropsRef.current),
			publicWidgetProps(nextProps),
		);
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
		setErrorMessage(
			t("widgetDocumentFailedToLoad", "The widget document failed to load."),
		);
		setPhase("error");
	}, [t]);

	// A rebuilt local package prunes the placed bundle, so a failure is the cue to look for the new one.
	const reloader = useMicroWidgetReloader();
	const refreshInstalled = reloader?.refresh;
	const failed =
		(desktop && !bundleHash) ||
		grantState.status === "error" ||
		grantState.status === "unsupported" ||
		frameSource.error !== undefined ||
		phase === "error";
	useEffect(() => {
		if (failed) refreshInstalled?.();
	}, [failed, refreshInstalled]);
	const onReload =
		reloader?.updateFor(component) != null
			? () => void reloader.reload(componentId)
			: undefined;

	if (desktop && !bundleHash) {
		return (
			<MicroWidgetErrorCard
				elementRef={elementRef}
				widgetId={widgetId}
				message={t(
					"widgetBundleHashMissing",
					"The widget bundle hash is missing, so the local bundle cannot be resolved.",
				)}
				onReload={onReload}
			/>
		);
	}

	if (grantState.status === "unsupported") {
		return (
			<MicroWidgetUnsupportedCard
				elementRef={elementRef}
				widgetId={widgetId}
				detail={grantState.detail}
				onReload={onReload}
			/>
		);
	}

	if (grantState.status === "blocked") {
		return (
			<MicroWidgetBlockedCard
				elementRef={elementRef}
				reviewRef={reviewButtonRef}
				widgetId={widgetId}
				summary={
					refusedSummary.current ?? {
						level: null,
						hostCount: policyHostCount(grantState.subject.policy),
						includesRuntime: false,
					}
				}
				onReview={review}
				onRunBaseline={
					grantState.canRunBaseline ? grant.runRestricted : undefined
				}
			/>
		);
	}

	const failure =
		grantState.status === "error"
			? grantErrorMessage(grantState, t)
			: (frameSource.error ?? (phase === "error" ? errorMessage : null));
	if (failure !== null) {
		return (
			<MicroWidgetErrorCard
				elementRef={elementRef}
				widgetId={widgetId}
				message={failure}
				onReload={onReload}
			/>
		);
	}

	if (grantState.status === "pending" && queue.waiting) {
		return (
			<MicroWidgetQueuedCard
				elementRef={elementRef}
				widgetId={widgetId}
				height={height}
			/>
		);
	}

	return (
		<div
			ref={setContainer}
			tabIndex={-1}
			className={cn(
				"relative w-full overflow-hidden outline-none",
				resolveStyle(style),
			)}
			style={{ ...resolveInlineStyle(style), height }}
			data-widget-instance={instanceId}
			data-widget-id={widgetId}
			data-widget-consent={grant.consent}
			data-widget-grant={grantState.status}
		>
			{prompt && queue.active && (
				<MicroWidgetConsentDialog
					prompt={prompt}
					actions={grant}
					position={queue.position}
					total={queue.total}
					onRestoreFocus={restoreFocus}
				/>
			)}
			{frame && <MicroWidgetNoticeSlot grant={grant} onReview={review} />}
			{mediaState.state !== "idle" && (
				<fieldset
					className="absolute bottom-2 right-2 z-50 m-0 flex min-w-0 items-center gap-2 rounded border-0 bg-background p-2 text-sm shadow"
					aria-label={t("widgetAudioPlayback", "Widget audio playback")}
				>
					<span>{mediaStateLabel(mediaState, t)}</span>
					<button type="button" onClick={() => void mediaRef.current?.resume()}>
						{t("widgetAudioPlay", "Play")}
					</button>
					<button type="button" onClick={() => mediaRef.current?.pause()}>
						{t("pause", "Pause")}
					</button>
					<button type="button" onClick={() => mediaRef.current?.stop()}>
						{t("widgetAudioStop", "Stop audio")}
					</button>
				</fieldset>
			)}
			{recording && (
				<button
					type="button"
					className="absolute top-2 right-2 z-50 rounded bg-destructive px-3 py-2 text-sm text-destructive-foreground"
					onClick={() => microphoneRef.current?.stop()}
				>
					{t("stopWidgetRecording", "Stop recording")}
				</button>
			)}
			{phase !== "ready" && <Skeleton className="absolute inset-0" />}
			{src ? (
				<iframe
					// A new grant is a new document URL and a new handshake.
					key={src}
					ref={iframeRef}
					src={src}
					title={t("widgetWidgetid", "Widget {{widgetId}}", { widgetId })}
					sandbox={
						framePolicy?.downloads === true
							? "allow-scripts allow-downloads"
							: "allow-scripts"
					}
					referrerPolicy="no-referrer"
					onLoad={handleFrameLoad}
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
 * mirror into the elements payload as `"{instanceId}/values"`. Network sites
 * and browser capabilities come from the backend's descriptor of the widget,
 * never from page JSON. They take effect only through a grant minted after the
 * user approves them; until then no document is loaded, and a blocked widget
 * can still run at baseline.
 */
export function A2UIMicroWidget({
	elementRef,
	component,
	componentId,
	style,
}: ComponentProps) {
	const microComponent = component as unknown as MicroWidgetInstanceComponent;
	return (
		<WidgetInstanceProvider
			instanceId={microComponent.instanceId}
			widgetId={microComponent.widgetId}
			componentId={componentId}
			actionBindings={
				(microComponent.actionBindings ?? {}) as Record<string, ActionBinding>
			}
		>
			<MicroWidgetFrame
				// A synced revision needs a new handshake and must recover from the old frame's error.
				key={JSON.stringify([
					microComponent.instanceId,
					microComponent.packageId,
					microComponent.widgetId,
					microComponent.packageVersion,
					microComponent.bundleHash,
				])}
				elementRef={elementRef}
				component={microComponent}
				componentId={componentId}
				style={style ?? microComponent.style}
			/>
		</WidgetInstanceProvider>
	);
}
