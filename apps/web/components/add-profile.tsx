import {
	Button,
	Dialog,
	DialogClose,
	DialogContent,
	DialogDescription,
	DialogFooter,
	DialogHeader,
	DialogTitle,
	type ISettingsProfile,
	Input,
	Label,
	Textarea,
	useBackend,
	useInvoke,
} from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import { createId } from "@paralleldrive/cuid2";
import { Save } from "lucide-react";
import type React from "react";
import { useCallback, useMemo, useRef, useState } from "react";

export interface CreateProfileDialogProps {
	open: boolean;
	setOpen: (open: boolean) => void;
	onCreate: (payload: ISettingsProfile) => Promise<void>;
	triggerLabel?: string;
	defaultOpen?: boolean;
}

export const CreateProfileDialog: React.FC<CreateProfileDialogProps> = ({
	open,
	setOpen,
	onCreate,
	triggerLabel = "New Profile",
	defaultOpen = false,
}) => {
	const { t } = useTranslation("common");
	const backend = useBackend();
	const currentProfile = useInvoke(
		backend.userState.getSettingsProfile,
		backend.userState,
		[],
	);
	const [name, setName] = useState<string>("");
	const [description, setDescription] = useState<string>("");
	const [creating, setCreating] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const busy = useRef(false);
	const pendingId = useRef<string | null>(null);

	const canCreate = useMemo(
		() => name.trim().length > 0 && name.length <= 100,
		[name],
	);

	const handleCreate = useCallback(async () => {
		if (!currentProfile.data) return;
		if (!canCreate || busy.current) return;
		busy.current = true;
		setCreating(true);
		setError(null);
		const now = new Date().toISOString();
		pendingId.current ??= createId();
		const payload: ISettingsProfile = {
			...currentProfile.data,
			hub_profile: {
				...currentProfile.data.hub_profile,
				id: pendingId.current,
				name: name.trim(),
				description: description.trim() || null,
				icon: null,
				interests: [],
				tags: [],
				apps: [],
				bits: [],
				custom_bits: [],
				shortcuts: [],
				home_layout: null,
				thumbnail: null,
				theme: null,
			},
		};
		try {
			await onCreate(payload);
			pendingId.current = null;
			setName("");
			setDescription("");
			setOpen(false);
		} catch (error) {
			setError(
				error instanceof Error
					? error.message
					: "Could not create the profile. Try again.",
			);
		} finally {
			busy.current = false;
			setCreating(false);
		}
	}, [canCreate, name, description, onCreate, currentProfile.data, setOpen]);

	return (
		<Dialog
			open={open}
			onOpenChange={(value) => {
				if (!busy.current) setOpen(value);
			}}
		>
			<DialogContent className="sm:max-w-lg">
				<DialogHeader>
					<DialogTitle>{t("createProfile", "Create Profile")}</DialogTitle>
					<DialogDescription>
						{t(
							"provideANameAndOptionalDescription",
							"Provide a name and optional description.",
						)}
					</DialogDescription>
				</DialogHeader>

				<div className="grid gap-4 py-4">
					<div className="space-y-2">
						<Label htmlFor="create-profile-name">Name</Label>
						<Input
							id="create-profile-name"
							maxLength={100}
							disabled={creating}
							value={name}
							onChange={(e) => setName(e.target.value)}
							placeholder={t("profileName", "Profile name")}
							autoFocus
						/>
					</div>

					<div className="space-y-2">
						<Label htmlFor="create-profile-description">
							{t("descriptionOptional", "Description (optional)")}
						</Label>
						<Textarea
							id="create-profile-description"
							disabled={creating}
							value={description}
							onChange={(e) => setDescription(e.target.value)}
							placeholder={t("shortDescription", "Short description...")}
							rows={3}
						/>
					</div>
				</div>

				{error && (
					<p role="alert" className="text-sm text-destructive">
						{error}
					</p>
				)}
				<DialogFooter>
					<DialogClose asChild>
						<Button variant="ghost" disabled={creating}>
							{t("cancel", "Cancel")}
						</Button>
					</DialogClose>
					<Button
						onClick={handleCreate}
						disabled={!canCreate || !currentProfile.data || creating}
						className="flex items-center gap-2"
					>
						<Save className="h-4 w-4" />
						{creating ? t("creating", "Creating…") : t("create", "Create")}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
};
