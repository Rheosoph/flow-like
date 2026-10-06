"use client";

import { useTranslation } from "@flow-like/locales";
import { useId } from "react";
import { DvButton } from "../../../settings/devices/primitives/dv-button";
import type {
	FormSessionActions,
	FormSessionState,
	NavigateIntent,
} from "../contracts";
import { InterfaceModal } from "./interface-modal";
import { leaveBody, leaveCountsOf } from "./leave";

export interface LeaveDialogProps {
	readonly state: Pick<FormSessionState, "view" | "runs" | "rail">;
	readonly actions: Pick<FormSessionActions, "closeOverlay">;
	readonly navigate: (intent: NavigateIntent) => void;
}

/** "Leave this form?" before a route button leaves while runs or next files wait (spec M5). Esc and Stay stay. */
export function LeaveDialog({
	state,
	actions,
	navigate,
}: Readonly<LeaveDialogProps>) {
	const { t } = useTranslation("interfaces");
	const titleId = useId();
	const overlay = state.view.overlay;
	const body = leaveBody(t, leaveCountsOf(state));

	const stay = () => actions.closeOverlay();
	const leave = () => {
		if (overlay?.id !== "leave") return;
		actions.closeOverlay();
		navigate({ route: overlay.route, replace: overlay.replace });
	};

	return (
		<InterfaceModal
			open={overlay?.id === "leave"}
			onClose={stay}
			labelledBy={titleId}
			width={400}
			placement="center"
		>
			<div className="flex flex-col gap-2 px-6 pt-5">
				<h2 id={titleId} className="text-[15px] font-semibold leading-5">
					{t("workbench.shell.leave.title", "Leave this form?")}
				</h2>
				{body ? (
					<p className="text-[13.5px] text-ink-2 leading-5">{body}</p>
				) : null}
			</div>
			<div className="flex justify-end gap-2 px-6 pt-5 pb-5">
				<DvButton variant="default" className="h-9 px-4" onClick={leave}>
					{t("workbench.shell.leave.leave", "Leave")}
				</DvButton>
				<DvButton
					variant="primary"
					className="h-9 px-4"
					data-autofocus=""
					onClick={stay}
				>
					{t("workbench.shell.leave.stay", "Stay")}
				</DvButton>
			</div>
		</InterfaceModal>
	);
}
