"use client";

import { useTranslation } from "@flow-like/locales";
import { useEffect, useId, useRef, useState } from "react";
import type { ServiceTunnelListener } from "../../../../lib/device-management/tunnel-services";
import {
	type RuntimeSession,
	openRuntimeSession,
} from "../../../../lib/service-runtime/session";
import { ServiceRequestError } from "../../../../lib/service-runtime/transport";
import { DvButton } from "../primitives/dv-button";
import { DvInput } from "../primitives/form-fields";
import { InlineResult } from "../primitives/inline-result";
import { useAttentionState } from "../workspace";
import { RuntimeView } from "./runtime-view";

export default function DeviceRuntime({
	deviceId,
	placementId,
	listener,
}: {
	deviceId: string;
	placementId: string;
	listener: ServiceTunnelListener;
}) {
	const { t } = useTranslation("devices");
	const tokenId = useId();
	const { workspace } = useAttentionState();
	const [session, setSession] = useState<RuntimeSession>();
	const credential = useRef<string | null>(null);
	const [attempt, setAttempt] = useState(0);
	const [input, setInput] = useState("");
	const [busy, setBusy] = useState(true);
	const [needsToken, setNeedsToken] = useState(false);
	const [error, setError] = useState("");
	// biome-ignore lint/correctness/useExhaustiveDependencies: An explicit retry starts a fresh session.
	useEffect(() => {
		const controller = new AbortController();
		let opened: RuntimeSession | undefined;
		let mounted = true;
		let release: (() => void) | undefined;
		const releaseDemand = () => {
			release?.();
			release = undefined;
		};
		setBusy(true);
		setSession(undefined);
		setError("");
		setNeedsToken(false);
		const stop = (reason: string) => {
			credential.current = null;
			controller.abort(new Error(reason));
			opened?.close();
			releaseDemand();
			if (mounted) {
				setInput("");
				setSession(undefined);
				setBusy(false);
				setError(reason);
			}
		};
		const offKeys = workspace.keys.subscribe(() => {
			if (workspace.keys.snapshot(deviceId).state !== "unlocked")
				stop(
					t(
						"serviceConfig.runtime.locked",
						"Device keys are locked. Unlock the device and reopen the app.",
					),
				);
		});
		let connected = workspace.live.state(deviceId).kind === "live";
		const offLive = workspace.live.subscribe(() => {
			const state = workspace.live.state(deviceId).kind;
			if (state === "live") connected = true;
			if (
				["failed", "unreachable", "reconnecting"].includes(state) ||
				(connected && state === "idle")
			)
				stop(
					t(
						"serviceConfig.runtime.disconnected",
						"The device disconnected. Reopen the app to start a new session.",
					),
				);
		});
		void (async () => {
			try {
				if (workspace.keys.snapshot(deviceId).state !== "unlocked")
					throw new Error(
						t(
							"serviceConfig.runtime.locked",
							"Device keys are locked. Unlock the device and reopen the app.",
						),
					);
				release = workspace.live.acquire(deviceId, "stream");
				const server = listener.tls_server_name ?? listener.host;
				const opening = openRuntimeSession(
					{
						open: (signal) =>
							workspace.live.openService(deviceId, placementId, listener.id, {
								mode: "http",
								signal,
							}),
						authority: `${server.includes(":") && !server.startsWith("[") ? `[${server}]` : server}:${listener.port}`,
						signal: controller.signal,
					},
					credential.current,
				);
				credential.current = null;
				opened = await opening;
				if (!mounted || controller.signal.aborted) {
					opened.close();
					return;
				}
				setInput("");
				setSession(opened);
			} catch (cause) {
				if (!mounted || controller.signal.aborted) return;
				setInput("");
				setNeedsToken(
					cause instanceof ServiceRequestError && cause.status === 401,
				);
				setError(cause instanceof Error ? cause.message : String(cause));
			} finally {
				credential.current = null;
				if (!opened || controller.signal.aborted) releaseDemand();
				if (mounted) setBusy(false);
			}
		})();
		return () => {
			mounted = false;
			offKeys();
			offLive();
			controller.abort();
			opened?.close();
			releaseDemand();
		};
	}, [
		workspace,
		deviceId,
		placementId,
		listener.id,
		listener.host,
		listener.port,
		listener.tls_server_name,
		attempt,
		t,
	]);
	if (session) return <RuntimeView key={session.appId} session={session} />;
	return (
		<div className="mx-auto flex w-full max-w-md flex-col gap-4 p-6">
			{busy ? (
				<output>
					{t("serviceConfig.runtime.opening", "Opening deployed app…")}
				</output>
			) : (
				<>
					{error && <InlineResult tone="critical">{error}</InlineResult>}
					<form
						className="flex flex-col gap-4"
						onSubmit={(event) => {
							event.preventDefault();
							credential.current = needsToken ? input : null;
							setAttempt((attempt) => attempt + 1);
						}}
					>
						{needsToken && (
							<>
								<p className="text-sm text-muted-foreground">
									{t(
										"serviceConfig.runtime.tokenHint",
										"Enter the service access token supplied by the device owner. It stays in this session until you close the app or lock the device.",
									)}
								</p>
								<label htmlFor={tokenId} className="flex flex-col gap-2">
									<span>
										{t("serviceConfig.runtime.token", "Service access token")}
									</span>
									<DvInput
										id={tokenId}
										type="password"
										autoComplete="off"
										maxLength={4096}
										value={input}
										onChange={(event) => setInput(event.target.value)}
									/>
								</label>
							</>
						)}
						<DvButton type="submit" disabled={needsToken && !input}>
							{t("serviceConfig.runtime.reopen", "Open app")}
						</DvButton>
					</form>
				</>
			)}
		</div>
	);
}
