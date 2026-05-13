"use client";

import { useEffect, useState } from "react";
import {
	type RpaConsentRememberScope,
	type RpaConsentRequest,
	saveRpaAutomationConsent,
} from "./rpa-consent";
import { RpaConsentDialog } from "./rpa-consent-dialog";
import { RpaPermissionDialog } from "./rpa-permission-dialog";

type RpaPermissionRetry = {
	appId: string;
	boardId: string;
	nodeId: string;
	payload?: object;
	permissions?: {
		accessibility: boolean;
		screen_recording: boolean;
	};
	checkError?: string;
	requestId?: string;
	skipConsentCheck?: boolean;
};

export function RpaPermissionProvider() {
	const [permissionOpen, setPermissionOpen] = useState(false);
	const [consentOpen, setConsentOpen] = useState(false);
	const [pendingRetry, setPendingRetry] = useState<RpaPermissionRetry | null>(
		null,
	);
	const [pendingSystemRequestId, setPendingSystemRequestId] = useState<
		string | null
	>(null);
	const [pendingConsent, setPendingConsent] =
		useState<RpaConsentRequest | null>(null);

	useEffect(() => {
		const handlePermissionsRequired = (event: Event) => {
			const permissionEvent = event as CustomEvent<RpaPermissionRetry>;
			if (permissionEvent.detail.requestId) {
				setPendingSystemRequestId(permissionEvent.detail.requestId);
				setPendingRetry(null);
			} else {
				setPendingSystemRequestId(null);
				setPendingRetry(permissionEvent.detail);
			}
			setPermissionOpen(true);
		};

		window.addEventListener(
			"flow:rpa-permissions-required",
			handlePermissionsRequired,
		);
		return () => {
			window.removeEventListener(
				"flow:rpa-permissions-required",
				handlePermissionsRequired,
			);
		};
	}, []);

	useEffect(() => {
		const handleConsentRequired = (event: Event) => {
			const consentEvent = event as CustomEvent<RpaConsentRequest>;
			setPendingConsent(consentEvent.detail);
			setConsentOpen(true);
		};

		window.addEventListener("flow:rpa-consent-required", handleConsentRequired);
		return () => {
			window.removeEventListener(
				"flow:rpa-consent-required",
				handleConsentRequired,
			);
		};
	}, []);

	const completeSystemPermissionRequest = (granted: boolean) => {
		if (!pendingSystemRequestId) return;
		window.dispatchEvent(
			new CustomEvent("flow:rpa-permissions-result", {
				detail: {
					granted,
					requestId: pendingSystemRequestId,
				},
			}),
		);
		setPendingSystemRequestId(null);
	};

	const completeConsentRequest = (granted: boolean) => {
		if (!pendingConsent) return;
		window.dispatchEvent(
			new CustomEvent("flow:rpa-consent-result", {
				detail: {
					granted,
					requestId: pendingConsent.requestId,
				},
			}),
		);
		setPendingConsent(null);
	};

	const retryExecution = () => {
		if (pendingSystemRequestId) {
			completeSystemPermissionRequest(true);
			return;
		}
		if (!pendingRetry) return;

		window.dispatchEvent(
			new CustomEvent("flow:rpa-permissions-retry", {
				detail: pendingRetry,
			}),
		);
		setPendingRetry(null);
	};

	const confirmConsent = (rememberFor: RpaConsentRememberScope) => {
		if (!pendingConsent) return;

		if (rememberFor === "board") {
			saveRpaAutomationConsent("board", pendingConsent.boardId);
		}
		if (rememberFor === "event" && pendingConsent.eventId) {
			saveRpaAutomationConsent("event", pendingConsent.eventId);
		}

		setConsentOpen(false);
		completeConsentRequest(true);
	};

	const cancelConsent = () => {
		setConsentOpen(false);
		completeConsentRequest(false);
	};

	return (
		<>
			<RpaConsentDialog
				open={consentOpen}
				context={pendingConsent?.context ?? "execution"}
				boardId={pendingConsent?.boardId}
				eventId={pendingConsent?.eventId}
				onCancel={cancelConsent}
				onConfirm={confirmConsent}
			/>
			<RpaPermissionDialog
				open={permissionOpen}
				onOpenChange={(nextOpen) => {
					setPermissionOpen(nextOpen);
					if (!nextOpen) {
						if (pendingSystemRequestId) completeSystemPermissionRequest(false);
						setPendingRetry(null);
					}
				}}
				onContinueAnyway={retryExecution}
				onPermissionsGranted={retryExecution}
			/>
		</>
	);
}
