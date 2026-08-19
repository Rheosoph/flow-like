"use client";

import { useTranslation } from "@flow-like/locales";
import type { SurfaceComponent } from "../../a2ui/types";
import { FlowPilot } from "../../flowpilot";

export interface A2UICopilotProps {
	/** App owning this UI surface; used for hosted-model usage attribution. */
	appId?: string;
	currentComponents: SurfaceComponent[];
	selectedComponentIds: string[];
	onComponentsGenerated?: (components: SurfaceComponent[]) => void;
	onApplyComponents?: (
		components: SurfaceComponent[],
		canvasSettings?: {
			backgroundColor?: string;
			padding?: string;
			customCss?: string;
		},
	) => void;
	className?: string;
	onClose?: () => void;
	/** Custom function to capture a screenshot for context */
	captureScreenshot?: () => Promise<string | null>;
}

/**
 * A2UICopilot - AI assistant for UI generation
 *
 * This is a wrapper around the unified FlowPilot component
 * configured for UI mode (agentMode="ui")
 */
export function A2UICopilot({
	appId,
	currentComponents,
	selectedComponentIds,
	onComponentsGenerated,
	onApplyComponents,
	className,
	onClose,
	captureScreenshot,
}: A2UICopilotProps) {
	const { t } = useTranslation("flow");
	return (
		<FlowPilot
			agentMode="ui"
			appId={appId}
			title={t('flowpilot', 'FlowPilot')}
			className={className}
			onClose={onClose}
			currentComponents={currentComponents}
			selectedComponentIds={selectedComponentIds}
			onComponentsGenerated={onComponentsGenerated}
			onApplyComponents={onApplyComponents}
			captureScreenshot={captureScreenshot}
		/>
	);
}

export default A2UICopilot;
