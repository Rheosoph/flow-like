"use client";

import { useTranslation } from "@flow-like/locales";
import Editor, { type Monaco } from "@monaco-editor/react";
import { useQuery } from "@tanstack/react-query";
import { CheckCircle2, CopyIcon, FileCode2, Waypoints } from "lucide-react";
import { useTheme } from "next-themes";
import Link from "next/link";
import { useCallback, useEffect, useRef, useState } from "react";
import { FLOW_KEY_OPT_OUT_CLASS } from "../../../../lib/monaco-key-guard";
import type { IProfile } from "../../../../lib/schema/profile/profile";
import { useBackend } from "../../../../state/backend-state";
import {
	FLOWSCRIPT_LANGUAGE_ID,
	setupFlowScriptEditor,
} from "../../../flow/flowscript/flowscript-language";
import {
	Badge,
	Button,
	RelativeTime,
	Separator,
	Sheet,
	SheetContent,
	SheetDescription,
	SheetHeader,
	SheetTitle,
	Skeleton,
} from "../../../ui";
import { FlowScriptOutcomeBadge } from "./flowscript-failures-shared";
import { EmptyState } from "./telemetry-shared";
import type { IFlowScriptFailureDetail } from "./types";

function MetaRow({
	label,
	children,
}: {
	readonly label: string;
	readonly children: React.ReactNode;
}) {
	return (
		<div className="flex items-baseline justify-between gap-4 py-1 text-xs">
			<span className="shrink-0 text-muted-foreground">{label}</span>
			<span className="min-w-0 truncate text-right font-mono">{children}</span>
		</div>
	);
}

/** Read-only FlowScript with line numbers, so a diagnostic's line still points somewhere. */
function RedactedSource({ source }: { readonly source: string }) {
	const { resolvedTheme } = useTheme();
	const monacoRef = useRef<Monaco | null>(null);
	const isDark = resolvedTheme === "dark";

	const handleBeforeMount = useCallback(
		(monaco: Monaco) => {
			monacoRef.current = monaco;
			setupFlowScriptEditor(monaco, isDark);
		},
		[isDark],
	);

	useEffect(() => {
		if (!monacoRef.current) return;
		setupFlowScriptEditor(monacoRef.current, isDark);
	}, [isDark]);

	return (
		<Editor
			className={FLOW_KEY_OPT_OUT_CLASS}
			height="46dvh"
			language={FLOWSCRIPT_LANGUAGE_ID}
			value={source}
			beforeMount={handleBeforeMount}
			options={{
				readOnly: true,
				domReadOnly: true,
				minimap: { enabled: false },
				fontSize: 12,
				scrollBeyondLastLine: false,
				wordWrap: "on",
				renderLineHighlight: "none",
			}}
		/>
	);
}

function CopySourceButton({ source }: { readonly source: string }) {
	const { t } = useTranslation("admin");
	const [copied, setCopied] = useState(false);
	return (
		<Button
			variant="outline"
			size="sm"
			onClick={() => {
				navigator.clipboard.writeText(source).catch(() => null);
				setCopied(true);
				setTimeout(() => setCopied(false), 1200);
			}}
		>
			{copied ? (
				<CheckCircle2 className="mr-1 h-3.5 w-3.5 text-green-600" />
			) : (
				<CopyIcon className="mr-1 h-3.5 w-3.5" />
			)}
			{t("copy", "Copy")}
		</Button>
	);
}

export function FlowScriptFailureDetailSheet({
	failureId,
	open,
	onOpenChange,
	profile,
}: {
	readonly failureId: string | null;
	readonly open: boolean;
	readonly onOpenChange: (open: boolean) => void;
	readonly profile: IProfile | undefined;
}) {
	const { t } = useTranslation("admin");
	const backend = useBackend();

	const detail = useQuery<IFlowScriptFailureDetail>({
		queryKey: [
			"admin",
			"telemetry",
			"flowscript-failures",
			"detail",
			failureId,
		],
		queryFn: async () => {
			if (!profile) throw new Error("Profile not loaded");
			if (!failureId) throw new Error("No failure selected");
			return backend.apiState.get<IFlowScriptFailureDetail>(
				profile,
				`admin/telemetry/flowscript-failures/${encodeURIComponent(failureId)}`,
			);
		},
		enabled: Boolean(profile && failureId && open),
	});

	const record = detail.data?.record;

	return (
		<Sheet open={open} onOpenChange={onOpenChange}>
			<SheetContent className="w-full overflow-y-auto sm:max-w-3xl lg:max-w-5xl">
				<SheetHeader>
					<SheetTitle className="flex items-center gap-2">
						<FileCode2 className="h-4 w-4" />
						{t("flowScriptApply", "FlowScript apply")}
					</SheetTitle>
					<SheetDescription>
						{t(
							"theSourceTheUserSubmittedWithDeclaredValuesDroppedAndLongLiteralsGeneralized",
							"The source the user submitted, with declared values dropped and long literals generalized.",
						)}
					</SheetDescription>
				</SheetHeader>

				{detail.isLoading ? (
					<div className="space-y-3 p-4">
						<Skeleton className="h-8 w-64" />
						<Skeleton className="h-64 w-full" />
					</div>
				) : detail.isError || !record || !detail.data ? (
					<EmptyState
						message={t(
							"thisCaptureCouldNotBeLoaded",
							"This capture could not be loaded.",
						)}
						className="m-4 py-10 text-sm"
					/>
				) : (
					<div className="space-y-4 p-4">
						<div className="flex flex-wrap items-center gap-2">
							<FlowScriptOutcomeBadge outcome={record.outcome} />
							<Badge variant="outline" className="text-[10px]">
								{record.source}
							</Badge>
							{record.origin === "agent" ? (
								<Badge variant="secondary" className="text-[10px]">
									{t("flowPilot", "FlowPilot")}
								</Badge>
							) : null}
							{record.userName || record.userId ? (
								<span className="text-xs text-muted-foreground">
									{record.userName ?? record.userId}
								</span>
							) : null}
							<RelativeTime
								value={record.createdAt}
								className="text-xs text-muted-foreground"
							/>
							<div className="ml-auto flex items-center gap-2">
								{record.traceId ? (
									<Button asChild variant="outline" size="sm">
										<Link
											href={`/admin/telemetry/traces?trace=${encodeURIComponent(record.traceId)}`}
										>
											<Waypoints className="mr-1 h-3.5 w-3.5" />
											{t("openTrace", "Open trace")}
										</Link>
									</Button>
								) : null}
								<CopySourceButton source={detail.data.flowscript} />
							</div>
						</div>

						<div className="rounded-lg border border-destructive/40 bg-destructive/5 p-3 text-sm">
							{record.errorMessage ?? record.cause}
						</div>

						<div className="overflow-hidden rounded-lg border">
							<RedactedSource source={detail.data.flowscript} />
						</div>

						{record.diagnostics.length > 0 ? (
							<div className="space-y-1">
								<div className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
									{t("diagnostics", "Diagnostics")}
								</div>
								<ul className="space-y-1">
									{record.diagnostics.map((diagnostic) => (
										<li
											key={diagnostic}
											className="rounded-md border bg-muted/40 px-2.5 py-1.5 text-xs"
										>
											{diagnostic}
										</li>
									))}
								</ul>
							</div>
						) : null}

						{detail.data.corrections.length > 0 ? (
							<div className="space-y-1">
								<div className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">
									{t("corrections", "Corrections")}
								</div>
								<ul className="space-y-1">
									{detail.data.corrections.map((correction) => (
										<li
											key={correction}
											className="rounded-md border bg-muted/40 px-2.5 py-1.5 text-xs"
										>
											{correction}
										</li>
									))}
								</ul>
							</div>
						) : null}

						<Separator />

						<div className="grid gap-x-6 sm:grid-cols-2">
							<MetaRow label={t("app", "App")}>{record.appId}</MetaRow>
							<MetaRow label={t("boardLabel", "Board")}>
								{record.boardId}
							</MetaRow>
							<MetaRow label={t("layer", "Layer")}>
								{record.layerId ?? "—"}
							</MetaRow>
							<MetaRow label={t("commands", "Commands")}>
								{record.commandCount.toLocaleString()}
							</MetaRow>
							<MetaRow label={t("deletionsAllowed", "Deletions allowed")}>
								{String(record.allowDeletions)}
							</MetaRow>
							<MetaRow label={t("platform", "Platform")}>
								{record.platform ?? "—"}
							</MetaRow>
							<MetaRow label={t("version", "Version")}>
								{record.appVersion ?? "—"}
							</MetaRow>
							<MetaRow label={t("redaction", "Redaction")}>
								{t(
									"droppedValuesValuesLiteralsLiterals",
									"{{values}} values · {{literals}} literals",
									{
										values: record.droppedValues,
										literals: record.redactedLiterals,
									},
								)}
							</MetaRow>
						</div>

						{record.truncated ? (
							<div className="text-xs text-muted-foreground">
								{t(
									"thisSourceHitItsStorageCapAndWasTruncated",
									"This source hit its storage cap and was truncated.",
								)}
							</div>
						) : null}
					</div>
				)}
			</SheetContent>
		</Sheet>
	);
}
