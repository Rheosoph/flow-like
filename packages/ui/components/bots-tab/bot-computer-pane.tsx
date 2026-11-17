"use client";

import { useMutation, useQuery } from "@tanstack/react-query";
import { useCallback, useEffect, useState } from "react";
import { useBackend } from "../../state/backend-state";
import type {
	IBot,
	IBotComputerHost,
	IBotComputerUpdate,
	IBotInputEvent,
} from "../../state/backend-state/bot-state";
import { ComputerPairingStage } from "./computer-pairing";
import { ComputerPairingDialog } from "./computer-pairing-dialog";
import { ComputerStage, computerStageState } from "./computer-stage";
import { DeviceComputerPane } from "./device-computer-pane";

const ACTIVE_POLL_MS = 2000;
const ASLEEP_POLL_MS = 5000;
const CONTROL_POLL_MS = 600;

function useComputerPush(
	host: IBotComputerHost | undefined,
	botId: string,
	deviceId: string | null,
): IBotComputerUpdate | undefined {
	const [update, setUpdate] = useState<IBotComputerUpdate>();
	const watches = Boolean(host?.watch);
	useEffect(() => {
		setUpdate(undefined);
		return watches ? host?.watch?.(botId, deviceId, setUpdate) : undefined;
	}, [host, watches, botId, deviceId]);
	return update;
}

/**
 * The Computer tab: the latest picture of what the bot's browser or screen shows. A host that
 * pushes (a desktop bot watched from another app) drives it live; the rest are pulled every
 * couple of seconds, faster while the owner is in control. Device bots open in their own pane
 * unless the host reaches devices itself.
 */
export function BotComputerPane({
	bot,
	name,
}: Readonly<{ bot: IBot; name: string }>) {
	const host = useBackend().botState?.computer;
	const deviceId = bot.location === "device" ? bot.device_id : null;
	const pushed = Boolean(host?.watch);
	const update = useComputerPush(host, bot.id, deviceId);
	const query = useQuery({
		queryKey: ["bot-computer", bot.id, deviceId],
		queryFn: () =>
			host ? host.view(bot.id, deviceId) : Promise.resolve(undefined),
		enabled: Boolean(host) && !pushed,
		refetchInterval: (current) =>
			current.state.data?.taken_over
				? CONTROL_POLL_MS
				: current.state.data?.state === "active"
					? ACTIVE_POLL_MS
					: ASLEEP_POLL_MS,
		refetchIntervalInBackground: false,
	});
	const act = useMutation({
		mutationFn: async (run: () => Promise<void>) => {
			await run();
			if (!pushed) await query.refetch();
		},
	});
	const send = useCallback(
		(event: IBotInputEvent) => {
			void host?.input(bot.id, event, deviceId).catch(() => {});
		},
		[host, bot.id, deviceId],
	);
	if (deviceId && !host?.devices)
		return <DeviceComputerPane bot={bot} name={name} deviceId={deviceId} />;
	const view = pushed ? update?.view : query.data;
	const failed = pushed ? update?.status === "unreachable" : query.isError;
	const pairScreen = host?.pairScreen;
	const stage =
		pushed && update?.status === "unpaired" && pairScreen ? (
			<ComputerPairingStage
				name={name}
				hostLabel={update.host_label}
				detect={host?.scanBarcodes}
				onPair={(link) => pairScreen.call(host, bot.id, link)}
			/>
		) : (
			<ComputerStage
				state={computerStageState(view, failed)}
				view={view}
				name={name}
				stopping={act.isPending}
				onStop={() =>
					act.mutate(() => host?.stop(bot.id, deviceId) ?? Promise.resolve())
				}
				onRetry={() =>
					pushed ? host?.retry?.(bot.id, deviceId) : void query.refetch()
				}
				onTakeOver={() =>
					act.mutate(
						() => host?.takeOver(bot.id, deviceId) ?? Promise.resolve(),
					)
				}
				onHandBack={() =>
					act.mutate(
						() => host?.handBack(bot.id, deviceId) ?? Promise.resolve(),
					)
				}
				onInput={deviceId || pushed ? send : undefined}
			/>
		);
	if (!host?.pairing || deviceId) return stage;
	return (
		<>
			<div className="flex justify-end px-3 pt-3">
				<ComputerPairingDialog pairing={host.pairing} />
			</div>
			{stage}
		</>
	);
}
