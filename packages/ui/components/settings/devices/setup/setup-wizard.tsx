"use client";

import { useTranslation } from "@flow-like/locales";
import {
	type FormEvent,
	type ReactNode,
	type Ref,
	useMemo,
	useState,
} from "react";
import type { SetupRoute } from "../../../../lib/device-management/model/types";
import { prepareDevicePackage } from "../../../../lib/device-management/setup";
import { StateView } from "../primitives/state-view";
import { WizardStepper } from "../primitives/wizard";
import type { ScreenProps } from "../screen-props";
import { useDeviceWorkspace } from "../workspace";
import { LockedStepsBanner, ResumeBanner } from "./resume-banner";
import { SetupAside } from "./setup-aside";
import { SetupCancelled } from "./setup-cancelled";
import { SetupProvider } from "./setup-context";
import { SetupFoot } from "./setup-foot";
import { SetupHeader } from "./setup-header";
import { SetupHeadline } from "./setup-headline";
import { NEW_SLOT, readDrafts, waitingSetup } from "./setup-state";
import { CheckStep } from "./steps/check-step";
import { CreateStep } from "./steps/create-step";
import { NameStep } from "./steps/name-step";
import { PasswordStep } from "./steps/password-step";
import { PlatformStep } from "./steps/platform-step";
import { SaveStep } from "./steps/save-step";
import { StartStep } from "./steps/start-step";
import { WaitingStep } from "./steps/waiting-step";
import {
	type SetupFlowInput,
	useSetupController,
} from "./use-setup-controller";
import type { PrepareSetup } from "./use-setup-create";
import { useSetupSession } from "./use-setup-session";

/** Seams for tests and previews; the app never passes it. */
export interface SetupHarness {
	/** Replaces the call that makes the keys and builds the package (it needs the WASM crypto). */
	prepare?: PrepareSetup;
}

const STEPS: readonly ((props: {
	headingRef?: Ref<HTMLHeadingElement>;
}) => ReactNode)[] = [
	CheckStep,
	NameStep,
	PlatformStep,
	PasswordStep,
	CreateStep,
	SaveStep,
	StartStep,
	WaitingStep,
];

/** One setup, from the hub check to the first check-in: header, stepper, the step with its foot, and the side column. */
function SetupFlow(input: Readonly<SetupFlowInput>) {
	const { t } = useTranslation("devices");
	const { scopeKey } = useDeviceWorkspace();
	const { controller, steps, formRef, advance } = useSetupController(input);
	const { draft, step, labels, nowS, expired, leaveLink } = controller;
	const [resumeDismissed, setResumeDismissed] = useState(false);
	/* A setup this window made earlier and that still waits for its device (read once, when the wizard opens). */
	const [earlier] = useState(() =>
		input.slot === NEW_SLOT
			? waitingSetup(readDrafts(scopeKey), nowS)
			: undefined,
	);
	const waiting = draft.created || resumeDismissed ? undefined : earlier;
	const settled = expired || draft.checkedInAt !== undefined;
	const Step = STEPS[step] ?? CheckStep;

	const onSubmit = (event: FormEvent<HTMLFormElement>) => {
		event.preventDefault();
		advance();
	};

	return (
		<SetupProvider value={controller}>
			<div
				ref={steps.rootRef}
				data-setup-step={step}
				className="@container/wiz flex max-w-[1124px] min-w-0 scroll-mt-16 flex-col gap-4"
			>
				{waiting ? (
					<ResumeBanner
						setup={waiting}
						link={leaveLink({
							screen: "setup",
							enrollmentId: waiting.enrollmentId,
							step: waiting.draft.step,
						})}
						onDismiss={() => setResumeDismissed(true)}
					/>
				) : null}
				{steps.lockedNote && !settled ? (
					<LockedStepsBanner
						name={draft.name}
						onDismiss={steps.dismissLockedNote}
					/>
				) : null}
				<SetupHeader />
				<SetupHeadline />
				<WizardStepper
					steps={labels}
					current={step}
					label={t("setup.steps.label", "Set up steps")}
				/>
				<div className="grid grid-cols-1 items-start gap-x-6 gap-y-4 @[1000px]/wiz:grid-cols-[minmax(0,760px)_minmax(300px,340px)]">
					<form
						ref={formRef}
						noValidate
						onSubmit={onSubmit}
						className="@container/setup flex min-w-0 flex-col gap-4"
					>
						<section className="flex min-w-0 flex-col gap-4">
							{draft.cancelledAt ? (
								<SetupCancelled headingRef={steps.headingRef} />
							) : (
								<Step headingRef={steps.headingRef} />
							)}
						</section>
						<SetupFoot />
					</form>
					<aside
						aria-label={t("setup.aside.title", "This setup")}
						className="flex min-w-0 flex-col gap-4"
					>
						<SetupAside />
					</aside>
				</div>
			</div>
		</SetupProvider>
	);
}

/** SPEC §5.5 / IA N5: create, deliver and confirm a new device, resumable from the waiting step. */
export function SetupWizard({
	route,
	harness,
}: Readonly<ScreenProps & { harness?: SetupHarness }>) {
	const { t } = useTranslation("devices");
	const setupRoute = useMemo<SetupRoute>(
		() => (route.screen === "setup" ? route : { screen: "setup" }),
		[route],
	);
	const session = useSetupSession(setupRoute);
	if (!session.draft)
		return (
			<StateView
				kind="loading"
				title={t("setup.loading", "Reading this setup…")}
			/>
		);
	return (
		<SetupFlow
			key={session.key}
			route={setupRoute}
			draft={session.draft}
			slot={session.slot}
			prepare={harness?.prepare ?? prepareDevicePackage}
			update={session.update}
			startNew={session.startNew}
		/>
	);
}
