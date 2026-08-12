import { type MapStore, type ReadableAtom, atom, map } from "nanostores";
import {
	CONTRACT_VERSION,
	type WidgetContract,
	contractDefaults,
} from "./contract";
import type {
	QueryArgs,
	QueryReturns,
	WidgetDefinition,
	WidgetEventsShape,
	WidgetInputsShape,
	WidgetQueriesShape,
} from "./define";
import {
	type FlwEnvelope,
	type FlwMessageType,
	type FlwPayloadMap,
	type InitPayload,
	type PropsUpdatePayload,
	type QueryPayload,
	type ThemeState,
	createEnvelope,
	isFlwEnvelope,
} from "./protocol";
import {
	detectColorScheme,
	renderStandaloneBadge,
	themeForMode,
	watchColorScheme,
} from "./standalone";
import { applyTheme } from "./theme";
import { validateInputValue, validateSchema } from "./validate";

export const INIT_TIMEOUT_MS = 300;

export type BridgeMode = "connecting" | "hosted" | "standalone";

export type EmitPayloadArgs<E, Name extends keyof E> = E[Name] extends void
	? []
	: undefined extends E[Name]
		? [payload?: E[Name]]
		: [payload: E[Name]];

export interface WidgetBridge<
	I extends WidgetInputsShape = Record<string, unknown>,
	E extends WidgetEventsShape = Record<string, unknown>,
	Q extends WidgetQueriesShape = Record<
		string,
		{ args: unknown; returns: unknown }
	>,
> {
	readonly $props: MapStore<I>;
	readonly $theme: ReadableAtom<ThemeState>;
	readonly $mode: ReadableAtom<BridgeMode>;
	emit<Name extends keyof E & string>(
		name: Name,
		...args: EmitPayloadArgs<E, Name>
	): void;
	onQuery<Name extends keyof Q & string>(
		name: Name,
		handler: (
			args: QueryArgs<Q, Name>,
		) => QueryReturns<Q, Name> | Promise<QueryReturns<Q, Name>>,
	): () => void;
	setValues(values: Record<string, unknown>): void;
	dispose(): void;
}

export interface FlwStandaloneGlobal {
	query(name: string, args?: unknown): Promise<unknown>;
	bridge: WidgetBridge;
}

type FlwGlobals = {
	__FLW_CONTRACT__?: WidgetContract;
	__flw?: FlwStandaloneGlobal;
};

export function acceptEnvelope(
	data: unknown,
	expectedNonce: string | null,
	sourceIsParent: boolean,
): FlwEnvelope | null {
	if (!sourceIsParent) return null;
	if (!isFlwEnvelope(data)) return null;
	if (expectedNonce === null) return data.type === "init" ? data : null;
	if (data.nonce !== expectedNonce) return null;
	return data;
}

export function mergeInitProps(
	contract: WidgetContract | null | undefined,
	initProps: Record<string, unknown>,
): Record<string, unknown> {
	return { ...contractDefaults(contract), ...initProps };
}

export interface RejectedPatchEntry {
	key: string;
	errors: string[];
}

export function filterPropsPatch(
	contract: WidgetContract | null | undefined,
	patch: Record<string, unknown>,
): { accepted: Record<string, unknown>; rejected: RejectedPatchEntry[] } {
	const accepted: Record<string, unknown> = {};
	const rejected: RejectedPatchEntry[] = [];
	if (!contract) {
		return { accepted: { ...patch }, rejected };
	}
	for (const [key, value] of Object.entries(patch)) {
		const input = contract.inputs?.[key];
		if (!input) {
			rejected.push({ key, errors: ["not declared in the contract"] });
			continue;
		}
		const result = validateInputValue(input, value);
		if (result.valid) {
			accepted[key] = value;
		} else {
			rejected.push({ key, errors: result.errors });
		}
	}
	return { accepted, rejected };
}

export function mountFlowWidget<
	I extends WidgetInputsShape,
	E extends WidgetEventsShape,
	Q extends WidgetQueriesShape,
>(definition: WidgetDefinition<I, E, Q>): WidgetBridge<I, E, Q> {
	if (typeof window === "undefined") {
		throw new Error(
			"mountFlowWidget requires a browser environment (window is undefined)",
		);
	}

	const flwGlobals = globalThis as FlwGlobals;
	const contract = flwGlobals.__FLW_CONTRACT__;

	const $props = map<I>(contractDefaults(contract) as I);
	const $theme = atom<ThemeState>(themeForMode("light"));
	const $mode = atom<BridgeMode>("connecting");

	let nonce: string | null = null;
	let instanceId = "";
	let preview = false;
	let disposed = false;
	const queryHandlers = new Map<string, (args: unknown) => unknown>();

	let standaloneTimer: ReturnType<typeof setTimeout> | null = null;
	let standaloneCleanup: (() => void) | null = null;
	let resizeObserver: ResizeObserver | null = null;
	let resizeRaf: number | null = null;

	const post = <T extends FlwMessageType>(
		type: T,
		payload: FlwPayloadMap[T],
	) => {
		window.parent.postMessage(
			createEnvelope(type, payload, nonce ?? "", instanceId),
			"*",
		);
	};

	const setAndApplyTheme = (theme: ThemeState) => {
		$theme.set(theme);
		applyTheme(theme);
	};

	const startAutoResize = () => {
		if (resizeObserver || typeof ResizeObserver === "undefined") return;
		const resizable =
			contract?.sizing?.resizable ?? definition.sizing?.resizable ?? true;
		if (!resizable) return;
		resizeObserver = new ResizeObserver(() => {
			if (resizeRaf !== null) return;
			resizeRaf = requestAnimationFrame(() => {
				resizeRaf = null;
				post("resize", { height: document.documentElement.scrollHeight });
			});
		});
		resizeObserver.observe(document.documentElement);
	};

	const cleanupStandalone = () => {
		standaloneCleanup?.();
		standaloneCleanup = null;
	};

	const handleInit = (envelope: FlwEnvelope<"init">) => {
		const payload = envelope.payload as InitPayload;
		if (standaloneTimer !== null) {
			clearTimeout(standaloneTimer);
			standaloneTimer = null;
		}
		cleanupStandalone();
		nonce = envelope.nonce;
		instanceId = payload.instanceId || envelope.instanceId;
		preview = payload.capabilities?.preview === true;
		$props.set(mergeInitProps(contract, payload.props ?? {}) as I);
		setAndApplyTheme(payload.theme);
		post("ready", {
			contractVersion: contract?.contractVersion ?? CONTRACT_VERSION,
		});
		$mode.set("hosted");
		startAutoResize();
	};

	const handlePropsUpdate = (payload: PropsUpdatePayload) => {
		const { accepted, rejected } = filterPropsPatch(
			contract,
			payload.props ?? {},
		);
		for (const entry of rejected) {
			console.warn(
				`[flow-like/widget-sdk] dropped props:update value for "${entry.key}": ${entry.errors.join("; ")}`,
			);
		}
		const store = $props as MapStore<Record<string, unknown>>;
		for (const [key, value] of Object.entries(accepted)) {
			store.setKey(key, value);
		}
	};

	const handleQuery = async (payload: QueryPayload) => {
		const handler = queryHandlers.get(payload.name);
		if (!handler) {
			post("query:result", {
				queryId: payload.queryId,
				ok: false,
				error: `Unknown query "${payload.name}"`,
			});
			return;
		}
		try {
			const value = await handler(payload.args);
			post("query:result", { queryId: payload.queryId, ok: true, value });
		} catch (error) {
			post("query:result", {
				queryId: payload.queryId,
				ok: false,
				error: error instanceof Error ? error.message : String(error),
			});
		}
	};

	const onMessage = (event: MessageEvent) => {
		if (disposed) return;
		const envelope = acceptEnvelope(
			event.data,
			nonce,
			event.source === window.parent,
		);
		if (!envelope) return;
		switch (envelope.type) {
			case "init":
				handleInit(envelope as FlwEnvelope<"init">);
				break;
			case "props:update":
				handlePropsUpdate(envelope.payload as PropsUpdatePayload);
				break;
			case "theme:change":
				setAndApplyTheme(envelope.payload as ThemeState);
				break;
			case "query":
				void handleQuery(envelope.payload as QueryPayload);
				break;
			default:
				break;
		}
	};

	const emit = <Name extends keyof E & string>(
		name: Name,
		...args: EmitPayloadArgs<E, Name>
	) => {
		const payload = (args as unknown[])[0];
		if ($mode.get() !== "hosted") {
			console.info("[flow-like/widget-sdk] event (standalone)", {
				name,
				payload,
			});
			return;
		}
		if (preview) return;
		if (contract) {
			const eventSpec = contract.events?.[name];
			if (!eventSpec) {
				console.warn(
					`[flow-like/widget-sdk] dropped event "${name}": not declared in the contract`,
				);
				return;
			}
			const result = validateSchema(eventSpec.payloadSchema, payload);
			if (!result.valid) {
				console.warn(
					`[flow-like/widget-sdk] dropped event "${name}" with invalid payload: ${result.errors.join("; ")}`,
				);
				return;
			}
		}
		post("event", { name, payload });
	};

	const onQuery = <Name extends keyof Q & string>(
		name: Name,
		handler: (
			args: QueryArgs<Q, Name>,
		) => QueryReturns<Q, Name> | Promise<QueryReturns<Q, Name>>,
	): (() => void) => {
		const untyped = handler as (args: unknown) => unknown;
		queryHandlers.set(name, untyped);
		return () => {
			if (queryHandlers.get(name) === untyped) queryHandlers.delete(name);
		};
	};

	const setValues = (values: Record<string, unknown>) => {
		if ($mode.get() === "hosted") {
			post("value:changed", { values });
		} else {
			console.info("[flow-like/widget-sdk] value:changed (standalone)", values);
		}
	};

	const dispose = () => {
		if (disposed) return;
		disposed = true;
		window.removeEventListener("message", onMessage);
		if (standaloneTimer !== null) {
			clearTimeout(standaloneTimer);
			standaloneTimer = null;
		}
		if (resizeRaf !== null) {
			cancelAnimationFrame(resizeRaf);
			resizeRaf = null;
		}
		resizeObserver?.disconnect();
		resizeObserver = null;
		cleanupStandalone();
		queryHandlers.clear();
	};

	const bridge: WidgetBridge<I, E, Q> = {
		$props,
		$theme,
		$mode,
		emit,
		onQuery,
		setValues,
		dispose,
	};

	const bootStandalone = () => {
		if (disposed || $mode.get() !== "connecting") return;
		standaloneTimer = null;
		$props.set(contractDefaults(contract) as I);
		setAndApplyTheme(themeForMode(detectColorScheme()));
		const unwatch = watchColorScheme((mode) => {
			setAndApplyTheme(themeForMode(mode));
		});
		const removeBadge = renderStandaloneBadge();
		const standaloneGlobal: FlwStandaloneGlobal = {
			query: async (name, args) => {
				const handler = queryHandlers.get(name);
				if (!handler) {
					throw new Error(`No handler registered for query "${name}"`);
				}
				return await handler(args);
			},
			bridge: bridge as unknown as WidgetBridge,
		};
		flwGlobals.__flw = standaloneGlobal;
		standaloneCleanup = () => {
			unwatch();
			removeBadge();
			if (flwGlobals.__flw === standaloneGlobal) flwGlobals.__flw = undefined;
		};
		$mode.set("standalone");
	};

	window.addEventListener("message", onMessage);
	if (window.self !== window.top) {
		window.parent.postMessage(createEnvelope("hello", {}, "", ""), "*");
		standaloneTimer = setTimeout(bootStandalone, INIT_TIMEOUT_MS);
	} else {
		bootStandalone();
	}

	return bridge;
}
