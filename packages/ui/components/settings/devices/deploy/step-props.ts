import type { ReactNode } from "react";
import type { PreparedProjectArtifact } from "../../../../lib/device-management/artifacts";
import type { DeploymentVariable } from "../../../../lib/device-management/deployment";
import type { PreparedFlow } from "../../../../lib/device-management/latest-flows";
import type {
	DeployDraft,
	DeployPlan,
	DeployResult,
	PlanCheck,
} from "../../../../lib/device-management/model/deploy-plan";
import type {
	DeployRoute,
	DeployStepId,
	DevicesScope,
} from "../../../../lib/device-management/model/types";
import type { ApprovedOnlineMetadata } from "../../../../lib/device-management/online-metadata";
import type { DeployDraftState } from "./use-deploy-draft";
import type { DeployPrepareState } from "./use-deploy-prepare";

export type { DeployStepId } from "../../../../lib/device-management/model/types";

/** The bundle step 2 prepared; steps 6–8 upload and apply exactly this. */
export interface DeployPrepared {
	artifact: PreparedProjectArtifact;
	approved?: ApprovedOnlineMetadata;
	preparedAt: number;
	/** What preparing did for the flows of the events that follow Latest: created a version, or found one equal to the flow. */
	flows?: readonly PreparedFlow[];
	/** The flow version each of those events is shipped at, by event id. */
	latest?: Readonly<Record<string, [number, number, number]>>;
}

/** What a device said about the copy once it arrived (offline apps): events it refuses and the settings of the events it accepts. */
export interface DeployDeviceCheck {
	/** Event id → the device's reason. */
	refusals: Readonly<Record<string, string>>;
	/** Event id → variable definitions, as the device discovered them. */
	variables?: Readonly<Record<string, readonly DeploymentVariable[]>>;
}

/** Props of every deploy wizard step (plan §2.5). */
export interface DeployStepProps {
	/** Embedded deployment keeps the source route even when its host URL is unchanged. */
	route?: DeployRoute;
	scope: DevicesScope;
	draft: DeployDraft;
	/** Derived from the draft, per target. */
	plan: DeployPlan;
	/** Per-target errors and exceptions. */
	check: PlanCheck;
	update(patch: Partial<DeployDraft>): void;
	goTo(step: DeployStepId): void;
	onFinished(result: DeployResult): void;
	prepared?: DeployPrepared | null;
	/** The sentence for `check.firstBlocking`, already translated by the frame. */
	blockingText?: string;
	/** Step 6 (offline) hands each device's event check back; Where and Settings then show it. */
	reportDeviceCheck?(deviceId: string, check: DeployDeviceCheck): void;
	/** Rollout's "Start over…": the frame asks first, then forgets the choices and opens What. */
	startOver?(): void;
	/** Rollout: the frame's "This deploy" bar for narrow containers; the step places it above its foot. */
	summaryBar?: ReactNode;
	/** The host dialog's footer: Review portals its Deploy action there instead of rendering it in the page. */
	footerContainer?: HTMLElement | null;
	/** The step runs inside another dialog: it neither links away from it nor offers its own exit. */
	embedded?: boolean;
	/** Rollout's "Deploy to more devices…" when the frame restarts the flow itself. */
	deployMore?(): void;
}

/** Steps 1–5 also read the wizard's draft state and the preparation of step 2. */
export interface PlanStepProps extends DeployStepProps {
	route: DeployRoute;
	state: DeployDraftState;
	prepare: DeployPrepareState;
}
