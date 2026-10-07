import type {
	RemotePushApi,
	RemotePushListener,
	RemotePushPayload,
} from "./remote-push";

export interface RemotePushRegistrationOptions {
	api: RemotePushApi;
	register: (token: string) => Promise<void>;
	onNotification: (notification: RemotePushPayload) => void | Promise<void>;
	onError: (error: unknown) => void;
	retryBaseDelayMs?: number;
	retryMaxDelayMs?: number;
}

export interface RemotePushRegistration {
	refresh: () => void;
	stop: () => void;
}

export function startRemotePushRegistration({
	api,
	register,
	onNotification,
	onError,
	retryBaseDelayMs = 1_000,
	retryMaxDelayMs = 60_000,
}: RemotePushRegistrationOptions): RemotePushRegistration {
	let stopped = false;
	let running = false;
	let tokenListener: RemotePushListener | undefined;
	let notificationListener: RemotePushListener | undefined;
	let retryTimer: ReturnType<typeof setTimeout> | undefined;
	let retryAttempt = 0;
	let acquireRequested = true;
	let registrationRequested = false;
	let permissionGranted = false;
	let permissionDenied = false;
	let latestToken: string | undefined;
	let tokenRevision = 0;

	function clearRetry() {
		if (retryTimer !== undefined) {
			clearTimeout(retryTimer);
			retryTimer = undefined;
		}
	}

	function unregister(listener: RemotePushListener | undefined) {
		if (!listener) return;
		void Promise.resolve()
			.then(() => listener.unregister())
			.catch((error) => {
				if (!stopped) onError(error);
			});
	}

	function scheduleRetry() {
		if (stopped || retryTimer !== undefined) return;
		const delay = Math.min(
			retryMaxDelayMs,
			retryBaseDelayMs * 2 ** Math.min(retryAttempt++, 30),
		);
		retryTimer = setTimeout(() => {
			retryTimer = undefined;
			void run();
		}, delay);
	}

	async function subscribe(): Promise<boolean> {
		if (!tokenListener) {
			const listener = await api.onTokenRefresh((token) => {
				if (stopped || !token || token === latestToken) return;
				latestToken = token;
				tokenRevision++;
				registrationRequested = true;
				if (permissionGranted) {
					acquireRequested = false;
					clearRetry();
					void run();
				} else if (!permissionDenied && !running) {
					acquireRequested = true;
					clearRetry();
					void run();
				}
			});
			if (stopped) {
				unregister(listener);
				return false;
			}
			tokenListener = listener;
		}
		if (!notificationListener) {
			const listener = await api.onNotificationReceived((notification) => {
				if (stopped) return;
				void Promise.resolve()
					.then(() => {
						if (!stopped) return onNotification(notification);
					})
					.catch((error) => {
						if (!stopped) onError(error);
					});
			});
			if (stopped) {
				unregister(listener);
				return false;
			}
			notificationListener = listener;
		}
		return true;
	}

	async function run() {
		if (stopped || running) return;
		running = true;
		let phase: "subscribe" | "acquire" | "register" = "subscribe";
		try {
			// Subscribe before asking for a token: native registration can emit a
			// refreshed token while getToken or the first server request is pending.
			if (!(await subscribe())) return;
			while (!stopped && (acquireRequested || registrationRequested)) {
				if (acquireRequested) {
					phase = "acquire";
					acquireRequested = false;
					const permission = await api.requestPermission();
					if (stopped) return;
					permissionGranted = permission.granted;
					permissionDenied = !permission.granted;
					if (!permissionGranted) {
						registrationRequested = false;
						retryAttempt = 0;
						return;
					}

					const revision = tokenRevision;
					const token = await api.getToken();
					if (stopped) return;
					if (!token) throw new Error("Remote push returned an empty token.");
					// A callback received during acquisition is newer than its result.
					if (revision === tokenRevision) {
						latestToken = token;
						registrationRequested = true;
					}
				}

				if (registrationRequested && permissionGranted && latestToken) {
					phase = "register";
					registrationRequested = false;
					await register(latestToken);
					if (stopped) return;
					retryAttempt = 0;
				} else {
					return;
				}
			}
		} catch (error) {
			if (stopped) return;
			if (phase === "acquire") {
				acquireRequested = !permissionGranted || !registrationRequested;
			} else if (phase === "register") {
				registrationRequested = true;
			}
			onError(error);
			scheduleRetry();
		} finally {
			running = false;
		}
	}

	void run();
	return {
		refresh() {
			if (stopped) return;
			clearRetry();
			acquireRequested = true;
			void run();
		},
		stop() {
			if (stopped) return;
			stopped = true;
			clearRetry();
			unregister(tokenListener);
			unregister(notificationListener);
			tokenListener = undefined;
			notificationListener = undefined;
		},
	};
}
