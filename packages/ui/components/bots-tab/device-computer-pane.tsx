"use client";

import { useTranslation } from "@flow-like/locales";
import { LockIcon, MonitorOffIcon } from "lucide-react";
import {
	type ReactNode,
	useCallback,
	useEffect,
	useRef,
	useState,
} from "react";
import type { PlaygroundControl } from "../../lib/bot-screens/playground-codec";
import type { ScreenStream } from "../../lib/bot-screens/screen-stream";
import { ScreenViewer } from "../../lib/bot-screens/screen-viewer";
import { holdForModelUse } from "../../lib/device-management/workspace/keys";
import type {
	IBot,
	IBotComputerView,
	IBotInputEvent,
} from "../../state/backend-state/bot-state";
import { AreaOverlays } from "../settings/devices/overlays/area-overlays";
import { ConfirmProvider } from "../settings/devices/primitives/confirm-sheet";
import { MemoryDevicesRoute } from "../settings/devices/routing/use-devices-route";
import {
	type DeviceWorkspaceOverrides,
	DeviceWorkspaceProvider,
	type WorkspaceGate,
	useDeviceWorkspace,
} from "../settings/devices/workspace/device-workspace-provider";
import { useOverlay } from "../settings/devices/workspace/overlay-store";
import { useKeySession } from "../settings/devices/workspace/use-keys";
import { Button } from "../ui/button";
import { ComputerStage, computerStageState } from "./computer-stage";

const RETRY_BASE_MS = 1000;
const RETRY_MAX_MS = 15_000;

export interface PaneViewer {
	readonly view: IBotComputerView;
	subscribe(listener: (view: IBotComputerView) => void): () => void;
	send(control: PlaygroundControl): Promise<void>;
	readonly ended: Promise<string>;
	close(): void;
}

export type PaneViewerFactory = (stream: ScreenStream) => PaneViewer;

const defaultViewer: PaneViewerFactory = (stream) => new ScreenViewer(stream);

const backoffMs = (attempt: number) =>
	Math.min(RETRY_BASE_MS * 2 ** attempt, RETRY_MAX_MS);

function pause(ms: number, signal: AbortSignal): Promise<void> {
	return new Promise((resolve) => {
		const timer = setTimeout(done, ms);
		function done() {
			clearTimeout(timer);
			signal.removeEventListener("abort", done);
			resolve();
		}
		signal.addEventListener("abort", done);
	});
}

function untilAborted(signal: AbortSignal): Promise<void> {
	return new Promise((resolve) => {
		if (signal.aborted) resolve();
		else signal.addEventListener("abort", () => resolve(), { once: true });
	});
}

function CalmStage({
	icon,
	title,
	hint,
	action,
}: Readonly<{
	icon: ReactNode;
	title: string;
	hint?: string;
	action?: ReactNode;
}>) {
	return (
		<div className="flex flex-1 flex-col items-center justify-center gap-3 px-6 py-10 text-center">
			<span className="flex size-11 items-center justify-center rounded-2xl bg-muted text-muted-foreground [&_svg]:size-5">
				{icon}
			</span>
			<p className="max-w-72 text-sm font-medium">{title}</p>
			{hint && <p className="max-w-72 text-xs text-muted-foreground">{hint}</p>}
			{action}
		</div>
	);
}

function GateStage({
	gate,
	name,
}: Readonly<{ gate: WorkspaceGate; name: string }>) {
	const { t } = useTranslation("bots");
	if (gate.kind === "loading")
		return (
			<CalmStage
				icon={<MonitorOffIcon aria-hidden />}
				title={t("computer.device.loading", "Loading your devices…")}
			/>
		);
	if (gate.kind === "signed_out")
		return (
			<CalmStage
				icon={<LockIcon aria-hidden />}
				title={t("computer.device.signIn", "Sign in to watch {{name}}", {
					name,
				})}
				hint={t(
					"computer.device.signInHint",
					"{{name}} runs on a device. Its screen opens only for the account that owns it.",
					{ name },
				)}
				action={
					gate.signIn && (
						<Button type="button" size="sm" onClick={gate.signIn}>
							{t("computer.device.signInButton", "Sign in")}
						</Button>
					)
				}
			/>
		);
	return (
		<CalmStage
			icon={<MonitorOffIcon aria-hidden />}
			title={t(
				"computer.device.accountFailed",
				"Your account couldn't be loaded",
			)}
			hint={t(
				"computer.device.accountFailedHint",
				"Check your connection and try again to watch {{name}}.",
				{ name },
			)}
			action={
				<Button type="button" size="sm" onClick={gate.retry}>
					{t("computer.device.retry", "Try again")}
				</Button>
			}
		/>
	);
}

interface LiveView {
	view: IBotComputerView | undefined;
	failed: boolean;
	send(control: PlaygroundControl): void;
	retry(): void;
}

/** Opens the bot's stream while the device is unlocked and reopens it with backoff until unmounted. */
function useBotStream(
	botId: string,
	deviceId: string,
	makeViewer: PaneViewerFactory,
): LiveView {
	const workspace = useDeviceWorkspace();
	const [view, setView] = useState<IBotComputerView | undefined>();
	const [failed, setFailed] = useState(false);
	const [nonce, setNonce] = useState(0);
	const viewer = useRef<PaneViewer | null>(null);
	const factory = useRef(makeViewer);
	factory.current = makeViewer;

	// biome-ignore lint/correctness/useExhaustiveDependencies: nonce restarts the loop after a manual retry
	useEffect(() => {
		const abort = new AbortController();
		const { signal } = abort;
		const { live, keys } = workspace;
		let attempt = 0;
		const watch = async () => {
			const stream = await live.openBotPlayground(deviceId, botId, { signal });
			const release = holdForModelUse(keys, deviceId);
			void stream.closed.then(release, release);
			const current = factory.current(stream);
			viewer.current = current;
			const stop = current.subscribe(setView);
			setView(current.view);
			setFailed(false);
			attempt = 0;
			await Promise.race([current.ended, untilAborted(signal)]);
			stop();
			current.close();
			viewer.current = null;
		};
		void (async () => {
			workspace.touch(deviceId);
			while (!signal.aborted) {
				try {
					await watch();
				} catch {
					if (signal.aborted) break;
					setFailed(true);
				}
				if (signal.aborted) break;
				await pause(backoffMs(attempt), signal);
				attempt += 1;
			}
		})();
		return () => {
			abort.abort();
			viewer.current?.close();
			viewer.current = null;
		};
	}, [workspace, botId, deviceId, nonce]);

	const send = useCallback((control: PlaygroundControl) => {
		void viewer.current?.send(control).catch(() => {});
	}, []);
	const retry = useCallback(() => {
		setFailed(false);
		setNonce((value) => value + 1);
	}, []);
	return { view, failed, send, retry };
}

function LiveComputer({
	botId,
	deviceId,
	name,
	makeViewer,
}: Readonly<{
	botId: string;
	deviceId: string;
	name: string;
	makeViewer: PaneViewerFactory;
}>) {
	const { view, failed, send, retry } = useBotStream(
		botId,
		deviceId,
		makeViewer,
	);
	const onInput = useCallback(
		(event: IBotInputEvent) => send({ type: "input", event }),
		[send],
	);
	return (
		<ComputerStage
			state={computerStageState(view, failed)}
			view={view}
			name={name}
			onStop={() => send({ type: "stop" })}
			onRetry={retry}
			onTakeOver={() => send({ type: "take_over" })}
			onHandBack={() => send({ type: "hand_back" })}
			onInput={onInput}
		/>
	);
}

function DeviceComputerBody({
	botId,
	deviceId,
	name,
	makeViewer,
}: Readonly<{
	botId: string;
	deviceId: string;
	name: string;
	makeViewer: PaneViewerFactory;
}>) {
	const { t } = useTranslation("bots");
	const overlay = useOverlay();
	const session = useKeySession(deviceId);
	const device = t("computer.device.thisDevice", "this device");
	if (session.state === "unlocked")
		return (
			<LiveComputer
				botId={botId}
				deviceId={deviceId}
				name={name}
				makeViewer={makeViewer}
			/>
		);
	if (session.state === "unlocking")
		return (
			<CalmStage
				icon={<LockIcon aria-hidden />}
				title={t("computer.device.connecting", "Connecting to {{device}}…", {
					device,
				})}
			/>
		);
	if (session.state === "none" || session.state === "stale")
		return (
			<CalmStage
				icon={<LockIcon aria-hidden />}
				title={t("computer.device.noKeys", "No keys for {{device}} here yet", {
					device,
				})}
				hint={t(
					"computer.device.noKeysHint",
					"Restore the device's keys in Devices to watch {{name}} here.",
					{ name },
				)}
			/>
		);
	return (
		<CalmStage
			icon={<LockIcon aria-hidden />}
			title={t(
				"computer.device.locked",
				"Unlock {{device}} to watch {{name}}",
				{
					device,
					name,
				},
			)}
			hint={t(
				"computer.device.lockedHint",
				"{{name}}'s screen is end-to-end encrypted. Unlocking {{device}} opens it here.",
				{ name, device },
			)}
			action={
				<Button
					type="button"
					size="sm"
					onClick={() => overlay.openUnlock(deviceId, { connectLive: true })}
				>
					{t("computer.device.unlock", "Unlock…")}
				</Button>
			}
		/>
	);
}

/** A device bot's Computer tab for the web and phone apps: unlock the device here, then watch over its encrypted tunnel. */
export function DeviceComputerPane({
	bot,
	name,
	deviceId,
	overrides,
	makeViewer = defaultViewer,
}: Readonly<{
	bot: Pick<IBot, "id">;
	name: string;
	deviceId: string;
	overrides?: DeviceWorkspaceOverrides;
	makeViewer?: PaneViewerFactory;
}>) {
	const renderGate = useCallback(
		(gate: WorkspaceGate) => <GateStage gate={gate} name={name} />,
		[name],
	);
	return (
		<MemoryDevicesRoute>
			<DeviceWorkspaceProvider
				passive
				overrides={overrides}
				renderGate={renderGate}
			>
				<ConfirmProvider>
					<DeviceComputerBody
						botId={bot.id}
						deviceId={deviceId}
						name={name}
						makeViewer={makeViewer}
					/>
					<AreaOverlays />
				</ConfirmProvider>
			</DeviceWorkspaceProvider>
		</MemoryDevicesRoute>
	);
}
