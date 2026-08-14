"use client";

import { useTranslation } from "@flow-like/locales";
import { Loader2, Package, Rocket } from "lucide-react";
import { useCallback, useMemo, useState } from "react";
import {
	Badge,
	Button,
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	Label,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
	Textarea,
} from "../ui";

type VersionBump = "patch" | "minor" | "major";

export interface PublishDialogProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	packageName: string;
	currentVersion?: string;
	isNewPackage: boolean;
	onPublish: (data: {
		versionBump: VersionBump;
		releaseNotes: string;
	}) => void;
	isPublishing: boolean;
}

function bumpVersion(current: string, bump: VersionBump): string {
	const parts = current.split(".").map(Number);
	if (parts.length !== 3) return current;
	switch (bump) {
		case "major":
			return `${parts[0] + 1}.0.0`;
		case "minor":
			return `${parts[0]}.${parts[1] + 1}.0`;
		case "patch":
			return `${parts[0]}.${parts[1]}.${parts[2] + 1}`;
	}
}

const MIN_RELEASE_NOTES_LENGTH = 10;

export function PublishDialog({
	open,
	onOpenChange,
	packageName,
	currentVersion,
	isNewPackage,
	onPublish,
	isPublishing,
}: PublishDialogProps) {
	const { t } = useTranslation("store");
	const [versionBump, setVersionBump] = useState<VersionBump>("patch");
	const [releaseNotes, setReleaseNotes] = useState("");

	const nextVersion = useMemo(
		() => (currentVersion ? bumpVersion(currentVersion, versionBump) : null),
		[currentVersion, versionBump],
	);

	const isValid = releaseNotes.trim().length >= MIN_RELEASE_NOTES_LENGTH;

	const handlePublish = useCallback(() => {
		if (!isValid) return;
		onPublish({ versionBump, releaseNotes: releaseNotes.trim() });
	}, [isValid, onPublish, versionBump, releaseNotes]);

	const handleOpenChange = useCallback(
		(next: boolean) => {
			if (!next) {
				setVersionBump("patch");
				setReleaseNotes("");
			}
			onOpenChange(next);
		},
		[onOpenChange],
	);

	return (
		<Dialog open={open} onOpenChange={handleOpenChange}>
			<DialogContent className="sm:max-w-md">
				<DialogHeader>
					<DialogTitle className="flex items-center gap-2">
						{isNewPackage ? (
							<Rocket className="h-5 w-5" />
						) : (
							<Package className="h-5 w-5" />
						)}
						{isNewPackage ? "First Publish" : "Publish Update"}
					</DialogTitle>
					<DialogDescription>
						{isNewPackage
							? t('publishPackagenameToTheRegistryForTheFirstTime', 'Publish {{packageName}} to the registry for the first time.', { packageName })
							: t('publishANewVersionOfPackagename', 'Publish a new version of {{packageName}}.', { packageName })}
					</DialogDescription>
				</DialogHeader>

				<div className="space-y-4 py-2">
					{!isNewPackage && currentVersion && (
						<div className="space-y-2">
							<Label>{t('versionBump', 'Version Bump')}</Label>
							<div className="flex items-center gap-3">
								<Select
									value={versionBump}
									onValueChange={(v) => setVersionBump(v as VersionBump)}
								>
									<SelectTrigger className="w-32">
										<SelectValue />
									</SelectTrigger>
									<SelectContent>
										<SelectItem value="patch">{t('patch', 'Patch')}</SelectItem>
										<SelectItem value="minor">{t('minor', 'Minor')}</SelectItem>
										<SelectItem value="major">{t('major', 'Major')}</SelectItem>
									</SelectContent>
								</Select>

								<div className="flex items-center gap-2 text-sm">
									<Badge variant="outline">{currentVersion}</Badge>
									<span className="text-muted-foreground">→</span>
									<Badge>{nextVersion}</Badge>
								</div>
							</div>
						</div>
					)}

					<div className="space-y-2">
						<Label htmlFor="release-notes">{t('releaseNotes', 'Release Notes')}</Label>
						<Textarea
							id="release-notes"
							placeholder={t('describeWhatChangedInThisVersion', 'Describe what changed in this version…')}
							value={releaseNotes}
							onChange={(e) => setReleaseNotes(e.target.value)}
							rows={4}
							className="resize-none"
						/>
						{releaseNotes.length > 0 &&
							releaseNotes.trim().length < MIN_RELEASE_NOTES_LENGTH && (
								<p className="text-xs text-destructive">
									{t(
										"releaseNotesMustBeAtLeastMin_release_notes_length",
										"Release notes must be at least {{MIN_RELEASE_NOTES_LENGTH}} characters.",
										{ MIN_RELEASE_NOTES_LENGTH },
									)}
								</p>
							)}
					</div>
				</div>

				<DialogFooter>
					<Button
						variant="outline"
						onClick={() => handleOpenChange(false)}
						disabled={isPublishing}
					>
						{t('cancel', 'Cancel')}
					</Button>
					<Button onClick={handlePublish} disabled={!isValid || isPublishing}>
						{isPublishing ? (
							<>
								<Loader2 className="h-4 w-4 mr-2 animate-spin" />
								{t('publishing', 'Publishing…')}
							</>
						) : (
							"Publish"
						)}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
