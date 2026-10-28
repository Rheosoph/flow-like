"use client";

import { PatSelectorDialog, useBackend } from "@flow-like/flow-like-ui";
import { getErrorMessage } from "@flow-like/flow-like-ui/lib/error-message";
import { useTranslation } from "@flow-like/locales";
import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import { AuthContext } from "react-oidc-context";
import { toast } from "sonner";
import type { TauriBackend } from "../tauri-provider";
import {
	LOCAL_SINK_CONSENT_REQUIRED,
	LOCAL_SINK_CONSENT_RESULT,
	type LocalSinkConsentAnswer,
	type LocalSinkConsentRememberScope,
	type LocalSinkConsentRequest,
	type LocalSinkConsentResult,
	isWrongAccountRefusal,
	saveLocalSinkConsent,
} from "./local-sink-consent";
import {
	LocalSinkConsentDialog,
	LocalTriggerTokenDialog,
} from "./local-sink-consent-dialog";

/** Pause between two dialogs, so a click cannot answer the next one unseen. */
const SETTLE_MS = 300;

/** "picker" is the token list the "token" step and an approval lead to, and a repair opens with. */
type ConsentStage = "consent" | "token" | "picker";

type AnswerRequest = (
	request: LocalSinkConsentRequest,
	answer: LocalSinkConsentAnswer,
	pat?: string,
) => void;

/**
 * A repair opens the token list itself: the token step's "Save without
 * trigger" belongs to a save, and the user already chose "Add token".
 */
function openingStage(request: LocalSinkConsentRequest | null): ConsentStage {
	if (request?.approve) return "consent";
	return request?.repair ? "picker" : "token";
}

function enqueue(request: LocalSinkConsentRequest) {
	return (queue: LocalSinkConsentRequest[]) => [...queue, request];
}

function dequeue(requestId: string) {
	const others = (queued: LocalSinkConsentRequest) =>
		queued.requestId !== requestId;
	return (queue: LocalSinkConsentRequest[]) => queue.filter(others);
}

/** The requests waiting for an answer, answered one at a time and each only once. */
function useConsentQueue() {
	const [queue, setQueue] = useState<LocalSinkConsentRequest[]>([]);
	const [settling, setSettling] = useState(false);
	const answered = useRef(new Set<string>());

	useEffect(() => {
		const onRequired = (event: Event) => {
			const request = (event as CustomEvent<LocalSinkConsentRequest>).detail;
			setQueue(enqueue(request));
		};
		window.addEventListener(LOCAL_SINK_CONSENT_REQUIRED, onRequired);
		return () => {
			window.removeEventListener(LOCAL_SINK_CONSENT_REQUIRED, onRequired);
		};
	}, []);

	useEffect(() => {
		if (!settling) return;
		const handle = window.setTimeout(() => {
			setSettling(false);
		}, SETTLE_MS);
		return () => {
			window.clearTimeout(handle);
		};
	}, [settling]);

	const pause = useCallback(() => {
		setSettling(true);
	}, []);

	const answer = useCallback<AnswerRequest>((request, result, pat) => {
		if (answered.current.has(request.requestId)) return;
		answered.current.add(request.requestId);
		window.dispatchEvent(
			new CustomEvent<LocalSinkConsentResult>(LOCAL_SINK_CONSENT_RESULT, {
				detail: { answer: result, requestId: request.requestId, pat },
			}),
		);
		setQueue(dequeue(request.requestId));
		setSettling(true);
	}, []);

	return { pending: queue[0] ?? null, settling, pause, answer };
}

/**
 * Hosts the dialogs behind `requestLocalSinkConsent`: the approval, then the
 * token the trigger signs in with.
 */
export function LocalSinkConsentProvider() {
	const { t } = useTranslation("common");
	const { pending, settling, pause, answer } = useConsentQueue();
	const [progress, setProgress] = useState<{
		requestId: string;
		stage: ConsentStage;
	} | null>(null);
	const stage =
		progress && progress.requestId === pending?.requestId
			? progress.stage
			: openingStage(pending);

	const advance = useCallback(
		(request: LocalSinkConsentRequest, next: ConsentStage) => {
			setProgress({ requestId: request.requestId, stage: next });
			pause();
		},
		[pause],
	);

	const confirm = useCallback(
		(rememberFor: LocalSinkConsentRememberScope) => {
			if (!pending) return;
			if (rememberFor !== "none") saveLocalSinkConsent(rememberFor, pending);
			if (pending.needsToken) advance(pending, "picker");
			else answer(pending, "allow");
		},
		[advance, answer, pending],
	);

	const decline = useCallback(() => {
		if (!pending) return;
		toast.info(
			t(
				"theEventIsSavedWithoutATriggerOnThisDevice",
				"The event is saved without a trigger on this device.",
			),
		);
		answer(pending, "decline");
	}, [answer, pending, t]);

	const dismiss = useCallback(() => {
		if (pending) answer(pending, "dismiss");
	}, [answer, pending]);

	const chooseToken = useCallback(() => {
		if (pending) advance(pending, "picker");
	}, [advance, pending]);

	const visible = pending !== null && !settling;
	return (
		<>
			<LocalSinkConsentDialog
				open={visible && stage === "consent"}
				eventName={pending?.eventName}
				eventType={pending?.eventType}
				needsToken={pending?.needsToken}
				movesFromApp={pending?.movesFromApp}
				onDecline={decline}
				onDismiss={dismiss}
				onConfirm={confirm}
			/>
			<LocalTriggerTokenDialog
				open={visible && stage === "token"}
				eventName={pending?.eventName}
				eventType={pending?.eventType}
				onDecline={decline}
				onDismiss={dismiss}
				onChooseToken={chooseToken}
			/>
			{pending && stage === "picker" && (
				<LocalTriggerTokenPicker
					key={pending.requestId}
					request={pending}
					open={visible}
					onAnswer={answer}
				/>
			)}
		</>
	);
}

/**
 * The token list. A chosen token is checked against the signed-in account
 * before it answers the request; closing the list abandons the save.
 */
function LocalTriggerTokenPicker({
	request,
	open,
	onAnswer,
}: Readonly<{
	request: LocalSinkConsentRequest;
	open: boolean;
	onAnswer: AnswerRequest;
}>) {
	const { t } = useTranslation("common");
	const backend = useBackend() as TauriBackend | undefined;
	const verifying = useRef(false);
	const closingAfterSelection = useRef(false);

	const verify = useCallback(
		async (pat: string) => {
			verifying.current = true;
			try {
				await invoke("verify_trigger_token", { appId: request.appId, pat });
				onAnswer(request, "allow", pat);
			} catch (error) {
				const reason = getErrorMessage(error);
				if (isWrongAccountRefusal(reason)) {
					toast.error(
						t(
							"triggerTokenWrongAccount",
							"This token belongs to another account. Choose one of your own tokens for this hub.",
						),
					);
				} else {
					toast.error(
						t(
							"settings:couldNotAddToken",
							"Could not add a token to {{name}}.",
							{
								name: request.eventName,
							},
						),
						{ description: reason },
					);
				}
			} finally {
				verifying.current = false;
			}
		},
		[onAnswer, request, t],
	);

	const select = useCallback(
		(pat: string) => {
			closingAfterSelection.current = true;
			if (!verifying.current) void verify(pat);
		},
		[verify],
	);

	const close = useCallback(
		(nextOpen: boolean) => {
			if (nextOpen) return;
			if (closingAfterSelection.current) {
				closingAfterSelection.current = false;
				return;
			}
			onAnswer(request, "dismiss");
		},
		[onAnswer, request],
	);

	return (
		// This provider sits outside the auth tree so a profile switch never drops a
		// pending request; the list keys its stored token by the backend's session.
		<AuthContext.Provider value={backend?.auth}>
			<PatSelectorDialog
				open={open}
				onOpenChange={close}
				onPatSelected={select}
				title={t("localTriggerTokenTitle", "Choose a token for {{name}}", {
					name: request.eventName,
				})}
				description={t(
					"localTriggerTokenDescription",
					"This trigger runs on this computer and signs in to the hub with a personal access token. Without one, the event is saved but its trigger doesn't run here.",
				)}
			/>
		</AuthContext.Provider>
	);
}
