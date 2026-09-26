"use client";

import { RelativeTime } from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import { LayoutTemplate, Play } from "lucide-react";
import type { ReactNode } from "react";
import {
	NodeDebugger,
	type NodeDebuggerSession,
	NodeDebuggerStatus,
	ReinspectButton,
} from "./node-debugger";
import { WidgetTester } from "./widget-tester";

function InspectionBar({
	session,
	children,
}: Readonly<{ session: NodeDebuggerSession; children?: ReactNode }>) {
	const { t } = useTranslation("common");
	return (
		<div className="flex flex-wrap items-center justify-between gap-3">
			<div className="flex min-w-0 items-center gap-2 text-sm text-muted-foreground">
				{children}
				{session.inspectedAt > 0 && (
					<span className="text-xs">
						{t("workspaceInspected", "Inspected")}{" "}
						<RelativeTime value={session.inspectedAt} />
					</span>
				)}
			</div>
			<ReinspectButton session={session} />
		</div>
	);
}

/** Nodes tab for a linked checkout: the inspected nodes and their lint. */
export function LocalNodesTab({
	session,
	onRun,
}: Readonly<{ session: NodeDebuggerSession; onRun: () => void }>) {
	const { t } = useTranslation("common");
	return (
		<div className="space-y-4">
			<InspectionBar session={session} />
			<NodeDebuggerStatus
				session={session}
				emptyHint={t(
					"noNodesInThisBuild",
					"This build exports no nodes. Build the package and inspect it again.",
				)}
			/>
			{session.nodes.length > 0 && (
				<div className="grid items-start gap-4 min-[1800px]:grid-cols-[minmax(0,2fr)_minmax(0,1fr)]">
					<NodeDebugger session={session} view="nodes" onOpenView={onRun} />
					<NodeDebugger session={session} view="lint" onOpenView={onRun} />
				</div>
			)}
		</div>
	);
}

function TestSection({
	icon: Icon,
	title,
	children,
}: Readonly<{ icon: typeof Play; title: string; children: ReactNode }>) {
	return (
		<section className="space-y-3">
			<h2 className="flex items-center gap-2 text-sm font-semibold">
				<Icon className="size-4 text-muted-foreground" />
				{title}
			</h2>
			{children}
		</section>
	);
}

/** Test tab for a linked checkout: run a node in the debug sandbox, then preview the widgets. */
export function LocalTestTab({
	session,
	projectPath,
}: Readonly<{ session: NodeDebuggerSession; projectPath: string }>) {
	const { t } = useTranslation("common");
	const inspection = session.inspection;
	const hasWidgets = Boolean(
		inspection?.widgetBundlePath || inspection?.widgets.length,
	);
	const showNodes = session.nodes.length > 0 || !hasWidgets;

	return (
		<div className="space-y-8">
			{showNodes && (
				<TestSection icon={Play} title={t("workspaceRunNodes", "Run nodes")}>
					<NodeDebuggerStatus
						session={session}
						emptyHint={t(
							"workspaceNothingToTest",
							"Nothing to run yet. Build the package (mise run build), then re-inspect it.",
						)}
					/>
					{session.nodes.length > 0 && (
						<div className="space-y-3">
							<NodeDebugger session={session} view="run" />
						</div>
					)}
				</TestSection>
			)}
			{hasWidgets && (
				<TestSection
					icon={LayoutTemplate}
					title={t("workspacePreviewWidgets", "Preview widgets")}
				>
					<WidgetTester projectPath={projectPath} />
				</TestSection>
			)}
		</div>
	);
}
