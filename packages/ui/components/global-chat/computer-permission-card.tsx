"use client";

import { useTranslation } from "@flow-like/locales";
import { RotateCcwIcon, ShieldAlertIcon } from "lucide-react";
import { isTauri } from "../../lib/platform";
import {
	RequestCard,
	RequestCardFooter,
	RequestCardHeader,
	RequestMark,
	requestButtonClass,
} from "../interfaces/chat-default/request-card";
import { Button } from "../ui/button";
import {
	type ComputerPermissionNeed,
	requestComputerPermission,
} from "./computer-permission";

/**
 * A bot's computer tool needs a macOS permission. In the desktop app the button opens the same
 * permission dialog flow runs use; elsewhere the card says where to allow it.
 */
export function ComputerPermissionCard({
	need,
}: Readonly<{ need: ComputerPermissionNeed }>) {
	const { t } = useTranslation("bots");
	const desktop = isTauri();
	const screen = need.permission === "screen_recording";
	const permission = screen
		? t("computer.permission.screenRecording", "Screen Recording")
		: t("computer.permission.accessibility", "Accessibility");
	const title = need.restartRequired
		? t("computer.permission.restartTitle", "Quit and reopen Flow Like")
		: t("computer.permission.title", "Flow Like needs {{permission}} access", {
				permission,
			});
	const description = need.restartRequired
		? t(
				"computer.permission.restartHint",
				"macOS applies {{permission}} only after Flow Like starts again. Quit Flow Like, open it again, then ask the bot to try again.",
				{ permission },
			)
		: [
				screen
					? t(
							"computer.permission.screenReason",
							"Without it, the bot sees only the wallpaper, not the windows of other apps.",
						)
					: t(
							"computer.permission.accessibilityReason",
							"Without it, the bot can't use the mouse and keyboard or list windows.",
						),
				desktop
					? t(
							"computer.permission.afterAllow",
							"After you allow it, quit and reopen Flow Like.",
						)
					: t(
							"computer.permission.elsewhere",
							"Allow it in System Settings > Privacy & Security on the Mac that runs this bot, then quit and reopen Flow Like there.",
						),
			].join(" ");
	const canOpen = desktop && !need.restartRequired;
	return (
		<RequestCard
			aria-label={title}
			data-computer-permission={need.permission}
			data-restart-required={need.restartRequired || undefined}
			tone="warning"
			className="my-2 text-sm"
		>
			<RequestCardHeader
				dense
				mark={
					<RequestMark
						icon={need.restartRequired ? RotateCcwIcon : ShieldAlertIcon}
						tone="warning"
						size="sm"
					/>
				}
				title={title}
				description={description}
				className={canOpen ? undefined : "pb-3"}
			/>
			{canOpen && (
				<RequestCardFooter>
					<Button
						type="button"
						size="sm"
						className={requestButtonClass}
						onClick={() => requestComputerPermission(need.permission)}
					>
						{t("computer.permission.open", "Open permissions")}
					</Button>
				</RequestCardFooter>
			)}
		</RequestCard>
	);
}
