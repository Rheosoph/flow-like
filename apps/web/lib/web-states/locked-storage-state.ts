import {
	LockedStorageClient,
	lockedStoragePath,
} from "@flow-like/flow-like-ui/lib/locked-storage";
import { type WebBackendRef, apiFetch } from "./api-utils";

export class WebLockedStorageState extends LockedStorageClient {
	constructor(backend: WebBackendRef) {
		super((appId, action, { method, body, signal }) =>
			apiFetch(
				lockedStoragePath(appId, action),
				{
					method,
					body: body === undefined ? undefined : JSON.stringify(body),
					signal,
				},
				backend.auth,
			),
		);
	}
}
