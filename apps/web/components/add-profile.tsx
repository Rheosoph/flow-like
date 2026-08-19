import { useTranslation } from "@flow-like/locales";
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
import { createId } from "@paralleldrive/cuid2";
import { Save } from "lucide-react";
import type React from "react";
import { useCallback, useMemo, useState } from "react";

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

	const canCreate = useMemo(() => name.trim().length > 0, [name]);

	const handleCreate = useCallback(async () => {
		if (!currentProfile.data) return;
		if (!canCreate) return;
		const now = new Date().toISOString();
		const payload: ISettingsProfile = {
			...currentProfile.data,
			hub_profile: {
				...currentProfile.data.hub_profile,
				id: createId(),
				name: name.trim(),
				description: description.trim() || null,
				icon: null,
				interests: [],
				thumbnail: null,
				theme: null,
			},
		};
		await onCreate(payload);
		setName("");
		setDescription("");
		setOpen(false);
	}, [canCreate, name, description, onCreate, currentProfile.data]);

	return (
		<Dialog open={open} onOpenChange={setOpen}>
			<DialogContent className="sm:max-w-lg">
				<DialogHeader>
					<DialogTitle>{t('createProfile', 'Create Profile')}</DialogTitle>
					<DialogDescription>
						{t('provideANameAndOptionalDescription', 'Provide a name and optional description.')}
					</DialogDescription>
				</DialogHeader>

				<div className="grid gap-4 py-4">
					<div className="space-y-2">
						<Label htmlFor="create-profile-name">Name</Label>
						<Input
							id="create-profile-name"
							value={name}
							onChange={(e) => setName(e.target.value)}
							placeholder={t('profileName', 'Profile name')}
							autoFocus
						/>
					</div>

					<div className="space-y-2">
						<Label htmlFor="create-profile-description">
							{t('descriptionOptional', 'Description (optional)')}
						</Label>
						<Textarea
							id="create-profile-description"
							value={description}
							onChange={(e) => setDescription(e.target.value)}
							placeholder={t('shortDescription', 'Short description...')}
							rows={3}
						/>
					</div>
				</div>

				<DialogFooter>
					<DialogClose asChild>
						<Button variant="ghost">{t('cancel', 'Cancel')}</Button>
					</DialogClose>
					<Button
						onClick={handleCreate}
						disabled={!canCreate}
						className="flex items-center gap-2"
					>
						<Save className="h-4 w-4" />
						{t('create', 'Create')}
					</Button>
				</DialogFooter>
			</DialogContent>
		</Dialog>
	);
};
