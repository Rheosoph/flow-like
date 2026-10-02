"use client";

import { useTranslation } from "@flow-like/locales";
import {
	type RefObject,
	useCallback,
	useEffect,
	useRef,
	useState,
} from "react";
import type {
	SetupRoute,
	SetupStep,
} from "../../../../lib/device-management/model/types";
import { useAreaTime } from "../primitives/area-context";
import { useDevicesRoute } from "../routing/use-devices-route";
import { useHubSupport } from "../workspace";
import type { SetupController, SetupSecrets } from "./setup-context";
import { checksBlocked, nextState } from "./setup-next";
import {
	CREATE_STEP,
	type SetupDraft,
	type TargetOption,
	WAIT_STEP,
	isExpired,
	modeAvailable,
	nameIssue,
	packsAgent,
	passwordIssue,
	repeatIssue,
} from "./setup-state";
import { type SetupChecks, useSetupChecks } from "./use-setup-checks";
import {
	type CreatedResult,
	type PrepareSetup,
	type SetupCreate,
	useSetupCreate,
} from "./use-setup-create";
import {
	useKnownRelease,
	useSetupLimits,
	useStepLabels,
	useTakenNames,
} from "./use-setup-facts";
import { useSetupLeave } from "./use-setup-leave";
import { type SetupSteps, useSetupSteps } from "./use-setup-steps";
import { useSetupArrival } from "./use-setup-wait";

export interface SetupFlowInput {
	route: SetupRoute;
	draft: SetupDraft;
	/** `new`, or the enrollment id in the URL. */
	slot: string;
	prepare: PrepareSetup;
	update(patch: Partial<SetupDraft>): void;
	startNew(name?: string): void;
}

export interface SetupFlowState {
	controller: SetupController;
	steps: SetupSteps;
	formRef: RefObject<HTMLFormElement | null>;
	/** The step's primary, also reached by Enter in a field: a gated primary does nothing (R7). */
	advance(): void;
}

/** The password fields: held in memory only, and cleared once the package exists. */
function useSecrets(): SetupSecrets & { clear(): void } {
	const [password, setPassword] = useState("");
	const [repeat, setRepeat] = useState("");
	const clear = useCallback(() => {
		setPassword("");
		setRepeat("");
	}, []);
	return { password, repeat, setPassword, setRepeat, clear };
}

interface CreateInput {
	draft: SetupDraft;
	option: TargetOption | undefined;
	checks: SetupChecks;
	secrets: SetupSecrets & { clear(): void };
	create: SetupCreate;
	lifetimeS: number;
	update(patch: Partial<SetupDraft>): void;
}

/** The choices a package is made from, once each is made and valid. */
function choicesOf(draft: SetupDraft, option: TargetOption | undefined) {
	const { name, target, mode } = draft;
	if (draft.created || !target || !mode) return undefined;
	if (nameIssue(name) || !modeAvailable(option, mode)) return undefined;
	return { name, target, mode };
}

/** Starting and retrying the creation, and filing its result in the draft. */
function useCreateHandlers(input: CreateInput) {
	const { draft, option, checks, secrets, create, lifetimeS, update } = input;
	const { password, repeat, clear } = secrets;

	const finish = useCallback(
		(result: CreatedResult | undefined) => {
			if (!result) return;
			clear();
			const { registration, finishedAt } = result;
			update({
				keySaved: false,
				packageSaved: false,
				acknowledged: false,
				created: {
					...(registration ? { enrollmentId: registration.enrollmentId } : {}),
					deviceId: result.deviceId,
					createdAt: registration?.createdAt ?? finishedAt,
					expiresAt: registration?.expiresAt ?? finishedAt + lifetimeS,
					outcome: result.outcome,
				},
			});
		},
		[clear, update, lifetimeS],
	);

	/** Sends nothing unless every check holds right now: readiness, the verified release, the choices and the password. */
	const start = useCallback(async () => {
		const { verified, config, pass } = checks;
		const choices = choicesOf(draft, option);
		if (!pass || !verified || !config || !choices) return;
		if (passwordIssue(password) || repeatIssue(password, repeat)) return;
		finish(
			await create.start({
				...choices,
				password,
				backup: draft.backup,
				packsAgent: packsAgent(option, choices.mode),
				release: config,
				verified,
				gateExtra: { readinessOk: true, releaseTrust: true },
			}),
		);
	}, [checks, draft, option, password, repeat, create, finish]);

	const retry = useCallback(async () => {
		const { verified, config, pass } = checks;
		if (!pass || !verified || !config) return;
		finish(await create.retry({ release: config, verified }));
	}, [checks, create, finish]);

	return { start, retry };
}

/**
 * A failed run belongs to the choices it was made from. Leaving the Create
 * step drops it, so coming back reviews the choices as they are now and
 * creates from those, never from the request that failed.
 */
function useDropFailedRun(create: SetupCreate, step: SetupStep) {
	const failed = create.run.status === "failed";
	const { reset } = create;
	useEffect(() => {
		if (failed && step !== CREATE_STEP) reset();
	}, [failed, step, reset]);
}

/** Everything the wizard's steps read and do, for one setup (SPEC §5.5). */
export function useSetupController(input: SetupFlowInput): SetupFlowState {
	const { route, draft, slot, prepare, update, startNew } = input;
	const { t } = useTranslation("devices");
	const { navigate } = useDevicesRoute();
	const { host } = useHubSupport();
	const checks = useSetupChecks();
	const create = useSetupCreate(prepare);
	const limits = useSetupLimits();
	const labels = useStepLabels();
	const takenNames = useTakenNames(draft);
	const secrets = useSecrets();
	const formRef = useRef<HTMLFormElement>(null);
	const nowS = Math.floor(useAreaTime().nowS);
	const { release, options } = useKnownRelease(checks);

	const { created } = draft;
	const option = options.find((item) => item.target === draft.target);
	const creating = create.run.status === "running";
	const expired = isExpired(draft, nowS);
	const done = draft.checkedInAt !== undefined;
	const steps = useSetupSteps(
		route,
		{ draft, nowS, creating, options, checksPass: !checksBlocked(checks) },
		update,
	);
	const { step, goTo, markTried } = steps;
	useDropFailedRun(create, step);

	const wait = useSetupArrival(
		draft,
		step === WAIT_STEP && !draft.cancelledAt && !expired && !done,
		update,
	);
	const handlers = useCreateHandlers({
		draft,
		option,
		checks,
		secrets,
		create,
		lifetimeS: limits.lifetimeS,
		update,
	});
	const { cancelSetup, cancelError, dismissCancelError, leave, leaveLink } =
		useSetupLeave({
			draft,
			slot,
			step,
			expired,
			create,
			limits,
			update,
			clearSecrets: secrets.clear,
		});

	const next = nextState({
		t,
		step,
		draft,
		checks,
		option,
		password: secrets.password,
		firstIssue: passwordIssue(secrets.password),
		secondIssue: repeatIssue(secrets.password, secrets.repeat),
		create,
	});

	const showMissing = () => {
		markTried();
		requestAnimationFrame(() =>
			formRef.current
				?.querySelector<HTMLElement>('[aria-invalid="true"]')
				?.focus(),
		);
	};

	const forward = () => {
		if (step === CREATE_STEP && !created)
			return void (create.run.status === "failed"
				? handlers.retry()
				: handlers.start());
		if (step < WAIT_STEP) return goTo((step + 1) as SetupStep);
		if (created?.deviceId)
			navigate({
				screen: "device",
				deviceId: created.deviceId,
				tab: "overview",
			});
	};

	const advance = () => {
		if (draft.cancelledAt) return startNew();
		if (expired) return startNew(draft.name);
		if (next.ok) return forward();
		if (next.field) showMissing();
	};

	const controller: SetupController = {
		host,
		draft,
		step,
		labels,
		nowS,
		update,
		goTo,
		checks,
		release,
		options,
		option,
		secrets,
		tried: steps.tried,
		takenNames,
		limits,
		create,
		built: create.built,
		wait,
		expired,
		next,
		cancelError,
		dismissCancelError,
		cancelSetup: () => {
			void cancelSetup();
		},
		startNew,
		leave: (to) => {
			void leave(to);
		},
		leaveLink,
	};

	return { controller, steps, formRef, advance };
}
