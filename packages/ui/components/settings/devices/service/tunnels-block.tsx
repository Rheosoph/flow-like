"use client";

import { useTranslation } from "@flow-like/locales";
import { Cable, Copy, Play, Square } from "lucide-react";
import {
	Suspense,
	lazy,
	useCallback,
	useEffect,
	useId,
	useRef,
	useState,
	useSyncExternalStore,
} from "react";
import {
	type DevicePortBridge,
	type DevicePortForward,
	startDevicePortForward,
} from "../../../../lib/device-management/tunnel-forward";
import { tunnelHttpRequest } from "../../../../lib/device-management/tunnel-http";
import {
	type ServiceTunnelListener,
	configuredTunnelServices,
	parseTunnelHeaders,
	readServiceListeners,
} from "../../../../lib/device-management/tunnel-services";
import { Block } from "../primitives/block";
import { DvButton } from "../primitives/dv-button";
import { DvSheet } from "../primitives/dv-sheet";
import {
	DvInput,
	DvSelect,
	DvTextarea,
	Field,
} from "../primitives/form-fields";
import { InlineResult } from "../primitives/inline-result";
import { useCopy } from "../primitives/use-copy";
import { useAttentionState } from "../workspace";
import type { SettingsEditor } from "./settings-sheets";
import { TunnelListeners } from "./tunnel-listeners";

const DISPLAY_LIMIT = 1024 * 1024;
const DeviceRuntime = lazy(() => import("./device-runtime"));
const message = (error: unknown) =>
	error instanceof Error ? error.message : String(error);

/** Locking, leaving this page, or losing the session ends every local operation. */
function useTunnelLifetime(deviceId: string, clear: () => void) {
	const { workspace } = useAttentionState();
	const controllers = useRef(new Set<AbortController>());
	const clearRef = useRef(clear);
	clearRef.current = clear;
	const mounted = useRef(true);
	const generation = useRef(0);
	useEffect(() => {
		mounted.current = true;
		const stop = (reason: string) => {
			for (const controller of controllers.current)
				controller.abort(new Error(reason));
			controllers.current.clear();
		};
		const offKeys = workspace.keys.subscribe(() => {
			if (workspace.keys.snapshot(deviceId).state !== "unlocked") {
				generation.current++;
				stop("Device keys are locked.");
				clearRef.current();
			}
		});
		let connected = workspace.live.state(deviceId).kind === "live";
		const offLive = workspace.live.subscribe(() => {
			const state = workspace.live.state(deviceId).kind;
			if (state === "live") connected = true;
			if (
				["failed", "unreachable", "reconnecting"].includes(state) ||
				(connected && state === "idle")
			)
				stop("The device disconnected. Start a new connection to continue.");
		});
		return () => {
			mounted.current = false;
			generation.current++;
			offKeys();
			offLive();
			stop("The service page closed.");
		};
	}, [workspace, deviceId]);
	return {
		workspace,
		mounted,
		generation,
		begin() {
			if (workspace.keys.snapshot(deviceId).state !== "unlocked")
				throw new Error("Unlock the device before connecting to a service.");
			const controller = new AbortController();
			controllers.current.add(controller);
			return controller;
		},
		end(controller: AbortController) {
			controllers.current.delete(controller);
		},
	};
}

export function ServiceTunnels({
	editor,
	deviceId,
	serviceId,
	portBridge,
}: Readonly<{
	editor?: SettingsEditor;
	deviceId: string;
	serviceId: string;
	portBridge?: DevicePortBridge;
}>) {
	const { t } = useTranslation("devices");
	const id = useId();
	const { workspace } = useAttentionState();
	const keyState = useSyncExternalStore(
		workspace.keys.subscribe,
		() => workspace.keys.snapshot(deviceId).state,
		() => "locked",
	);
	const [discovered, setDiscovered] = useState<ServiceTunnelListener[]>([]);
	const [discoveryError, setDiscoveryError] = useState("");
	const [refresh, setRefresh] = useState(0);
	// biome-ignore lint/correctness/useExhaustiveDependencies: Refresh explicitly retries listener discovery.
	useEffect(() => {
		if (editor || keyState !== "unlocked") {
			setDiscovered([]);
			return;
		}
		const abort = new AbortController();
		setDiscoveryError("");
		void readServiceListeners(
			workspace.live.call(deviceId, { signal: abort.signal }),
			serviceId,
		).then(
			(services) => {
				if (!abort.signal.aborted) setDiscovered(services);
			},
			(error) => {
				if (!abort.signal.aborted) setDiscoveryError(message(error));
			},
		);
		return () => abort.abort();
	}, [workspace, deviceId, serviceId, editor, keyState, refresh]);
	const services: ServiceTunnelListener[] = editor
		? configuredTunnelServices(editor.configuration.config)
		: discovered;
	const [selected, setSelected] = useState("");
	const service =
		services.find((entry) => entry.id === selected) ?? services[0];
	const [method, setMethod] = useState("GET");
	const [path, setPath] = useState("/");
	const [headers, setHeaders] = useState("");
	const [body, setBody] = useState("");
	const [response, setResponse] = useState("");
	const [responseHead, setResponseHead] = useState("");
	const [truncated, setTruncated] = useState(false);
	const [busy, setBusy] = useState(false);
	const [error, setError] = useState("");
	const [port, setPort] = useState("0");
	const [forward, setForward] = useState<DevicePortForward>();
	const [forwardBusy, setForwardBusy] = useState(false);
	const [forwardError, setForwardError] = useState("");
	const [runtime, setRuntime] = useState<ServiceTunnelListener>();
	const request = useRef<AbortController | null>(null);
	const forwarding = useRef<AbortController | null>(null);
	const { copied, copy } = useCopy();
	const clear = useCallback(() => {
		setRuntime(undefined);
		setHeaders("");
		setBody("");
		setPath("/");
		setResponse("");
		setResponseHead("");
		setError("");
		setTruncated(false);
		setForwardError("");
	}, []);
	const life = useTunnelLifetime(deviceId, clear);
	const unlocked = () =>
		life.mounted.current &&
		life.workspace.keys.snapshot(deviceId).state === "unlocked";

	const send = async () => {
		if (!service || service.protocol === "tcp" || request.current) return;
		const generation = life.generation.current;
		const current = () => unlocked() && generation === life.generation.current;
		let controller: AbortController | undefined;
		let timer: ReturnType<typeof setTimeout> | undefined;
		const chunks: string[] = [];
		let characters = 0;
		let dropped = false;
		const append = (part: string) => {
			if (!part) return;
			const last = chunks.length - 1;
			if (last >= 0 && chunks[last].length < 4096) chunks[last] += part;
			else chunks.push(part);
			characters += part.length;
			while (characters > DISPLAY_LIMIT) {
				dropped = true;
				const first = chunks[0];
				const excess = characters - DISPLAY_LIMIT;
				if (first.length <= excess) {
					chunks.shift();
					characters -= first.length;
				} else {
					chunks[0] = first.slice(excess);
					characters -= excess;
				}
			}
		};
		let result: Awaited<ReturnType<typeof tunnelHttpRequest>> | undefined;
		const publish = () => {
			timer = undefined;
			if (current() && !controller?.signal.aborted) {
				setResponse(chunks.join(""));
				setTruncated(dropped);
			}
		};
		try {
			controller = life.begin();
			request.current = controller;
			setBusy(true);
			setError("");
			setResponse("");
			setResponseHead("");
			setTruncated(false);
			result = await tunnelHttpRequest(
				(signal) =>
					life.workspace.live.openService(deviceId, serviceId, service.id, {
						mode: "http",
						signal,
					}),
				{
					method,
					path,
					authority: service.tls_server_name?.includes(":")
						? `[${service.tls_server_name}]`
						: (service.tls_server_name ?? "localhost"),
					headers: parseTunnelHeaders(headers),
					body: body || undefined,
					signal: controller.signal,
				},
			);
			if (!current() || controller.signal.aborted) return;
			setResponseHead(
				`${result.status} ${result.statusText}\n${Object.entries(result.headers)
					.map(([name, value]) => `${name}: ${value}`)
					.join("\n")}`,
			);
			const decoder = new TextDecoder();
			for await (const bytes of result.body) {
				controller.signal.throwIfAborted();
				append(decoder.decode(bytes, { stream: true }));
				if (!timer) timer = setTimeout(publish, 50);
			}
			append(decoder.decode());
			clearTimeout(timer);
			publish();
		} catch (cause) {
			if (current()) {
				setResponse(chunks.join(""));
				setTruncated(dropped);
				setError(message(cause));
			}
		} finally {
			clearTimeout(timer);
			result?.cancel();
			if (controller) life.end(controller);
			request.current = null;
			if (life.mounted.current) setBusy(false);
		}
	};

	const startForward = async () => {
		if (!service || forwarding.current) return;
		const generation = life.generation.current;
		const current = () => unlocked() && generation === life.generation.current;
		let controller: AbortController | undefined;
		let release: (() => void) | undefined;
		try {
			controller = life.begin();
			forwarding.current = controller;
			setForwardBusy(true);
			setForwardError("");
			if (!/^\d{1,5}$/.test(port) || Number(port) > 65535)
				throw new Error(
					"Choose a local port from 1 to 65535, or 0 for an available port.",
				);
			release = life.workspace.live.acquire(deviceId, "stream");
			const signal = controller.signal;
			// Check permission and listener reachability before publishing a local port.
			const probe = await life.workspace.live.openService(
				deviceId,
				serviceId,
				service.id,
				{ mode: "tcp", signal },
			);
			probe.reset();
			signal.throwIfAborted();
			const handle = await startDevicePortForward(
				{
					port: Number(port),
					signal,
					target: { deviceId, placementId: serviceId, serviceId: service.id },
				},
				portBridge,
			);
			if (!current() || signal.aborted) handle.close();
			else setForward(handle);
			if (life.mounted.current) setForwardBusy(false);
			await handle.closed;
			if (current() && signal.aborted) setForwardError(message(signal.reason));
		} catch (cause) {
			if (current()) setForwardError(message(cause));
		} finally {
			release?.();
			if (controller) life.end(controller);
			forwarding.current = null;
			if (life.mounted.current) {
				setForward(undefined);
				setForwardBusy(false);
			}
		}
	};

	return (
		<>
			{runtime && (
				<DvSheet
					open
					onOpenChange={(open) => {
						if (!open) setRuntime(undefined);
					}}
					title={t("serviceConfig.runtime.title", "Deployed app")}
					sub={t(
						"serviceConfig.runtime.sessionHint",
						"Closing this app clears this session's conversations and Page data.",
					)}
					closeOnOutside={false}
					className="flex h-[90dvh] w-[calc(100vw-32px)] flex-col sm:max-w-[1400px]"
					bodyClassName="overflow-hidden p-0 gap-0"
				>
					<Suspense
						fallback={
							<p className="p-6">
								{t("serviceConfig.runtime.loading", "Loading interface…")}
							</p>
						}
					>
						<DeviceRuntime
							deviceId={deviceId}
							placementId={serviceId}
							listener={runtime}
						/>
					</Suspense>
				</DvSheet>
			)}
			<Block
				icon={Cable}
				title={t("serviceConfig.tunnels.title", "Connect through Studio")}
			>
				<p className="text-xs text-muted-foreground">
					{t(
						"serviceConfig.tunnels.intro",
						"Traffic is encrypted between Studio and the device. The device checks your permission to connect to this deployment. Service authentication still applies.",
					)}
				</p>
				{services.length ? (
					<>
						<Field
							id={`${id}-service`}
							label={t("serviceConfig.tunnels.target", "Service listener")}
						>
							<DvSelect
								value={service?.id}
								onValueChange={setSelected}
								disabled={busy || forwardBusy || !!forward || !!runtime}
								options={services.map((entry) => ({
									value: entry.id,
									label: `${entry.id} (${entry.protocol.toUpperCase()} · ${entry.host}:${entry.port})`,
								}))}
							/>
						</Field>
						{service?.id === "hosting" && service.protocol !== "tcp" && (
							<div className="flex flex-wrap items-center gap-3">
								<DvButton
									disabled={keyState !== "unlocked"}
									onClick={() => setRuntime(service)}
								>
									{t("serviceConfig.runtime.open", "Open deployed app")}
								</DvButton>
								<p className="text-xs text-muted-foreground">
									{t(
										"serviceConfig.runtime.intro",
										"Use Pages, Chat, forms and quick actions through the encrypted tunnel.",
									)}
								</p>
							</div>
						)}
						{service?.protocol !== "tcp" ? (
							<>
								<div className="grid gap-3 sm:grid-cols-[8rem_1fr]">
									<Field
										id={`${id}-method`}
										label={t("serviceConfig.tunnels.method", "Method")}
									>
										<DvSelect
											value={method}
											onValueChange={setMethod}
											disabled={busy}
											options={[
												"GET",
												"POST",
												"PUT",
												"PATCH",
												"DELETE",
												"HEAD",
												"OPTIONS",
											].map((value) => ({ value, label: value }))}
										/>
									</Field>
									<Field
										id={`${id}-path`}
										label={t("serviceConfig.tunnels.path", "Request path")}
									>
										<DvInput
											mono
											value={path}
											disabled={busy}
											maxLength={8192}
											onChange={(event) => setPath(event.target.value)}
											placeholder="/api/example"
										/>
									</Field>
								</div>
								<Field
									id={`${id}-headers`}
									label={t("serviceConfig.tunnels.headers", "Request headers")}
									hint={t(
										"serviceConfig.tunnels.headersHint",
										"One Name: value per line. Include the service's access token if required. Credentials stay on this page until you leave or lock the device.",
									)}
								>
									<DvTextarea
										className="font-mono"
										value={headers}
										disabled={busy}
										autoComplete="off"
										spellCheck={false}
										maxLength={32768}
										onChange={(event) => setHeaders(event.target.value)}
										placeholder="Authorization: Bearer …"
										rows={2}
									/>
								</Field>
								<Field
									id={`${id}-body`}
									label={t("serviceConfig.tunnels.body", "Request body")}
								>
									<DvTextarea
										className="font-mono"
										value={body}
										disabled={busy}
										autoComplete="off"
										spellCheck={false}
										maxLength={8 * 1024 * 1024}
										onChange={(event) => setBody(event.target.value)}
										rows={3}
									/>
								</Field>
								<div className="flex gap-2">
									<DvButton icon={Play} busy={busy} onClick={() => void send()}>
										{t("serviceConfig.tunnels.send", "Send request")}
									</DvButton>
									{busy ? (
										<DvButton
											icon={Square}
											onClick={() =>
												request.current?.abort(new Error("Request stopped."))
											}
										>
											{t("serviceConfig.tunnels.stopRequest", "Stop request")}
										</DvButton>
									) : null}
								</div>
								{error ? (
									<InlineResult tone="critical">{error}</InlineResult>
								) : null}
								{responseHead ? (
									<pre
										data-tunnel-response-head=""
										className="max-h-40 overflow-auto rounded-lg bg-muted p-3 text-xs whitespace-pre-wrap break-all"
									>
										{responseHead}
									</pre>
								) : null}
								{responseHead ? (
									<pre
										data-tunnel-response=""
										aria-label={t(
											"serviceConfig.tunnels.response",
											"Response body",
										)}
										className="max-h-96 overflow-auto rounded-lg bg-muted p-3 text-xs whitespace-pre-wrap break-all"
									>
										{response}
									</pre>
								) : null}
								{truncated ? (
									<p className="text-xs text-muted-foreground">
										{t(
											"serviceConfig.tunnels.truncated",
											"Showing the most recent response text, up to 1,048,576 characters. The stream continues to be read.",
										)}
									</p>
								) : null}
							</>
						) : (
							<p className="text-xs text-muted-foreground">
								{t(
									"serviceConfig.tunnels.tcpHint",
									"Use Desktop Studio to connect an application to this TCP listener.",
								)}
							</p>
						)}
						{life.workspace.deps.platform === "desktop" ? (
							<div className="flex flex-col gap-3 border-t pt-3">
								<Field
									id={`${id}-port`}
									label={t("serviceConfig.tunnels.port", "Local port")}
									hint={t(
										"serviceConfig.tunnels.portHint",
										"Use 0 for an available port. Other applications on this computer can use it while this page is open.",
									)}
								>
									<DvInput
										type="number"
										min={0}
										max={65535}
										value={port}
										disabled={forwardBusy || !!forward}
										onChange={(event) => setPort(event.target.value)}
									/>
								</Field>
								<div className="flex flex-wrap items-center gap-2">
									{forward ? (
										<>
											<code data-tunnel-local-port="">
												{forward.host}:{forward.port}
											</code>
											<DvButton
												icon={Copy}
												onClick={() =>
													void copy(`${forward.host}:${forward.port}`)
												}
											>
												{copied
													? t("serviceConfig.tunnels.copied", "Copied address")
													: t("serviceConfig.tunnels.copy", "Copy address")}
											</DvButton>
											<DvButton icon={Square} onClick={() => forward.close()}>
												{t(
													"serviceConfig.tunnels.stopForward",
													"Close local port",
												)}
											</DvButton>
										</>
									) : (
										<DvButton
											icon={Cable}
											busy={forwardBusy}
											onClick={() => void startForward()}
										>
											{t(
												"serviceConfig.tunnels.startForward",
												"Open local port",
											)}
										</DvButton>
									)}
									{forwardBusy ? (
										<DvButton
											onClick={() =>
												forwarding.current?.abort(
													new Error("Local port cancelled."),
												)
											}
										>
											{t("common.cancel", "Cancel")}
										</DvButton>
									) : null}
								</div>
								{forwardError ? (
									<InlineResult tone="critical">{forwardError}</InlineResult>
								) : null}
							</div>
						) : (
							<p className="text-xs text-muted-foreground">
								{t(
									"serviceConfig.tunnels.desktopHint",
									"Desktop Studio can also expose a local port for your browser, database client, or another application.",
								)}
							</p>
						)}
					</>
				) : (
					<p className="text-xs text-muted-foreground">
						{t(
							"serviceConfig.tunnels.empty",
							"Enable hosting or add a listener below to connect to this deployment.",
						)}
					</p>
				)}
			</Block>
			{!editor ? (
				<div className="flex flex-col items-start gap-2">
					{discoveryError ? (
						<InlineResult tone="critical">{discoveryError}</InlineResult>
					) : null}
					<DvButton
						disabled={keyState !== "unlocked"}
						onClick={() => setRefresh((current) => current + 1)}
					>
						{t("serviceConfig.tunnels.refresh", "Refresh service listeners")}
					</DvButton>
				</div>
			) : (
				<TunnelListeners
					key={editor.configuration.config_revision}
					editor={editor}
				/>
			)}
		</>
	);
}
