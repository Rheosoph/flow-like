"use client";

import { Loader2, Send } from "lucide-react";
import { useCallback, useState } from "react";
import type { InviteUserRequest } from "../../lib/schema/wasm";
import {
	PackagePermissionBits,
} from "../../lib/permission/wasm-package-permission";
import {
	Button,
	Dialog,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	Input,
	Label,
	Select,
	SelectContent,
	SelectItem,
	SelectTrigger,
	SelectValue,
} from "../ui";

interface PackageInviteDialogProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	onInvite: (request: InviteUserRequest) => void;
	isSubmitting: boolean;
}

export function PackageInviteDialog({
	open,
	onOpenChange,
	onInvite,
	isSubmitting,
}: PackageInviteDialogProps) {
	const [userId, setUserId] = useState("");
	const [permissionLevel, setPermissionLevel] = useState<"maintainer" | "user">(
		"user",
	);

	const reset = useCallback(() => {
		setUserId("");
		setPermissionLevel("user");
	}, []);

	const handleOpenChange = useCallback(
		(next: boolean) => {
			if (!next) reset();
			onOpenChange(next);
		},
		[onOpenChange, reset],
	);

	const handleSubmit = useCallback(() => {
		const trimmed = userId.trim();
		if (!trimmed) return;

		const permission =
			permissionLevel === "maintainer"
				? PackagePermissionBits.Maintainer
				: PackagePermissionBits.User;

		onInvite({ inviteeId: trimmed, permission });
	}, [userId, permissionLevel, onInvite]);

	const canSubmit = userId.trim().length > 0 && !isSubmitting;

	return (
		<Dialog open={open} onOpenChange={handleOpenChange}>
			<DialogContent className="sm:max-w-md">
				<DialogHeader>
					<DialogTitle>Invite User</DialogTitle>
					<DialogDescription>
						Add a user to this package by their user ID.
					</DialogDescription>
				</DialogHeader>

				<div className="grid gap-4 py-4">
					<div className="grid gap-2">
						<Label htmlFor="invite-user-id">User ID</Label>
						<Input
							id="invite-user-id"
							placeholder="Enter user ID"
							value={userId}
							onChange={(e) => setUserId(e.target.value)}
							disabled={isSubmitting}
						/>
					</div>

					<div className="grid gap-2">
						<Label htmlFor="invite-permission">Permission Level</Label>
						<Select
							value={permissionLevel}
							onValueChange={(v) =>
								setPermissionLevel(v as "maintainer" | "user")
							}
							disabled={isSubmitting}
						>
							<SelectTrigger id="invite-permission">
								<SelectValue />
							</SelectTrigger>
							<SelectContent>
								<SelectItem value="user">User</SelectItem>
								<SelectItem value="maintainer">Maintainer</SelectItem>
							</SelectContent>
						</Select>
					</div>
				</div>

				<DialogFooter>
					<Button
						variant="outline"
						onClick={() => handleOpenChange(false)}
						disabled={isSubmitting}
					>
						Cancel
					</Button>
					<Button onClick={handleSubmit} disabled={!canSubmit}>
						{isSubmitting ? (
							<Loader2 className="mr-2 h-4 w-4 animate-spin" />
						) : (
							<Send className="mr-2 h-4 w-4" />
						)}
						Invite
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
}
