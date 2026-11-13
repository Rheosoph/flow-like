"use client";

import { useTranslation } from "@flow-like/locales";
import { useId } from "react";
import { useAccountDevices } from "../../../../hooks/use-account-devices";
import { botDockerState } from "../../../../lib/device-bot-docker";
import { cn } from "../../../../lib/utils";
import type {
	BotExecutionMode,
	BotLocation,
} from "../../../../state/backend-state/bot-state";
import { Button } from "../../../ui/button";
import { RadioGroup, RadioGroupItem } from "../../../ui/radio-group";

export function ExecutionModePicker({
	location,
	deviceId,
	value,
	onChange,
}: Readonly<{
	location: BotLocation;
	deviceId: string | null;
	value: BotExecutionMode;
	onChange: (value: BotExecutionMode) => void;
}>) {
	const { t } = useTranslation("bots");
	const id = useId();
	const { devices, isError, isFetching, refetch } = useAccountDevices(
		location === "device",
		true,
	);
	const state = botDockerState(
		devices?.find((device) => device.device_id === deviceId),
	);
	const ready = location === "device" && !isError && state === "ready";
	const messages = {
		ready: t("editor.docker.ready", "Docker is ready on this device."),
		choose_device: t(
			"editor.docker.chooseDevice",
			"Choose a device to check Docker availability.",
		),
		unknown: t(
			"editor.docker.unknown",
			"Update and start the device agent to check Docker availability.",
		),
		offline: t(
			"editor.docker.offline",
			"The device is offline or its Docker check has expired. Start the agent and check again.",
		),
		not_installed: t(
			"editor.docker.notInstalled",
			"Install Docker on the selected device, start Docker, then check again.",
		),
		unreachable: t(
			"editor.docker.unreachable",
			"Docker is installed, but the agent cannot connect. Start Docker and allow the agent to use it.",
		),
		remote_endpoint: t(
			"editor.docker.remoteEndpoint",
			"Select a local Docker connection on the device so the bot runs on this machine.",
		),
		linux_required: t(
			"editor.docker.linuxRequired",
			"Switch Docker to Linux containers on the device.",
		),
		image_unavailable: t(
			"editor.docker.imageUnavailable",
			"Update the device agent to a release that includes our prebuilt bot image for this machine.",
		),
		hub_unreachable: t(
			"editor.docker.hubUnreachable",
			"Configure the device agent with a hub address reachable from Docker. A localhost address only works on the host.",
		),
	};
	return (
		<div className="flex flex-col gap-3" data-bot-docker-state={state}>
			<RadioGroup
				value={value}
				onValueChange={(next) => {
					if (next === "native" || (next === "docker" && ready)) onChange(next);
				}}
				aria-label={t("editor.docker.title", "Execution environment")}
				className="grid gap-3 @xl:grid-cols-2"
			>
				<label
					htmlFor={`${id}-native`}
					className="flex cursor-pointer items-start gap-3 rounded-lg border p-3"
				>
					<RadioGroupItem
						id={`${id}-native`}
						value="native"
						className="mt-0.5"
					/>
					<span className="flex flex-col gap-1">
						<span className="text-sm font-medium">
							{t("editor.docker.native", "Run directly on the host")}
						</span>
						<span className="text-xs text-muted-foreground">
							{t(
								"editor.docker.nativeDescription",
								"Uses the device agent's own environment.",
							)}
						</span>
					</span>
				</label>
				<label
					htmlFor={`${id}-docker`}
					className={cn(
						"flex items-start gap-3 rounded-lg border p-3",
						ready
							? "cursor-pointer"
							: "cursor-not-allowed bg-muted/30 text-muted-foreground",
					)}
				>
					<RadioGroupItem
						id={`${id}-docker`}
						value="docker"
						disabled={!ready}
						aria-describedby={`${id}-status`}
						className="mt-0.5"
					/>
					<span className="flex flex-col gap-1">
						<span className="text-sm font-medium">
							{t("editor.docker.docker", "Run in Docker")}
						</span>
						<span className="text-xs text-muted-foreground">
							{t(
								"editor.docker.description",
								"A separate container for this bot, with its own browser, desktop and shell. Flow-Like pulls our prebuilt image and manages the container.",
							)}
						</span>
					</span>
				</label>
			</RadioGroup>
			<output id={`${id}-status`} className="text-xs text-muted-foreground">
				{location !== "device"
					? t(
							"editor.docker.deviceOnly",
							"Docker hosting is currently available for managed-device bots. Choose a device, or switch back to running directly on the host.",
						)
					: isError
						? t(
								"editor.docker.checkFailed",
								"The device's Docker status could not be checked. Try again.",
							)
						: messages[state]}
			</output>
			{location === "device" && deviceId && !ready && (
				<div className="flex flex-wrap items-center gap-3 text-xs">
					<a
						href="https://docs.docker.com/get-started/get-docker/"
						target="_blank"
						rel="noopener noreferrer"
						className="text-primary underline underline-offset-4"
					>
						{t("editor.docker.install", "Docker installation instructions")}
					</a>
					<Button
						type="button"
						size="sm"
						variant="ghost"
						disabled={isFetching}
						onClick={() => void refetch()}
					>
						{t("editor.docker.recheck", "Check again")}
					</Button>
				</div>
			)}
		</div>
	);
}
