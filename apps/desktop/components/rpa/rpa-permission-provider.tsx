"use client";

import { useCallback, useEffect, useRef, useState } from "react";
import {
	type RpaConsentRememberScope,
	type RpaConsentRequest,
	type RpaSystemPermissionRequest,
	saveRpaAutomationConsent,
} from "./rpa-consent";
import { RpaConsentDialog } from "./rpa-consent-dialog";
import { RpaPermissionDialog } from "./rpa-permission-dialog";

type PermissionRequest = RpaSystemPermissionRequest & {
	nodeId?: string;
	payload?: object;
	legacyRetry?: boolean;
};

function useRequestQueue<T extends { requestId: string }>(
	name: string,
	normalize: (detail: T) => T,
) {
	const requests = useRef<T[]>([]);
	const [current, setCurrent] = useState<T | null>(null);
	const finish = useCallback(
		(requestId: string, granted: boolean) => {
			if (requests.current[0]?.requestId !== requestId) return;
			const request = requests.current.shift();
			if (!request) return;
			setCurrent(requests.current[0] ?? null);
			window.dispatchEvent(
				new CustomEvent(`${name}-result`, { detail: { requestId, granted } }),
			);
			return request;
		},
		[name],
	);
	useEffect(() => {
		const receive = (event: Event) => {
			const request = normalize((event as CustomEvent<T>).detail);
			if (
				!request ||
				requests.current.some((item) => item.requestId === request.requestId)
			)
				return;
			requests.current.push(request);
			setCurrent(requests.current[0]);
		};
		window.addEventListener(`${name}-required`, receive);
		return () => {
			window.removeEventListener(`${name}-required`, receive);
			for (const request of requests.current.splice(0)) {
				window.dispatchEvent(
					new CustomEvent(`${name}-result`, {
						detail: { requestId: request.requestId, granted: false },
					}),
				);
			}
		};
	}, [name, normalize]);
	return { current, finish };
}

const normalizeConsent = (request: RpaConsentRequest) => request;
const normalizePermission = (
	request: PermissionRequest,
): PermissionRequest => ({
	...request,
	requestId: request.requestId ?? crypto.randomUUID(),
	required: request.required ?? ["input_control", "screen_capture"],
	legacyRetry: !request.requestId,
});

export function RpaPermissionProvider() {
	const consent = useRequestQueue("flow:rpa-consent", normalizeConsent);
	const permission = useRequestQueue(
		"flow:rpa-permissions",
		normalizePermission,
	);
	const [saving, setSaving] = useState(false);
	const [consentError, setConsentError] = useState<string | null>(null);
	const consentRequest = consent.current;
	const permissionRequest = permission.current;

	const confirmConsent = async (rememberFor: RpaConsentRememberScope) => {
		if (!consentRequest || saving) return;
		setSaving(true);
		setConsentError(null);
		try {
			await saveRpaAutomationConsent(consentRequest, rememberFor);
			consent.finish(consentRequest.requestId, true);
		} catch (error) {
			setConsentError(error instanceof Error ? error.message : String(error));
		} finally {
			setSaving(false);
		}
	};

	const finishPermission = (granted: boolean) => {
		if (!permissionRequest) return;
		const completed = permission.finish(permissionRequest.requestId, granted);
		if (completed?.legacyRetry && granted) {
			window.dispatchEvent(
				new CustomEvent("flow:rpa-permissions-retry", { detail: completed }),
			);
		}
	};

	return (
		<>
			<RpaConsentDialog
				key={consentRequest?.requestId ?? "no-consent"}
				open={!!consentRequest}
				context={consentRequest?.context ?? "execution"}
				boardId={consentRequest?.boardId}
				eventId={consentRequest?.eventId}
				required={consentRequest?.required}
				pending={saving}
				error={consentError}
				onCancel={() => {
					if (consentRequest) consent.finish(consentRequest.requestId, false);
					setConsentError(null);
				}}
				onConfirm={confirmConsent}
			/>
			<RpaPermissionDialog
				key={permissionRequest?.requestId ?? "no-permission"}
				open={!!permissionRequest}
				required={permissionRequest?.required}
				onOpenChange={(open) => {
					if (!open) finishPermission(false);
				}}
				onPermissionsGranted={() => finishPermission(true)}
			/>
		</>
	);
}
