"use client";

/*
 * The form session for a mounted interface (PLAN §3.2, §3.7): the controller of this form from the
 * registry (shared across remounts; a second concurrent mount gets its own), attached while the
 * component is mounted, read through `useSyncExternalStore`. The host's event and config become a
 * FormModel; a real change of it reaches the session as `formChanged`, nothing else does.
 */
import {
	useEffect,
	useMemo,
	useRef,
	useState,
	useSyncExternalStore,
} from "react";
import {
	isRuntimeNamespace,
	runtimeMemory,
} from "../../../../lib/service-runtime/session-scope";
import { useBackend } from "../../../../state/backend-state";
import { useExecutionEngine } from "../../../../state/execution-engine-context";
import type {
	FormModel,
	FormSessionHandle,
	HostCapabilities,
	UseFormSession,
	UseFormSessionInput,
} from "../contracts";
import { createFormModel } from "../model/fields";
import { createFormMemoryStore } from "../store/form-memory";
import {
	type FormSessionController,
	type RuntimeBackend,
	createSessionController,
	localDay,
} from "./controller";
import type { RuntimeEngine } from "./dispatch-run";
import { reduceSession } from "./reduce";
import {
	claimSessionController,
	detachSessionController,
	getSessionController,
} from "./registry";
import { initialSessionState } from "./state";

/** Sessions of one form part by host kind and memory scope: a profile or account switch starts afresh. */
export const partitionOf = (
	host: Pick<HostCapabilities, "kind" | "memoryScope">,
): string => `${host.kind}:${host.memoryScope ?? ""}`;

/** Session memory of a Devices runtime view lives in its namespace and goes when the view closes. */
const sessionMapOf = (appId: string) =>
	isRuntimeNamespace(appId) ? (runtimeMemory(appId) ?? null) : null;

interface ControllerParts {
	readonly engine: RuntimeEngine;
	readonly backend: RuntimeBackend;
	readonly form: FormModel;
	readonly navigate: UseFormSessionInput["navigate"];
}

const controllerFor = (parts: ControllerParts) =>
	createSessionController({
		engine: parts.engine,
		backend: parts.backend,
		form: parts.form,
		reduce: reduceSession,
		initial: initialSessionState,
		memory: createFormMemoryStore(
			parts.form.host.persistence,
			sessionMapOf(parts.form.appId),
		),
		navigate: parts.navigate,
	});

interface Held {
	readonly engine: object;
	readonly key: string;
	readonly controller: FormSessionController;
}

function useFormModel(input: UseFormSessionInput): FormModel {
	const { event, config, host, viewer, appId } = input;
	return useMemo(
		() =>
			createFormModel(event, config, host, viewer, localDay(new Date()), appId),
		[event, config, host, viewer, appId],
	);
}

/**
 * A mount hands its backend, navigate and form only to the controller it attached. The one it rendered
 * with first may be held by another mount of the same form (that mount's navigate and presentation
 * stay its own); the attach effect gives the claimed controller this mount's parts before `attach()`,
 * so navigation flushed on attach goes through this host.
 */
export const useFormSession: UseFormSession = (input) => {
	const engine = useExecutionEngine();
	const backend = useBackend();
	const form = useFormModel(input);
	const { appId, eventId } = form;
	const partition = partitionOf(form.host);
	const key = `${appId}\u0000${eventId}\u0000${partition}`;
	const parts: ControllerParts = {
		engine,
		backend,
		form,
		navigate: input.navigate,
	};
	const latest = useRef(parts);
	const attachedRef = useRef<FormSessionController | null>(null);
	const create = () => controllerFor(parts);
	const [held, setHeld] = useState<Held>(() => ({
		engine,
		key,
		controller: getSessionController(engine, appId, eventId, create, partition),
	}));
	const controller =
		held.engine === engine && held.key === key
			? held.controller
			: getSessionController(engine, appId, eventId, create, partition);

	useEffect(() => {
		latest.current = parts;
	});

	useEffect(() => {
		if (attachedRef.current !== controller) return;
		controller.update({ backend, navigate: input.navigate });
	}, [controller, backend, input.navigate]);

	useEffect(() => {
		if (attachedRef.current === controller) controller.setForm(form);
	}, [controller, form]);

	useEffect(() => {
		const claimed = claimSessionController(
			engine,
			appId,
			eventId,
			controller,
			() => controllerFor(latest.current),
			partition,
		);
		const now = latest.current;
		claimed.update({ backend: now.backend, navigate: now.navigate });
		claimed.setForm(now.form);
		claimed.attach();
		attachedRef.current = claimed;
		setHeld((previous) =>
			previous.engine === engine &&
			previous.key === key &&
			previous.controller === claimed
				? previous
				: { engine, key, controller: claimed },
		);
		return () => {
			if (attachedRef.current === claimed) attachedRef.current = null;
			detachSessionController(claimed);
		};
	}, [engine, appId, eventId, partition, key, controller]);

	const state = useSyncExternalStore(
		controller.subscribe,
		controller.getState,
		controller.getState,
	);
	return useMemo<FormSessionHandle>(
		() => ({ state, actions: controller.actions }),
		[state, controller],
	);
};
