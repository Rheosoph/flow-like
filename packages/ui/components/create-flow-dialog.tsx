"use client";

import { ExternalLink, Globe, Loader2, Sparkles, WifiOff } from "lucide-react";
import { useCallback, useEffect, useId, useState } from "react";
import { isTauri } from "../lib/platform";
import { handleUpgradeRequiredError } from "../state/upgrade-dialog-state";
import { AutoPlayNewProjectIcon } from "./animated-icons/animated-plus-autoplay";
import { Button } from "./ui/button";
import {
	Dialog,
	DialogClose,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
} from "./ui/dialog";
import { Input } from "./ui/input";
import { Label } from "./ui/label";
import { RadioGroup, RadioGroupItem } from "./ui/radio-group";

export interface CreateFlowDialogToast {
	success: (message: string) => void;
	error: (message: string) => void;
}

export interface CreateFlowDialogProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	onCreateProject: (projectName: string, isOnline: boolean) => Promise<void>;
	isAuthenticated?: boolean;
	defaultOnline?: boolean;
	toast: CreateFlowDialogToast;
}

export function CreateFlowDialog({
	open,
	onOpenChange,
	onCreateProject,
	isAuthenticated = true,
	defaultOnline = true,
	toast,
}: Readonly<CreateFlowDialogProps>) {
	const projectNameId = useId();
	const onlineId = useId();
	const offlineId = useId();
	const [projectName, setProjectName] = useState("");
	const [isOnline, setIsOnline] = useState(defaultOnline && isAuthenticated);
	const [isCreating, setIsCreating] = useState(false);

	useEffect(() => {
		if (!open) return;
		setIsOnline(defaultOnline && isAuthenticated);
	}, [defaultOnline, isAuthenticated, open]);

	const handleStartCoding = useCallback(async () => {
		if (!projectName.trim()) {
			toast.error("Please enter a project name");
			return;
		}

		if (isOnline && !isAuthenticated) {
			toast.error("You must be logged in to create an online project");
			return;
		}

		setIsCreating(true);
		try {
			await onCreateProject(projectName.trim(), isOnline);
			toast.success("Project created!");
			onOpenChange(false);
			setProjectName("");
			setIsOnline(defaultOnline && isAuthenticated);
		} catch (error) {
			console.error("Failed to create project:", error);
			if (handleUpgradeRequiredError(error, "project-limit")) {
				onOpenChange(false);
				return;
			}
			toast.error(
				error instanceof Error ? error.message : "Failed to create project",
			);
		} finally {
			setIsCreating(false);
		}
	}, [
		defaultOnline,
		isAuthenticated,
		isOnline,
		onCreateProject,
		onOpenChange,
		projectName,
		toast,
	]);

	return (
		<Dialog open={open} onOpenChange={onOpenChange}>
			<DialogContent>
				<DialogHeader>
					<DialogTitle className="flex items-center gap-2">
						<AutoPlayNewProjectIcon className="h-5 w-5" />
						Create Flow
					</DialogTitle>
					<DialogDescription>
						Create a new project with all embedding models from your current
						profile
					</DialogDescription>
				</DialogHeader>
				<div className="grid gap-4 py-4">
					<div className="grid gap-2">
						<Label htmlFor={projectNameId}>Project Name</Label>
						<Input
							id={projectNameId}
							placeholder="My Awesome Project"
							value={projectName}
							onChange={(e) => setProjectName(e.target.value)}
							onKeyDown={(e) => {
								if (e.key === "Enter" && !isCreating) {
									handleStartCoding();
								}
							}}
							disabled={isCreating}
						/>
					</div>
					<div className="grid gap-3">
						<Label>Connectivity</Label>
						<RadioGroup
							value={isOnline ? "online" : "offline"}
							onValueChange={(value) => {
								if (value === "online" && !isAuthenticated) {
									toast.error("Please log in to create online projects");
									return;
								}
								setIsOnline(value === "online");
							}}
							disabled={isCreating}
						>
							<div className="flex items-center space-x-2 relative">
								<RadioGroupItem
									value="online"
									id={onlineId}
									disabled={!isAuthenticated || isCreating}
								/>
								<Label
									htmlFor={onlineId}
									className={`flex items-center gap-2 font-normal ${
										isAuthenticated
											? "cursor-pointer"
											: "cursor-not-allowed opacity-50"
									}`}
								>
									<Globe className="h-4 w-4" />
									Online - Sync with cloud
									{!isAuthenticated && (
										<span className="text-xs text-muted-foreground ml-1">
											(Login required)
										</span>
									)}
								</Label>
							</div>
							{isTauri() ? (
								<div className="flex items-center space-x-2">
									<RadioGroupItem
										value="offline"
										id={offlineId}
										disabled={isCreating}
									/>
									<Label
										htmlFor={offlineId}
										className="flex items-center gap-2 font-normal cursor-pointer"
									>
										<WifiOff className="h-4 w-4" />
										Offline - Local only
									</Label>
								</div>
							) : (
								<div className="flex items-center space-x-2 opacity-50">
									<RadioGroupItem value="offline" id={offlineId} disabled />
									<Label
										htmlFor={offlineId}
										className="flex items-center gap-2 font-normal cursor-not-allowed"
									>
										<WifiOff className="h-4 w-4" />
										Offline - Local only
										<a
											href="https://flow-like.com/download"
											target="_blank"
											rel="noopener noreferrer"
											className="text-xs text-primary hover:underline flex items-center gap-1 ml-1"
											onClick={(e) => e.stopPropagation()}
										>
											(Get Studio <ExternalLink className="h-3 w-3" />)
										</a>
									</Label>
								</div>
							)}
						</RadioGroup>
					</div>
				</div>
				<DialogFooter>
					<DialogClose asChild>
						<Button variant="outline" disabled={isCreating}>
							Cancel
						</Button>
					</DialogClose>
					<Button onClick={handleStartCoding} disabled={isCreating}>
						{isCreating ? (
							<>
								<Loader2 className="mr-2 h-4 w-4 animate-spin" />
								Creating...
							</>
						) : (
							<>
								<Sparkles className="mr-2 h-4 w-4" />
								Create Project
							</>
						)}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
