"use client";

import { Button, Card, CardContent } from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import { open } from "@tauri-apps/plugin-dialog";
import { FileText, FolderOpen, Loader2 } from "lucide-react";
import { useSearchParams } from "next/navigation";
import { Suspense, useCallback, useEffect, useState } from "react";
import { ManifestEditor } from "../../../components/packages/workspace/manifest-editor";
import {
	ToolPageHeader,
	toolBackHref,
} from "../../../components/packages/workspace/tool-page-header";

function ManifestPageContent() {
	const { t } = useTranslation("common");
	const initialPath = useSearchParams().get("path") ?? "";
	const [projectPath, setProjectPath] = useState(initialPath);

	useEffect(() => {
		if (initialPath) setProjectPath(initialPath);
	}, [initialPath]);

	const selectProject = useCallback(async () => {
		const selected = await open({ directory: true, multiple: false });
		if (typeof selected === "string") setProjectPath(selected);
	}, []);

	const openButton = (
		<Button variant="outline" size="sm" onClick={selectProject}>
			<FolderOpen />
			{projectPath
				? t("changeProject", "Change project")
				: t("selectProjectDirectory", "Select Project Directory")}
		</Button>
	);

	return (
		<div className="flex flex-col h-full">
			<ToolPageHeader
				icon={FileText}
				title={t("manifestEditor", "Manifest Editor")}
				description={t(
					"configureYourFlowliketomlVisually",
					"Configure your flow-like.toml visually",
				)}
				backHref={toolBackHref(projectPath)}
				actions={openButton}
			/>
			<div className="flex-1 overflow-y-auto py-4">
				{projectPath ? (
					<ManifestEditor key={projectPath} projectPath={projectPath} />
				) : (
					<Card>
						<CardContent className="py-16 text-center space-y-3">
							<FileText className="h-12 w-12 text-muted-foreground/30 mx-auto" />
							<p className="font-medium">
								{t(
									"selectAProjectToEditItsManifest",
									"Select a project to edit its manifest",
								)}
							</p>
							{openButton}
						</CardContent>
					</Card>
				)}
			</div>
		</div>
	);
}

export default function ManifestPage() {
	return (
		<Suspense
			fallback={
				<div className="flex items-center justify-center h-full">
					<Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
				</div>
			}
		>
			<ManifestPageContent />
		</Suspense>
	);
}
