"use client";

import { Button } from "@flow-like/flow-like-ui";
import { WorkspaceNotice } from "@flow-like/flow-like-ui/components/store/package-workspace/workspace-parts";
import { getErrorMessage } from "@flow-like/flow-like-ui/lib/error-message";
import { useTranslation } from "@flow-like/locales";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { FolderOpen, Link2, Loader2 } from "lucide-react";
import { toast } from "sonner";
import {
	addProjectFolder,
	pickProjectFolder,
	readManifest,
} from "../local-projects";
import { mineQueryKeys } from "../use-mine-packages";

/** Adds a folder to the developer list and says so when its manifest id is not this package's. */
function useLinkFolder(packageId?: string) {
	const { t } = useTranslation("common");
	const queryClient = useQueryClient();
	return useMutation({
		mutationFn: async () => {
			const path = await pickProjectFolder();
			if (!path) return null;
			const [manifest, project] = await Promise.all([
				readManifest(path),
				addProjectFolder(path),
			]);
			return { project, manifestId: manifest?.id?.trim() || null };
		},
		onSuccess: (result) => {
			if (!result) return;
			void queryClient.invalidateQueries({ queryKey: mineQueryKeys.projects });
			if (packageId && result.manifestId !== packageId) {
				toast.warning(
					t(
						"linkedFolderIdMismatch",
						"Added {{name}}, but its manifest id is {{found}}, not {{id}}. It shows up as its own package.",
						{
							name: result.project.name,
							found: result.manifestId ?? "—",
							id: packageId,
						},
					),
				);
				return;
			}
			toast.success(
				t("addedName", "Added {{name}}", { name: result.project.name }),
			);
		},
		onError: (error) => toast.error(getErrorMessage(error)),
	});
}

/** Test and Manifest without a checkout on this machine. */
export function LinkFolderNotice({
	packageId,
}: Readonly<{ packageId?: string }>) {
	const { t } = useTranslation("common");
	const link = useLinkFolder(packageId);
	return (
		<WorkspaceNotice
			icon={FolderOpen}
			title={t("workspaceNoCheckoutTitle", "No checkout on this machine")}
			description={
				packageId
					? t(
							"workspaceNoCheckoutDescription",
							"Testing and editing the manifest need the source folder. Link the folder whose flow-like.toml uses {{id}}.",
							{ id: packageId },
						)
					: t(
							"workspaceNoCheckoutGeneric",
							"Testing and editing the manifest need the source folder on this machine.",
						)
			}
			action={
				<Button onClick={() => link.mutate()} disabled={link.isPending}>
					{link.isPending ? <Loader2 className="animate-spin" /> : <Link2 />}
					{t("linkFolder", "Link folder…")}
				</Button>
			}
		/>
	);
}
