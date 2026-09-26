"use client";

import {
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@flow-like/flow-like-ui";
import { Trans, useTranslation } from "@flow-like/locales";
import { open } from "@tauri-apps/plugin-dialog";
import { FolderOpen, LayoutTemplate, Loader2 } from "lucide-react";
import { useSearchParams } from "next/navigation";
import { Suspense, useCallback, useEffect, useState } from "react";
import {
	ToolPageHeader,
	toolBackHref,
} from "../../../components/packages/workspace/tool-page-header";
import { WidgetTester } from "../../../components/packages/workspace/widget-tester";

function TestWidgetPageContent() {
	const { t } = useTranslation("common");
	const initialProject = useSearchParams().get("project") ?? "";
	const [projectDir, setProjectDir] = useState(initialProject);

	useEffect(() => {
		if (initialProject) setProjectDir(initialProject);
	}, [initialProject]);

	const selectDirectory = useCallback(async () => {
		const selected = await open({ directory: true, multiple: false });
		if (typeof selected === "string") setProjectDir(selected);
	}, []);

	return (
		<div className="flex flex-col h-full">
			<ToolPageHeader
				icon={LayoutTemplate}
				title={t("testWidget", "Test Widget")}
				description={t(
					"renderYourProjectsBuiltWidgetsThroughTheRealSandboxedHost",
					"Render your project's built widgets through the real sandboxed host",
				)}
				backHref={toolBackHref(projectDir)}
				actions={
					<Button
						variant="outline"
						size="sm"
						onClick={selectDirectory}
						className="gap-1.5"
					>
						<FolderOpen className="h-4 w-4" />
						{projectDir
							? t("changeProject", "Change project")
							: t("selectProject", "Select project")}
					</Button>
				}
			/>

			{projectDir && (
				<p className="text-xs text-muted-foreground/60 font-mono pt-3 truncate">
					{projectDir}
				</p>
			)}

			<div className="flex-1 overflow-y-auto min-h-0 py-4 pb-12">
				{projectDir ? (
					<WidgetTester key={projectDir} projectPath={projectDir} />
				) : (
					<Card className="max-w-md mx-auto mt-12">
						<CardHeader>
							<CardTitle>
								{t("noProjectSelected", "No Project Selected")}
							</CardTitle>
							<CardDescription>
								{t(
									"pickALocalProjectDirectoryContainingABuilt",
									"Pick a local project directory containing a built",
								)}{" "}
								<Trans i18nKey="codewidgetsflwbcodeToPreviewItsWidgets">
									<code>widgets.flwb</code> to preview its widgets.
								</Trans>
							</CardDescription>
						</CardHeader>
						<CardContent>
							<Button onClick={selectDirectory} className="gap-1.5">
								<FolderOpen className="h-4 w-4" />
								{t("selectProjectDirectory", "Select Project Directory")}
							</Button>
						</CardContent>
					</Card>
				)}
			</div>
		</div>
	);
}

export default function TestWidgetPage() {
	return (
		<Suspense
			fallback={
				<div className="flex items-center justify-center h-full">
					<Loader2 className="h-6 w-6 animate-spin text-muted-foreground/60" />
				</div>
			}
		>
			<TestWidgetPageContent />
		</Suspense>
	);
}
