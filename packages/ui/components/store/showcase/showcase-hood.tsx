"use client";

import { useTranslation } from "@flow-like/locales";
import {
	Ban,
	Check,
	ChevronRight,
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
} from "lucide-react";
import type { ReactNode } from "react";
import type {
	IShowcaseFlow,
	IShowcaseHood,
} from "../../../lib/schema/app/showcase";
import { cn } from "../../../lib/utils";
import { PERMISSION_LABELS, formatNumber } from "./model";
import { Chip, ShowcaseSection } from "./showcase-ui";

const ACRONYM_KINDS = new Set(["llm", "vlm"]);
const modelKindLabel = (kind: string) =>
	ACRONYM_KINDS.has(kind.toLowerCase()) ? kind.toUpperCase() : kind;

export const averageScore = (f: IShowcaseFlow): number | null => {
	if (!f.scores) return null;
	const v = Object.values(f.scores);
	return v.length ? v.reduce((a, b) => a + b, 0) / v.length : null;
};

function SpecRow({
	icon: Icon,
	title,
	children,
}: Readonly<{ icon: LucideIcon; title: string; children: ReactNode }>) {
	return (
		<div className="grid gap-x-6 gap-y-2 px-4 py-3.5 sm:grid-cols-[11rem_minmax(0,1fr)]">
			<h3 className="flex items-center gap-2 text-[13px] font-medium leading-6 tracking-normal text-muted-foreground">
				<Icon className="h-4 w-4 shrink-0" aria-hidden="true" />
				{title}
			</h3>
			<div className="min-w-0 text-[13px] leading-6">{children}</div>
		</div>
	);
}

function YesNo({
	ok,
	icon: Icon,
	label,
}: Readonly<{ ok: boolean; icon?: LucideIcon; label: string }>) {
	const { t } = useTranslation("store");
	const Mark = ok ? Check : Ban;
	return (
		<li
			className={cn(
				"inline-flex items-center gap-1.5",
				!ok && "text-muted-foreground",
			)}
		>
			{Icon ? (
				<Icon
					className="h-3.5 w-3.5 text-muted-foreground"
					aria-hidden="true"
				/>
			) : null}
			{label}
			<Mark
				className={cn(
					"h-3.5 w-3.5",
					ok ? "text-[var(--showcase-time)]" : "text-muted-foreground",
				)}
				aria-hidden="true"
			/>
			<span className="sr-only">
				{ok ? t("showcaseYes", "Yes") : t("showcaseNo", "No")}
			</span>
		</li>
	);
}

function FlowRow({
	flow,
	onOpen,
}: Readonly<{ flow: IShowcaseFlow; onOpen: (id: string) => void }>) {
	const { t } = useTranslation("store");
	const avg = averageScore(flow);
	return (
		<button
			type="button"
			className="group grid w-full grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-x-3 px-4 py-3 text-left transition-colors hover:bg-muted/40 focus-visible:bg-muted/40 focus-visible:outline-none sm:grid-cols-[auto_minmax(0,1fr)_9rem_6rem_auto]"
			onClick={() => onOpen(flow.id)}
		>
			<span className="grid h-8 w-8 place-items-center rounded-lg border border-border bg-muted/60 text-muted-foreground">
				<Workflow className="h-4 w-4" aria-hidden="true" />
			</span>
			<span className="flex min-w-0 flex-col">
				<span className="truncate text-[13.5px] font-medium">{flow.name}</span>
				{flow.description ? (
					<small className="truncate text-xs text-muted-foreground">
						{flow.description}
					</small>
				) : null}
			</span>
			<span className="hidden items-center gap-2 sm:flex">
				{avg !== null ? (
					<>
						<span
							className="h-1.5 flex-1 overflow-hidden rounded-full bg-muted"
							role="img"
							aria-label={t("showcaseQualityScore", "Quality {{score}}/10", {
								score: avg.toFixed(1),
							})}
						>
							<span
								className="block h-full rounded-full bg-primary"
								style={{ width: `${Math.min(100, avg * 10)}%` }}
							/>
						</span>
						<span className="w-7 text-right text-xs font-medium tabular-nums">
							{avg.toFixed(1)}
						</span>
					</>
				) : null}
			</span>
			<span className="hidden text-right text-xs tabular-nums text-muted-foreground sm:block">
				{t("showcaseNodeCount", {
					defaultValue_one: "{{count}} node",
					defaultValue_other: "{{count}} nodes",
					count: flow.node_count,
				})}
			</span>
			<ChevronRight
				className="h-4 w-4 text-muted-foreground transition-transform group-hover:translate-x-0.5"
				aria-hidden="true"
			/>
		</button>
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
			<div className="divide-y divide-border overflow-hidden rounded-xl border border-border bg-card">
				{mode ? (
					<SpecRow
						icon={RunIcon}
						title={t("showcaseHoodRuns", "Where it runs")}
					>
						<ul className="flex flex-wrap gap-x-5 gap-y-1">
							<YesNo
								ok={cloud}
								icon={Cloud}
								label={t("showcaseHoodCloud", "Cloud")}
							/>
							<YesNo
								ok={device}
								icon={Smartphone}
								label={t("showcaseHoodDevice", "Your device")}
							/>
						</ul>
						{hood.requires_local ? (
							<p className="text-xs leading-5 text-muted-foreground">
								{t(
									"showcaseHoodRequiresLocal",
									"Some steps only run on your own device.",
								)}
							</p>
						) : null}
					</SpecRow>
				) : null}
				{hood.models && hood.models.length > 0 ? (
					<SpecRow icon={Sparkles} title={t("showcaseHoodModels", "AI models")}>
						<ul className="flex flex-wrap gap-1.5">
							{hood.models.map((m) => (
								<li
									key={m.id}
									className="inline-flex items-center gap-2 rounded-md border border-border py-0.5 pr-1 pl-2"
								>
									<span className="font-medium">{m.name}</span>
									<Chip>{modelKindLabel(m.kind)}</Chip>
								</li>
							))}
						</ul>
					</SpecRow>
				) : null}
				{hood.packages && hood.packages.length > 0 ? (
					<SpecRow
						icon={Package}
						title={t("showcaseHoodPackages", "Node packages")}
					>
						<ul className="flex flex-col gap-2.5">
							{hood.packages.map((p) => (
								<li key={p.id} className="flex flex-col gap-1">
									<span className="font-mono text-[12.5px] font-medium">
										{p.name}{" "}
										<span className="text-muted-foreground">{p.version}</span>
									</span>
									{p.permissions.length > 0 ? (
										<span className="flex flex-wrap items-center gap-1.5">
											<small className="text-xs text-muted-foreground">
												{t(
													"showcaseHoodSandboxed",
													"Runs sandboxed. Allowed to use:",
												)}
											</small>
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
									) : null}
								</li>
							))}
						</ul>
					</SpecRow>
				) : null}
				{hood.oauth ? (
					<SpecRow
						icon={KeyRound}
						title={t("showcaseHoodAccounts", "Accounts you connect")}
					>
						{hood.oauth.length > 0 ? (
							<ul className="flex flex-col gap-1.5">
								{hood.oauth.map((a) => (
									<li
										key={a.provider}
										className="flex flex-wrap items-center gap-1.5"
									>
										<span className="mr-1 font-medium">{a.provider}</span>
										{a.scopes.map((s) => (
											<Chip key={s} className="font-mono">
												{s}
											</Chip>
										))}
									</li>
								))}
							</ul>
						) : (
							<span className="text-muted-foreground">
								{t(
									"showcaseHoodNoAccounts",
									"None. It works without signing in to other services.",
								)}
							</span>
						)}
					</SpecRow>
				) : null}
				<SpecRow
					icon={ShieldCheck}
					title={t("showcaseHoodPolicies", "Policies")}
				>
					<ul className="flex flex-wrap gap-x-5 gap-y-1">
						<YesNo
							ok={!hood.refuse_bots}
							label={
								hood.refuse_bots
									? t("showcasePolicyBotsRefused", "Bots are refused")
									: t("showcasePolicyBotsAllowed", "Bots can use it")
							}
						/>
						<YesNo
							ok={hood.allow_forking}
							label={
								hood.allow_forking
									? t("showcasePolicyForkOn", "You can fork it")
									: t("showcasePolicyForkOff", "Forking is off")
							}
						/>
						<YesNo
							ok
							label={t(
								"showcasePolicyPersonal",
								"Personal columns are never shown here",
							)}
						/>
					</ul>
				</SpecRow>
			</div>
			{flows.length > 0 ? (
				<div className="overflow-hidden rounded-xl border border-border bg-card">
					<div className="flex items-center justify-between gap-3 border-b border-border bg-muted/30 px-4 py-2">
						<span className="flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-widest text-muted-foreground">
							<Workflow className="h-3.5 w-3.5" aria-hidden="true" />
							{t("showcaseHoodFlows", "Flows")}
						</span>
						<span className="text-xs tabular-nums text-muted-foreground">
							{t("showcaseHoodFlowTotals", {
								defaultValue_one: "{{count}} flow · {{nodes}} nodes",
								defaultValue_other: "{{count}} flows · {{nodes}} nodes",
								count: flows.length,
								nodes: formatNumber(totalNodes),
							})}
						</span>
					</div>
					<div className="divide-y divide-border">
						{flows.map((f) => (
							<FlowRow key={f.id} flow={f} onOpen={onOpenFlow} />
						))}
					</div>
				</div>
			) : null}
		</ShowcaseSection>
	);
}
