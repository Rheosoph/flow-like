"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Ban,
	Check,
	Cloud,
	Cpu,
	KeyRound,
	type LucideIcon,
	Monitor,
	Package,
	ShieldCheck,
	Smartphone,
	Sparkles,
	Workflow,
	X,
} from "lucide-react";
import type { ReactNode } from "react";
import type {
	IShowcaseFlow,
	IShowcaseHood,
} from "../../../lib/schema/app/showcase";
import { cn } from "../../../lib/utils";
import { PERMISSION_LABELS } from "./model";
import { Chip, ShowcaseSection, cardClass } from "./showcase-ui";

export const averageScore = (f: IShowcaseFlow): number | null => {
	if (!f.scores) return null;
	const v = Object.values(f.scores);
	return v.length ? v.reduce((a, b) => a + b, 0) / v.length : null;
};

function HoodCard({
	icon: Icon,
	title,
	children,
}: Readonly<{ icon: LucideIcon; title: string; children: ReactNode }>) {
	return (
		<div className={`${cardClass} space-y-3`}>
			<h3 className="flex items-center gap-2 text-sm font-semibold">
				<Icon className="h-4 w-4 text-muted-foreground" aria-hidden="true" />
				{title}
			</h3>
			{children}
		</div>
	);
}

function RunOption({
	on,
	icon: Icon,
	label,
}: Readonly<{ on: boolean; icon: LucideIcon; label: string }>) {
	const { t } = useTranslation("store");
	return (
		<div
			className={cn(
				"flex items-center gap-2 rounded-md border p-2.5 text-[13px] font-medium",
				on
					? "border-border"
					: "border-dashed border-border text-muted-foreground",
			)}
		>
			<Icon className="h-4 w-4" aria-hidden="true" />
			<span>{label}</span>
			{on ? (
				<Check
					className="ml-auto h-4 w-4 text-[var(--showcase-time)]"
					aria-hidden="true"
				/>
			) : (
				<X className="ml-auto h-4 w-4" aria-hidden="true" />
			)}
			<span className="sr-only">
				{on ? t("showcaseYes", "Yes") : t("showcaseNo", "No")}
			</span>
		</div>
	);
}

function Policy({
	ok,
	yes,
	no,
}: Readonly<{ ok: boolean; yes: string; no: string }>) {
	const Icon = ok ? Check : Ban;
	return (
		<li className="flex items-center gap-2">
			<Icon
				className={cn(
					"h-4 w-4",
					ok ? "text-[var(--showcase-time)]" : "text-muted-foreground",
				)}
				aria-hidden="true"
			/>
			{ok ? yes : no}
		</li>
	);
}

export function ShowcaseHood({
	hood,
	flows,
	onOpenFlow,
}: Readonly<{
	hood: IShowcaseHood;
	flows: IShowcaseFlow[];
	onOpenFlow: (id: string) => void;
}>) {
	const { t } = useTranslation("store");
	const mode = hood.execution_mode;
	const cloud = mode === "Any" || mode === "Remote";
	const device = mode === "Any" || mode === "Local";
	const RunIcon =
		mode === "Remote" ? Cloud : mode === "Local" ? Smartphone : Monitor;
	const totalNodes = flows.reduce((a, f) => a + f.node_count, 0);

	return (
		<ShowcaseSection
			id="hood"
			icon={Cpu}
			eyebrow={t("showcaseHoodEyebrow", "Under the hood")}
			title={t("showcaseHoodTitle", "What it needs and what it can reach")}
		>
			<div className="grid grid-cols-[repeat(auto-fill,minmax(min(100%,300px),1fr))] items-start gap-3">
				{mode ? (
					<HoodCard
						icon={RunIcon}
						title={t("showcaseHoodRuns", "Where it runs")}
					>
						<div className="grid grid-cols-2 gap-2">
							<RunOption
								on={cloud}
								icon={Cloud}
								label={t("showcaseHoodCloud", "Cloud")}
							/>
							<RunOption
								on={device}
								icon={Smartphone}
								label={t("showcaseHoodDevice", "Your device")}
							/>
						</div>
						{hood.requires_local ? (
							<p className="text-xs text-muted-foreground">
								{t(
									"showcaseHoodRequiresLocal",
									"Some steps only run on your own device.",
								)}
							</p>
						) : null}
					</HoodCard>
				) : null}
				{hood.models && hood.models.length > 0 ? (
					<HoodCard
						icon={Sparkles}
						title={t("showcaseHoodModels", "AI models")}
					>
						<ul className="flex flex-col gap-2.5">
							{hood.models.map((m) => (
								<li key={m.id} className="flex items-start gap-2.5">
									<span className="min-w-0 flex-1 text-[13px] font-medium">
										{m.name}
									</span>
									<Chip>{m.kind}</Chip>
								</li>
							))}
						</ul>
					</HoodCard>
				) : null}
				{hood.packages && hood.packages.length > 0 ? (
					<HoodCard
						icon={Package}
						title={t("showcaseHoodPackages", "Node packages")}
					>
						<ul className="flex flex-col gap-3">
							{hood.packages.map((p) => (
								<li key={p.id} className="flex flex-col gap-1.5">
									<b className="font-mono text-[13px] font-medium">
										{p.name}{" "}
										<span className="text-muted-foreground">{p.version}</span>
									</b>
									{p.permissions.length > 0 ? (
										<>
											<small className="text-xs text-muted-foreground">
												{t(
													"showcaseHoodSandboxed",
													"Runs sandboxed. Allowed to use:",
												)}
											</small>
											<span className="flex flex-wrap gap-1.5">
												{p.permissions.map((perm) => {
													const label = PERMISSION_LABELS[perm];
													return (
														<Chip key={perm}>
															{label
																? t(label[0] as "showcasePermCache", label[1])
																: perm}
														</Chip>
													);
												})}
											</span>
										</>
									) : null}
								</li>
							))}
						</ul>
					</HoodCard>
				) : null}
				{hood.oauth ? (
					<HoodCard
						icon={KeyRound}
						title={t("showcaseHoodAccounts", "Accounts you connect")}
					>
						{hood.oauth.length > 0 ? (
							<ul className="flex flex-col gap-3">
								{hood.oauth.map((a) => (
									<li key={a.provider} className="flex flex-col gap-1.5">
										<b className="text-[13px] font-medium">{a.provider}</b>
										<span className="flex flex-wrap gap-1.5">
											{a.scopes.map((s) => (
												<Chip key={s} className="font-mono">
													{s}
												</Chip>
											))}
										</span>
									</li>
								))}
							</ul>
						) : (
							<p className="text-xs text-muted-foreground">
								{t(
									"showcaseHoodNoAccounts",
									"None. It works without signing in to other services.",
								)}
							</p>
						)}
					</HoodCard>
				) : null}
				<HoodCard
					icon={ShieldCheck}
					title={t("showcaseHoodPolicies", "Policies")}
				>
					<ul className="flex flex-col gap-2 text-[13px]">
						<Policy
							ok={!hood.refuse_bots}
							yes={t("showcasePolicyBotsAllowed", "Bots can use it")}
							no={t("showcasePolicyBotsRefused", "Bots are refused")}
						/>
						<Policy
							ok={hood.allow_forking}
							yes={t("showcasePolicyForkOn", "You can fork it")}
							no={t("showcasePolicyForkOff", "Forking is off")}
						/>
						<Policy
							ok
							yes={t(
								"showcasePolicyPersonal",
								"Personal columns are never shown here",
							)}
							no=""
						/>
					</ul>
				</HoodCard>
				{flows.length > 0 ? (
					<HoodCard icon={Workflow} title={t("showcaseHoodFlows", "Flows")}>
						<p className="text-[13px]">
							{t("showcaseHoodFlowTotals", {
								defaultValue_one: "{{count}} flow · {{nodes}} nodes",
								defaultValue_other: "{{count}} flows · {{nodes}} nodes",
								count: flows.length,
								nodes: totalNodes,
							})}
						</p>
						<ul className="flex flex-col">
							{flows.map((f) => {
								const avg = averageScore(f);
								return (
									<li key={f.id}>
										<button
											type="button"
											className="flex w-full items-center justify-between gap-2 border-b border-border py-2 text-left text-[13px] hover:text-primary focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
											onClick={() => onOpenFlow(f.id)}
										>
											<span className="wrap-anywhere">{f.name}</span>
											<span className="flex shrink-0 items-center gap-2 text-xs tabular-nums text-muted-foreground">
												{avg !== null ? (
													<Chip>
														{t("showcaseQualityScore", "Quality {{score}}/10", {
															score: avg.toFixed(1),
														})}
													</Chip>
												) : null}
												{t("showcaseNodeCount", {
													defaultValue_one: "{{count}} node",
													defaultValue_other: "{{count}} nodes",
													count: f.node_count,
												})}
											</span>
										</button>
									</li>
								);
							})}
						</ul>
					</HoodCard>
				) : null}
			</div>
		</ShowcaseSection>
	);
}
