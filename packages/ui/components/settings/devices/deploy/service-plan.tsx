"use client";

import { useTranslation } from "@flow-like/locales";
import { Info } from "lucide-react";
import type {
	DeployPlan,
	PlanCheck,
	PlannedService,
	ServiceSplit,
	ServiceWhy,
} from "../../../../lib/device-management/model/deploy-plan";
import type { DevicesT } from "../primitives/area-context";
import { ChoiceCards, DvInput, Field } from "../primitives/form-fields";
import { eventName, issueText, planNames } from "./deploy-copy";

/* APP §3.5 item 5: which services the chosen events become. Forced splits and limits are stated, never silent. */

/** Why a service is limited to one instance, or split from another: the plan's reason as a sentence. */
export function serviceWhyText(
	t: DevicesT,
	plan: DeployPlan,
	why: ServiceWhy,
): string {
	if (why.code === "background")
		return t(
			"devices:deploy.services.whyRunsAlone",
			"{{event}} runs on its own, so this service runs 1 instance. Only services with a Page, chat or Endpoint can run several.",
			{ event: eventName(plan, why.eventId) },
		);
	if (why.code === "scheduled")
		return t(
			"devices:deploy.services.whyScheduled",
			"{{event}} runs on a schedule, so this service runs 1 instance. Two instances would start every run twice.",
			{ event: eventName(plan, why.eventId) },
		);
	if (why.code === "bot")
		return t(
			"devices:deploy.services.whyBot",
			"{{event}} is a bot, so this service runs 1 instance. Two instances would answer every message twice.",
			{ event: eventName(plan, why.eventId) },
		);
	if (why.code === "on_demand")
		return t(
			"devices:deploy.services.whyOnDemand",
			"{{event}} is started by a person and nothing in this service is served on the web, so it runs 1 instance. More instances need a Page, chat or Endpoint.",
			{ event: eventName(plan, why.eventId) },
		);
	if (why.code === "writes")
		return t(
			"devices:deploy.services.whyWrites",
			"Write buffering is on, so this service runs 1 instance.",
		);
	return t(
		"devices:deploy.services.whySplit",
		"{{first}} and {{second}} disagree about {{variable}}, so {{second}} gets its own service.",
		{
			first: eventName(plan, why.events[0]),
			second: eventName(plan, why.events[1]),
			variable: why.variable,
		},
	);
}

/** "{id} is taken on {device}; this one becomes {id}-2" per target that had to rename. */
function renameNotes(
	t: DevicesT,
	plan: DeployPlan,
	service: PlannedService,
): string[] {
	return plan.targets.flatMap((target) =>
		target.services
			.filter((row) => row.key === service.key && row.renamedFrom)
			.map((row) =>
				t(
					"devices:deploy.services.renamed",
					"{{from}} is taken on {{device}}; this one becomes {{to}}.",
					{ from: row.renamedFrom, device: target.name, to: row.serviceId },
				),
			),
	);
}

function ServiceRow({
	plan,
	check,
	service,
	single,
	onRename,
}: Readonly<{
	plan: DeployPlan;
	check: PlanCheck;
	service: PlannedService;
	/** One event makes one service: only its ID is asked, with what the ID is for. */
	single: boolean;
	onRename(id: string): void;
}>) {
	const { t } = useTranslation("devices");
	const issue = check.issues.find(
		(row) => row.step !== "where" && row.serviceKey === service.key,
	);
	const notes = [
		...service.why.map((why) => serviceWhyText(t, plan, why)),
		...renameNotes(t, plan, service),
	];
	return (
		<li
			data-service={service.key}
			className={
				single
					? "flex min-w-0 flex-col gap-2"
					: "grid grid-cols-[minmax(0,280px)_minmax(0,1fr)] gap-x-4 gap-y-2 rounded-lg border border-border px-3 py-3 @max-[560px]/svcplan:grid-cols-1"
			}
		>
			<Field
				id={`deploy-service-id-${service.key}`}
				label={t("devices:deploy.services.id", "Service ID")}
				error={issue ? issueText(t, issue, planNames(t, plan)) : undefined}
				hint={
					single
						? t(
								"devices:deploy.services.idHintSingle",
								"Can't be changed after deploy. It names the service on each device, and cloud approvals are tied to it. A device that already uses it gets a -2 suffix.",
							)
						: t(
								"devices:deploy.services.idHint",
								"Can't be changed after deploy",
							)
				}
			>
				<DvInput
					mono
					value={service.id}
					spellCheck={false}
					autoComplete="off"
					onChange={(event) => onRename(event.target.value)}
				/>
			</Field>
			{single ? null : (
				<div className="min-w-0">
					<p className="text-label font-semibold tracking-[0.06em] text-muted-foreground uppercase">
						{t("devices:deploy.services.events", "Events")}
					</p>
					<p className="mt-1.5 text-ui">
						{service.events
							.map((eventId) => eventName(plan, eventId))
							.join(" · ")}
					</p>
				</div>
			)}
			{notes.map((note) => (
				<p
					key={note}
					className="col-span-full flex items-start gap-1.5 text-xs text-ink-2"
				>
					<Info
						aria-hidden
						className="mt-0.5 size-3 shrink-0 text-muted-foreground"
					/>
					<span>{note}</span>
				</p>
			))}
		</li>
	);
}

export function ServicePlan({
	plan,
	check,
	showSplit,
	onSplit,
	onRename,
}: Readonly<{
	plan: DeployPlan;
	check: PlanCheck;
	/** The one / per-event choice; off when one event makes one service. */
	showSplit: boolean;
	onSplit(split: ServiceSplit): void;
	onRename(serviceKey: string, id: string): void;
}>) {
	const { t } = useTranslation("devices");
	const offline = plan.mode === "offline";
	return (
		<div className="@container/svcplan flex min-w-0 flex-col gap-3">
			{showSplit ? (
				<ChoiceCards<ServiceSplit>
					id="deploy-split"
					legend={t("devices:deploy.services.legend", "Services")}
					value={plan.draft.split}
					onValueChange={onSplit}
					className="[&_[role=radiogroup]]:grid-cols-2 @max-[560px]/svcplan:[&_[role=radiogroup]]:grid-cols-1"
					options={[
						{
							value: "one",
							title: t(
								"devices:deploy.services.one",
								"One service for these events",
							),
							hint: offline
								? t(
										"devices:deploy.services.oneHintOffline",
										"One endpoint and one set of settings per device.",
									)
								: t(
										"devices:deploy.services.oneHint",
										"One endpoint, one set of settings and one cloud access per device.",
									),
						},
						{
							value: "per_event",
							title: t(
								"devices:deploy.services.perEvent",
								"One service per event",
							),
							hint: offline
								? t(
										"devices:deploy.services.perEventHintOffline",
										"Each event gets its own service and settings.",
									)
								: t(
										"devices:deploy.services.perEventHint",
										"Each event gets its own service, settings and cloud access.",
									),
						},
					]}
				/>
			) : null}
			<ul className="flex flex-col gap-2">
				{plan.services.map((service) => (
					<ServiceRow
						key={service.key}
						plan={plan}
						check={check}
						service={service}
						single={!showSplit && plan.services.length === 1}
						onRename={(id) => onRename(service.key, id)}
					/>
				))}
			</ul>
		</div>
	);
}
