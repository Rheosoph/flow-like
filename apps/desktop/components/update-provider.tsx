"use client";

import {
	type TelemetryBreadcrumbLevel,
	addTelemetryBreadcrumb,
} from "@flow-like/flow-like-ui/lib/telemetry/breadcrumbs";
import {
	captureTelemetryError,
	normalizeError,
} from "@flow-like/flow-like-ui/lib/telemetry/errors";
import { startTelemetrySpan } from "@flow-like/flow-like-ui/lib/telemetry/tracing";
import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { type UnlistenFn, listen } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { confirm } from "@tauri-apps/plugin-dialog";
import {
	type DownloadEvent,
	type Update,
	check,
} from "@tauri-apps/plugin-updater";
import { useEffect } from "react";
import { toast } from "sonner";
import { isMobileDevice, isTauriRuntime } from "../lib/platform";

const UPDATE_CHECK_INTERVAL = 30 * 60_000;
const UPDATE_CHECK_TIMEOUT = 30_000;
const UPDATE_TOAST_ID = "flow-like-update";
const UPDATE_MENU_TOAST_ID = "flow-like-update-menu";
const DISMISSED_VERSION_KEY = "updater:dismissed-version";
const UPDATE_ATTEMPT_KEY = "updater:install-attempt";
const UPDATE_ATTEMPT_MAX_AGE = 7 * 24 * 60 * 60_000;

type CheckSource = "automatic" | "manual";
type CheckTrigger = "automatic" | "manual" | "retry" | "tray";
type UpdateAttemptPhase =
	| "starting"
	| "downloading"
	| "verifying_or_installing"
	| "restart_pending";
const CHECK_TRIGGERS: readonly CheckTrigger[] = [
	"automatic",
	"manual",
	"retry",
	"tray",
];
type UpdaterErrorStage =
	| "check"
	| "download_install"
	| "interrupted"
	| "prompt"
	| "restart"
	| "tray_listener";

interface UpdaterErrorContext {
	check_source?: CheckSource;
	check_trigger?: CheckTrigger;
	content_length_bytes?: number;
	current_version?: string;
	downloaded_bytes?: number;
	duration_ms?: number;
	target_version?: string;
}

interface PersistedUpdateAttempt {
	attempt_id: string;
	current_version: string;
	phase: UpdateAttemptPhase;
	started_at_ms: number;
	target_version: string;
	trigger: CheckTrigger;
}

const UPDATE_ATTEMPT_PHASES: readonly UpdateAttemptPhase[] = [
	"starting",
	"downloading",
	"verifying_or_installing",
	"restart_pending",
];

function isCheckTrigger(value: unknown): value is CheckTrigger {
	return CHECK_TRIGGERS.some((trigger) => trigger === value);
}

function isUpdateAttemptPhase(value: unknown): value is UpdateAttemptPhase {
	return UPDATE_ATTEMPT_PHASES.some((phase) => phase === value);
}

function isSafeUpdaterVersion(value: unknown): value is string {
	return (
		typeof value === "string" &&
		value.length > 0 &&
		value.length <= 128 &&
		/^[0-9A-Za-z.+_-]+$/.test(value)
	);
}

function readUpdateAttempt(): PersistedUpdateAttempt | undefined {
	try {
		const raw = localStorage.getItem(UPDATE_ATTEMPT_KEY);
		if (!raw) return undefined;
		const parsed = JSON.parse(raw) as Partial<PersistedUpdateAttempt>;
		if (
			typeof parsed.attempt_id !== "string" ||
			parsed.attempt_id.length === 0 ||
			parsed.attempt_id.length > 128 ||
			!isSafeUpdaterVersion(parsed.current_version) ||
			!isSafeUpdaterVersion(parsed.target_version) ||
			typeof parsed.started_at_ms !== "number" ||
			!Number.isFinite(parsed.started_at_ms) ||
			parsed.started_at_ms <= 0 ||
			Date.now() - parsed.started_at_ms > UPDATE_ATTEMPT_MAX_AGE ||
			parsed.started_at_ms > Date.now() + 60_000 ||
			!isCheckTrigger(parsed.trigger) ||
			!isUpdateAttemptPhase(parsed.phase)
		) {
			localStorage.removeItem(UPDATE_ATTEMPT_KEY);
			return undefined;
		}
		return parsed as PersistedUpdateAttempt;
	} catch (error) {
		addUpdaterBreadcrumb("install_attempt_read_failed", "warning");
		console.warn(
			`Failed to read updater install attempt: ${normalizeError(error).value}`,
		);
		return undefined;
	}
}

function writeUpdateAttempt(attempt: PersistedUpdateAttempt) {
	try {
		localStorage.setItem(UPDATE_ATTEMPT_KEY, JSON.stringify(attempt));
	} catch (error) {
		addUpdaterBreadcrumb("install_attempt_write_failed", "warning");
		console.warn(
			`Failed to persist updater install attempt: ${normalizeError(error).value}`,
		);
	}
}

function clearUpdateAttempt(attemptId?: string) {
	try {
		if (attemptId) {
			const raw = localStorage.getItem(UPDATE_ATTEMPT_KEY);
			if (!raw) return;
			const current = JSON.parse(raw) as Partial<PersistedUpdateAttempt>;
			if (current.attempt_id !== attemptId) return;
		}
		localStorage.removeItem(UPDATE_ATTEMPT_KEY);
	} catch (error) {
		addUpdaterBreadcrumb("install_attempt_clear_failed", "warning");
		console.warn(
			`Failed to clear updater install attempt: ${normalizeError(error).value}`,
		);
	}
}

function updaterVersionContext(update: Update): UpdaterErrorContext {
	return {
		current_version: update.currentVersion,
		target_version: update.version,
	};
}

function addUpdaterBreadcrumb(
	message: string,
	level: TelemetryBreadcrumbLevel = "info",
) {
	addTelemetryBreadcrumb({
		category: "desktop.updater",
		message,
		level,
	});
}

function reportUpdaterError(
	stage: UpdaterErrorStage,
	error: unknown,
	context: UpdaterErrorContext = {},
) {
	addUpdaterBreadcrumb(`${stage}_failed`, "error");
	const telemetryContext: Record<string, unknown> = {
		subsystem: "updater",
		updater_stage: stage,
		...context,
	};
	if (typeof navigator !== "undefined") {
		telemetryContext.network_online = navigator.onLine;
	}
	captureTelemetryError(error, {
		culprit: `desktop/updater/${stage}`,
		context: telemetryContext,
	});
}

function formatBytes(bytes: number): string {
	if (bytes < 1024) return `${bytes} B`;
	if (bytes < 1024 ** 2) return `${(bytes / 1024).toFixed(1)} KB`;
	return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
}

export function UpdateProvider() {
	useEffect(() => {
		// The updater plugin is registered only for desktop Tauri builds. This
		// provider is also rendered by the browser and mobile applications.
		if (!isTauriRuntime() || isMobileDevice()) return;
		// Secondary desktop webviews share the root layout. Only the main window
		// may own updater resources, prompts, and the global tray listener.
		if (getCurrentWebviewWindow().label !== "main") return;

		let active = true;
		let checking: Promise<Update | null> | null = null;
		let pendingUpdate: Update | null = null;
		let installing = false;
		let prompting = false;
		let updateKnownAvailable = false;
		let manualCheckQueued = false;
		let queuedCheckTrigger: CheckTrigger | undefined;
		let installQueued = false;
		let queuedInstallTrigger: CheckTrigger | undefined;
		let hasVisibleCheckError = false;
		let lastReportedCheckError: string | undefined;
		let repeatedCheckFailureNoted = false;
		let lastBreadcrumbUpdateVersion: string | undefined;
		let unlistenTray: UnlistenFn | undefined;
		let registeringTrayListener = false;

		const reportInterruptedUpdate = async () => {
			const attempt = readUpdateAttempt();
			if (!attempt) return;
			const runningVersion = await getVersion().catch(() => undefined);
			if (!active || !runningVersion) return;
			if (runningVersion === attempt.target_version) {
				clearUpdateAttempt(attempt.attempt_id);
				return;
			}
			const interrupted = runningVersion === attempt.current_version;
			try {
				await invoke("queue_updater_interruption", {
					attempt: {
						currentVersion: attempt.current_version,
						targetVersion: attempt.target_version,
						runningVersion,
						attemptPhase: attempt.phase,
						checkTrigger: attempt.trigger,
						startedAtMs: attempt.started_at_ms,
						networkOnline: navigator.onLine,
					},
				});
				clearUpdateAttempt(attempt.attempt_id);
				addUpdaterBreadcrumb(
					interrupted
						? "interrupted_report_queued"
						: "inconclusive_outcome_report_queued",
					interrupted ? "error" : "warning",
				);
			} catch (error) {
				addUpdaterBreadcrumb("interrupted_report_persist_failed", "warning");
				console.warn(
					`Failed to persist interrupted updater report: ${normalizeError(error).value}`,
				);
			}
		};

		const closeUpdate = async (update: Update) => {
			try {
				await update.close();
			} catch (error) {
				addUpdaterBreadcrumb("resource_cleanup_failed", "warning");
				console.warn(
					`Failed to close updater resource: ${normalizeError(error).value}`,
				);
			}
		};

		const dismissedVersion = (): string | null => {
			try {
				return sessionStorage.getItem(DISMISSED_VERSION_KEY);
			} catch (error) {
				addUpdaterBreadcrumb("dismissal_state_read_failed", "warning");
				console.warn(
					`Failed to read updater dismissal state: ${normalizeError(error).value}`,
				);
				return null;
			}
		};

		const dismissVersion = (version: string) => {
			try {
				sessionStorage.setItem(DISMISSED_VERSION_KEY, version);
			} catch (error) {
				addUpdaterBreadcrumb("dismissal_state_write_failed", "warning");
				console.warn(
					`Failed to save updater dismissal state: ${normalizeError(error).value}`,
				);
			}
		};

		const setTrayAvailability = async (available: boolean) => {
			updateKnownAvailable = available;
			try {
				await invoke("tray_update_state", {
					update: { updateState: { available } },
				});
			} catch (error) {
				addUpdaterBreadcrumb("tray_state_failed", "warning");
				console.warn(
					`Failed to update updater state in the tray: ${normalizeError(error).value}`,
				);
			}
		};

		const showError = (
			title: string,
			stage: UpdaterErrorStage,
			error: unknown,
			retry: () => void,
			context: UpdaterErrorContext = {},
			toastId = UPDATE_TOAST_ID,
		) => {
			const description = normalizeError(error).value;
			reportUpdaterError(stage, error, context);
			console.error(`${title}: ${description}`);
			toast.error(title, {
				id: toastId,
				description,
				duration: Number.POSITIVE_INFINITY,
				closeButton: true,
				action: {
					label: "Retry",
					onClick: retry,
				},
			});
		};

		const installUpdate = async (update: Update, trigger: CheckTrigger) => {
			if (!active || installing) return;

			installing = true;
			pendingUpdate = update;
			const versionContext = updaterVersionContext(update);
			const currentVersion = versionContext.current_version ?? "current";
			const targetVersion = versionContext.target_version ?? update.version;
			const installSpan = startTelemetrySpan("desktop.updater.install", {
				kind: "client",
				attributes: {
					...versionContext,
					check_trigger: trigger,
				},
			});
			const installStartedAt = Date.now();
			const attemptId = crypto.randomUUID();
			let downloaded = 0;
			let contentLength: number | undefined;
			let lastRenderedAt = 0;
			let lastRenderedPercent = -1;

			const renderProgress = (event: DownloadEvent) => {
				if (!active) return;

				if (event.event === "Started") {
					contentLength = event.data.contentLength;
					writeUpdateAttempt({
						attempt_id: attemptId,
						current_version: currentVersion,
						phase: "downloading",
						started_at_ms: installStartedAt,
						target_version: targetVersion,
						trigger,
					});
					addUpdaterBreadcrumb(
						`download_started ${currentVersion} -> ${targetVersion}`,
					);
					toast.loading(`Downloading Flow Like ${update.version}…`, {
						id: UPDATE_TOAST_ID,
						description: contentLength
							? `0% · 0 MB of ${formatBytes(contentLength)}`
							: "Starting download…",
						duration: Number.POSITIVE_INFINITY,
					});
					return;
				}

				if (event.event === "Finished") {
					writeUpdateAttempt({
						attempt_id: attemptId,
						current_version: currentVersion,
						phase: "verifying_or_installing",
						started_at_ms: installStartedAt,
						target_version: targetVersion,
						trigger,
					});
					addUpdaterBreadcrumb(
						`download_finished ${currentVersion} -> ${targetVersion}`,
					);
					toast.loading(`Installing Flow Like ${update.version}…`, {
						id: UPDATE_TOAST_ID,
						description:
							"The application will restart when installation finishes.",
						duration: Number.POSITIVE_INFINITY,
					});
					return;
				}

				downloaded += event.data.chunkLength;
				const now = Date.now();
				const percent = contentLength
					? Math.min(100, Math.floor((downloaded / contentLength) * 100))
					: undefined;
				const shouldRender =
					now - lastRenderedAt >= 250 ||
					(percent !== undefined && percent === 100);

				if (!shouldRender || percent === lastRenderedPercent) return;
				lastRenderedAt = now;
				lastRenderedPercent = percent ?? lastRenderedPercent;
				const description =
					percent === undefined || contentLength === undefined
						? `${formatBytes(downloaded)} downloaded`
						: `${percent}% · ${formatBytes(downloaded)} of ${formatBytes(contentLength)}`;
				toast.loading(`Downloading Flow Like ${update.version}…`, {
					id: UPDATE_TOAST_ID,
					description,
					duration: Number.POSITIVE_INFINITY,
				});
			};

			toast.loading(`Downloading Flow Like ${update.version}…`, {
				id: UPDATE_TOAST_ID,
				description: "Preparing download…",
				duration: Number.POSITIVE_INFINITY,
			});
			writeUpdateAttempt({
				attempt_id: attemptId,
				current_version: currentVersion,
				phase: "starting",
				target_version: targetVersion,
				started_at_ms: installStartedAt,
				trigger,
			});

			try {
				await update.downloadAndInstall(renderProgress);
			} catch (error) {
				clearUpdateAttempt(attemptId);
				installSpan.end("error", {
					downloaded_bytes: downloaded,
					content_length_bytes: contentLength,
				});
				installing = false;
				if (!active) {
					pendingUpdate = null;
					void closeUpdate(update);
					return;
				}
				showError(
					"Flow Like could not be updated",
					"download_install",
					error,
					() => {
						void installUpdate(update, "retry");
					},
					{
						...versionContext,
						check_source: trigger === "automatic" ? "automatic" : "manual",
						check_trigger: trigger,
						downloaded_bytes: downloaded,
						content_length_bytes: contentLength,
						duration_ms: Date.now() - installStartedAt,
					},
				);
				return;
			}

			// The Windows updater exits the process and launches the installer from
			// inside downloadAndInstall. This branch is reached on macOS/Linux.
			installSpan.end("ok", {
				downloaded_bytes: downloaded,
				content_length_bytes: contentLength,
			});
			writeUpdateAttempt({
				attempt_id: attemptId,
				current_version: currentVersion,
				phase: "restart_pending",
				started_at_ms: installStartedAt,
				target_version: targetVersion,
				trigger,
			});
			pendingUpdate = null;
			addUpdaterBreadcrumb(
				`install_completed ${currentVersion} -> ${targetVersion}`,
			);
			await setTrayAvailability(false);
			await closeUpdate(update);

			if (active) {
				toast.success("Flow Like was updated", {
					id: UPDATE_TOAST_ID,
					description: "Restarting the application…",
					duration: Number.POSITIVE_INFINITY,
				});
			}

			const restartStartedAt = Date.now();
			addUpdaterBreadcrumb(`restart_requested ${targetVersion}`);
			try {
				await invoke("restart_app");
			} catch (error) {
				const retryRestart = () => {
					void invoke("restart_app").catch((retryError) => {
						showError(
							"The update is installed, but Flow Like could not restart",
							"restart",
							retryError,
							retryRestart,
							{
								...versionContext,
								duration_ms: Date.now() - restartStartedAt,
							},
						);
					});
				};
				showError(
					"The update is installed, but Flow Like could not restart",
					"restart",
					error,
					retryRestart,
					{
						...versionContext,
						duration_ms: Date.now() - restartStartedAt,
					},
				);
			}
		};

		const promptForUpdate = async (update: Update, trigger: CheckTrigger) => {
			if (!active || prompting || installing) return;
			prompting = true;
			const promptStartedAt = Date.now();
			addUpdaterBreadcrumb(
				`prompt_shown ${update.currentVersion} -> ${update.version}`,
			);

			try {
				const shouldUpdate = await confirm(
					`Flow Like ${update.version} is available. Would you like to download and install it now?`,
				);
				if (!active) return;

				if (shouldUpdate) {
					addUpdaterBreadcrumb(`prompt_accepted ${update.version}`);
					await installUpdate(update, trigger);
					return;
				}

				addUpdaterBreadcrumb(`prompt_dismissed ${update.version}`);
				dismissVersion(update.version);
				pendingUpdate = null;
				await closeUpdate(update);
				toast.info(`Flow Like ${update.version} is available`, {
					id: UPDATE_TOAST_ID,
					description: "Install it later from the tray menu.",
					action: {
						label: "Install",
						onClick: () => {
							void checkForUpdate("manual", true);
						},
					},
				});
			} catch (error) {
				if (!active) return;
				showError(
					"Flow Like could not start the update",
					"prompt",
					error,
					() => {
						void promptForUpdate(update, "retry");
					},
					{
						...updaterVersionContext(update),
						check_source: trigger === "automatic" ? "automatic" : "manual",
						check_trigger: trigger,
						duration_ms: Date.now() - promptStartedAt,
					},
				);
			} finally {
				prompting = false;
			}
		};

		const runCheck = async (trigger: CheckTrigger): Promise<Update | null> => {
			const checkStartedAt = Date.now();
			const source: CheckSource =
				trigger === "automatic" ? "automatic" : "manual";
			const checkSpan = startTelemetrySpan("desktop.updater.check", {
				kind: "client",
				attributes: { check_source: source, check_trigger: trigger },
			});
			try {
				const update = await check({ timeout: UPDATE_CHECK_TIMEOUT });
				checkSpan.end("ok", {
					update_available: update !== null,
					target_version: update?.version,
				});
				const effectiveTrigger = queuedCheckTrigger ?? trigger;
				const manualCheck = source === "manual" || manualCheckQueued;
				const effectiveSource: CheckSource = manualCheck ? "manual" : source;
				const installWhenFound = installQueued;
				const installTrigger = queuedInstallTrigger ?? effectiveTrigger;
				manualCheckQueued = false;
				queuedCheckTrigger = undefined;
				installQueued = false;
				queuedInstallTrigger = undefined;
				if (!active) {
					if (update) await closeUpdate(update);
					return null;
				}
				if (hasVisibleCheckError) {
					hasVisibleCheckError = false;
					toast.dismiss(UPDATE_TOAST_ID);
				}
				lastReportedCheckError = undefined;
				repeatedCheckFailureNoted = false;

				if (!update) {
					lastBreadcrumbUpdateVersion = undefined;
					if (effectiveSource === "manual") {
						addUpdaterBreadcrumb("manual_check_complete no_update");
					}
					pendingUpdate = null;
					void setTrayAvailability(false);
					if (manualCheck) {
						toast.success("Flow Like is up to date", {
							id: UPDATE_TOAST_ID,
						});
					}
					return null;
				}

				if (lastBreadcrumbUpdateVersion !== update.version) {
					lastBreadcrumbUpdateVersion = update.version;
					addUpdaterBreadcrumb(
						`update_available ${update.currentVersion} -> ${update.version}`,
					);
				}
				pendingUpdate = update;
				void setTrayAvailability(true);

				if (installWhenFound) {
					void installUpdate(update, installTrigger);
					return update;
				}

				if (!manualCheck && dismissedVersion() === update.version) {
					pendingUpdate = null;
					void closeUpdate(update);
					return update;
				}

				void promptForUpdate(update, effectiveTrigger);
				return update;
			} catch (error) {
				checkSpan.end("error");
				if (!active) return null;
				hasVisibleCheckError = true;
				const normalized = normalizeError(error);
				const checkErrorKey = `${normalized.kind}:${normalized.value}`;
				const effectiveTrigger = queuedCheckTrigger ?? trigger;
				const effectiveSource: CheckSource =
					effectiveTrigger === "automatic" ? "automatic" : "manual";
				const retryShouldInstall = installQueued;
				manualCheckQueued = false;
				queuedCheckTrigger = undefined;
				installQueued = false;
				queuedInstallTrigger = undefined;
				const retry = () => {
					void checkForUpdate("retry", retryShouldInstall);
				};
				if (
					navigator.onLine &&
					(effectiveTrigger !== "automatic" ||
						lastReportedCheckError !== checkErrorKey)
				) {
					lastReportedCheckError = checkErrorKey;
					repeatedCheckFailureNoted = false;
					showError(
						"Flow Like could not check for updates",
						"check",
						error,
						retry,
						{
							check_source: effectiveSource,
							check_trigger: effectiveTrigger,
							duration_ms: Date.now() - checkStartedAt,
						},
					);
				} else {
					if (!repeatedCheckFailureNoted) {
						repeatedCheckFailureNoted = true;
						addUpdaterBreadcrumb(
							navigator.onLine
								? "check_failure_repeated"
								: "check_failure_offline",
							"warning",
						);
					}
					console.warn(
						`Flow Like could not check for updates: ${normalized.value}`,
					);
					toast.error("Flow Like could not check for updates", {
						id: UPDATE_TOAST_ID,
						description: normalized.value,
						duration: Number.POSITIVE_INFINITY,
						closeButton: true,
						action: { label: "Retry", onClick: retry },
					});
				}
				return null;
			}
		};

		function checkForUpdate(
			trigger: CheckTrigger,
			installWhenFound = false,
		): Promise<Update | null> {
			if (!active || installing || prompting) {
				return Promise.resolve(pendingUpdate);
			}

			if (pendingUpdate) {
				if (installWhenFound) void installUpdate(pendingUpdate, trigger);
				return Promise.resolve(pendingUpdate);
			}

			if (trigger !== "automatic") {
				manualCheckQueued = true;
				queuedCheckTrigger = trigger;
				addUpdaterBreadcrumb(`${trigger}_check_started`);
				toast.loading("Checking for Flow Like updates…", {
					id: UPDATE_TOAST_ID,
					duration: Number.POSITIVE_INFINITY,
				});
			}
			if (installWhenFound) {
				installQueued = true;
				queuedInstallTrigger = trigger;
			}

			if (checking) return checking;
			checking = runCheck(trigger).finally(() => {
				checking = null;
			});
			return checking;
		}

		const registerTrayListener = () => {
			if (!active || unlistenTray || registeringTrayListener) return;
			registeringTrayListener = true;
			const listenerStartedAt = Date.now();
			void listen("tray:update-requested", () => {
				void checkForUpdate("tray", updateKnownAvailable);
			})
				.then((unlisten) => {
					registeringTrayListener = false;
					if (active) {
						unlistenTray = unlisten;
						toast.dismiss(UPDATE_MENU_TOAST_ID);
					} else unlisten();
				})
				.catch((error) => {
					registeringTrayListener = false;
					if (!active) return;
					showError(
						"Flow Like could not connect the update menu",
						"tray_listener",
						error,
						() => {
							registerTrayListener();
						},
						{ duration_ms: Date.now() - listenerStartedAt },
						UPDATE_MENU_TOAST_ID,
					);
				});
		};

		registerTrayListener();
		void reportInterruptedUpdate().finally(() => {
			if (!active) return;
			void checkForUpdate("automatic");
		});
		const intervalId = window.setInterval(() => {
			void checkForUpdate("automatic");
		}, UPDATE_CHECK_INTERVAL);

		return () => {
			active = false;
			window.clearInterval(intervalId);
			unlistenTray?.();
			if (pendingUpdate && !installing) {
				const update = pendingUpdate;
				pendingUpdate = null;
				void closeUpdate(update);
			}
		};
	}, []);

	return null;
}
