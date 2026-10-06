"use client";

import { useTranslation } from "@flow-like/locales";
import { useId, useState } from "react";
import { InterfaceModal } from "../shell/interface-modal";
import { PresetSaveForm } from "./preset-save-form";
import { type SaveMode, saveStartOf } from "./preset-save-model";
import type { RailPartProps } from "./rail-model";
import { RailSheet } from "./rail-sheet";

const MODAL_WIDTH = 480;
/** The dialog sits 72 px below the top of the box, so it may take the box minus 96 px. */
const MODAL_MARGIN = 96;

/**
 * S1: the Save dialog. Open while `view.overlay` is `presetSave`: a modal inside the interface, a
 * full-height sheet on a touch screen.
 */
export function PresetSaveDialog(props: Readonly<RailPartProps>) {
	const overlay = props.state.view.overlay;
	if (overlay?.id !== "presetSave") return null;
	return (
		<PresetSaveHost
			{...props}
			mode={overlay.mode}
			fromRunId={overlay.fromRunId}
		/>
	);
}

function PresetSaveHost({
	state,
	actions,
	layout,
	mode,
	fromRunId,
}: Readonly<RailPartProps & { mode: SaveMode; fromRunId: string | null }>) {
	const { t } = useTranslation("interfaces");
	const titleId = useId();
	const [start] = useState(() => saveStartOf(state, mode, fromRunId));
	const title = start.updating
		? t("workbench.preset.save.titleUpdate", "Update {{name}}", {
				name: start.updating.name,
			})
		: t("workbench.preset.save.title", "Save inputs as a preset");
	const initialName =
		start.proposal.kind === "value"
			? start.proposal.text
			: t("workbench.preset.save.numbered", "Preset {{n}}", {
					n: start.proposal.n,
				});
	const form = (
		<PresetSaveForm
			state={state}
			actions={actions}
			layout={layout}
			start={start}
			fromRunId={fromRunId}
			initialName={initialName}
		/>
	);
	if (layout.touch)
		return (
			<RailSheet
				open
				onClose={() => actions.closeOverlay()}
				title={title}
				variant="full"
			>
				{form}
			</RailSheet>
		);
	return (
		<InterfaceModal
			open
			onClose={() => actions.closeOverlay()}
			labelledBy={titleId}
			width={MODAL_WIDTH}
		>
			<div
				className="flex min-h-0 flex-col"
				style={{ maxHeight: layout.height - MODAL_MARGIN }}
			>
				<header className="flex-none px-6 pt-5">
					<h2 id={titleId} className="m-0 text-[15px]/5 font-semibold">
						{title}
					</h2>
				</header>
				{form}
			</div>
		</InterfaceModal>
	);
}
