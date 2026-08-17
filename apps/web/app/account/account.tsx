"use client";

import { i18n as i18next, useTranslation } from "@flow-like/locales";
import {
	Avatar,
	AvatarFallback,
	AvatarImage,
	Badge,
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
	Input,
	Label,
	Separator,
	Textarea,
	useBackend,
	useInvoke,
	userAvatarUrl,
	userInitials,
} from "@flow-like/flow-like-ui";
import {
	CreditCard,
	Crown,
	Edit2,
	Eye,
	Lock,
	Upload,
	User,
} from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { toast } from "sonner";

export interface ProfileFormData {
	username: string;
	email: string;
	previewName: string;
	description: string;
	avatar: string;
}

export interface ProfileActions {
	updateEmail?: (email: string) => Promise<void>;
	handleAttributeUpdate?: (attribute: string, value: string) => Promise<void>;
	changePassword?: () => Promise<void>;
	viewBilling?: () => Promise<void>;
	previewProfile?: () => Promise<void>;
	viewSubscription?: () => Promise<void>;
}

interface ProfilePageProps {
	actions?: ProfileActions;
}

export function ProfilePage({ actions = {} }: Readonly<ProfilePageProps>) {
	const backend = useBackend();
	const [loading, setLoading] = useState(false);
	const info = useInvoke(backend.userState.getInfo, backend.userState, []);

	const [formData, setFormData] = useState<ProfileFormData>({
		username: "",
		email: "",
		previewName: "",
		description: "",
		avatar: "/placeholder.webp",
	});

	useEffect(() => {
		if (info.data) {
			setFormData({
				username: info.data.preferred_username ?? "",
				email: info.data.email ?? "",
				previewName: info.data.name ?? "",
				description: info.data.description ?? "",
				avatar: userAvatarUrl(info.data) ?? "/placeholder.webp",
			});
		}
	}, [info.data]);

	const hasChanges = useMemo(() => {
		return (
			formData.username !== info.data?.preferred_username ||
			formData.previewName !== info.data?.name ||
			formData.description !== info.data?.description
		);
	}, [formData, info.data]);

	const handleInputChange = useCallback(
		(field: keyof ProfileFormData, value: string) => {
			setFormData((prev) => ({ ...prev, [field]: value }));
		},
		[],
	);

	const handleInlineFieldUpdate = useCallback(
		async (field: keyof ProfileFormData, value: any) => {
			setFormData((prev) => ({ ...prev, [field]: value }));
		},
		[actions],
	);

	const handleAvatarUpload = useCallback(
		async (event: React.ChangeEvent<HTMLInputElement>) => {
			if (!event.target.files || event.target.files.length === 0) return;
			const file = event.target.files?.[0];
			if (!file) return;
			try {
				setLoading(true);
				await backend.userState.updateUser({}, file);
				setFormData((prev) => ({
					...prev,
					avatar: URL.createObjectURL(file),
				}));
				toast.success("Avatar updated successfully");
			} catch (error) {
				console.error("Failed to upload avatar:", error);
				toast.error("Failed to upload avatar");
			} finally {
				setLoading(false);
			}

			await info.refetch();
		},
		[backend.userState, info.refetch],
	);

	const handleSave = useCallback(async () => {
		const ref = toast.loading("Saving profile...");
		try {
			setLoading(true);

			if (
				formData.username !== info.data?.preferred_username &&
				actions.handleAttributeUpdate
			) {
				await actions.handleAttributeUpdate(
					"preferred_username",
					formData.username,
				);
			} else {
				console.log(
					"No username change detected, skipping update",
					formData.username,
					info.data?.preferred_username,
					typeof actions.handleAttributeUpdate,
				);
			}

			await backend.userState.updateUser({
				name: formData.previewName,
				description: formData.description,
			});

			await info.refetch();

			toast.success("Profile saved successfully");
		} catch (error) {
			console.error("Failed to save profile:", error);
			toast.error("Failed to save profile");
		} finally {
			setLoading(false);
			toast.dismiss(ref);
		}
	}, [
		formData,
		info.data,
		backend.userState,
		actions.handleAttributeUpdate,
		info.refetch,
	]);

	return (
		<div className="container max-w-4xl mx-auto p-6 space-y-6">
			<ProfileHeader />

			<div className="grid gap-6 md:grid-cols-3">
				<div className="md:col-span-1 space-y-6">
					<AvatarCard
						avatar={formData.avatar}
						previewName={formData.previewName}
						onAvatarUpload={handleAvatarUpload}
						canEdit={true}
					/>

					<SubscriptionCard
						tier={info.data?.tier}
						onViewSubscription={actions.viewSubscription}
						onViewBilling={actions.viewBilling}
					/>
				</div>

				<div className="md:col-span-2 space-y-6">
					<PersonalInfoCard
						formData={formData}
						onInputChange={handleInputChange}
						onInlineFieldUpdate={handleInlineFieldUpdate}
						onEmailClick={actions.updateEmail}
					/>

					<SecurityCard onChangePassword={actions.changePassword} />

					<ActionButtons
						onSave={handleSave}
						onViewBilling={actions.viewBilling}
						onPreviewProfile={actions.previewProfile}
						isLoading={loading}
						hasChanges={hasChanges}
					/>
				</div>
			</div>
		</div>
	);
}

const ProfileHeader: React.FC = () => (
	<div className="space-y-2">
		<h1 className="text-3xl font-bold tracking-tight">{i18next.t('profileSettings', 'Profile Settings')}</h1>
		<p className="text-muted-foreground">
			{i18next.t('manageYourAccountSettingsAndPreferences', 'Manage your account settings and preferences')}
		</p>
	</div>
);

interface AvatarCardProps {
	avatar: string;
	previewName: string;
	onAvatarUpload: (event: React.ChangeEvent<HTMLInputElement>) => void;
	canEdit: boolean;
}

const AvatarCard: React.FC<AvatarCardProps> = ({
	avatar,
	previewName,
	onAvatarUpload,
	canEdit,
}) => (
	<Card>
		<CardHeader>
			<CardTitle className="flex items-center gap-2">
				<User className="h-5 w-5" />
				{i18next.t('profilePicture', 'Profile Picture')}
			</CardTitle>
		</CardHeader>
		<CardContent className="space-y-4">
			<div className="flex flex-col items-center space-y-4">
				<Avatar className="h-24 w-24">
					<AvatarImage src={avatar} alt={previewName} />
					<AvatarFallback className="text-lg">
						{userInitials(previewName)}
					</AvatarFallback>
				</Avatar>

				<div className="text-center">
					<p className="font-medium">{previewName}</p>
				</div>

				{canEdit && (
					<div className="w-full flex justify-center">
						<Label htmlFor="avatar-upload" className="cursor-pointer">
							<div className="flex items-center gap-2 rounded-md border border-dashed p-4 hover:bg-muted transition-colors">
								<Upload className="h-4 w-4" />
								<span className="text-sm">{`Upload new photo`}</span>
							</div>
						</Label>
						<input
							id="avatar-upload"
							type="file"
							accept="image/*"
							onChange={onAvatarUpload}
							className="hidden"
						/>
					</div>
				)}
			</div>
		</CardContent>
	</Card>
);

interface PersonalInfoCardProps {
	formData: ProfileFormData;
	onInputChange: (field: keyof ProfileFormData, value: string) => void;
	onInlineFieldUpdate: (
		field: keyof ProfileFormData,
		value: string,
	) => Promise<void>;
	onEmailClick?: (email: string) => Promise<void>;
}

const PersonalInfoCard: React.FC<PersonalInfoCardProps> = ({
	formData,
	onInputChange,
	onInlineFieldUpdate,
	onEmailClick,
}) => (
	<Card>
		<CardHeader>
			<CardTitle>{i18next.t('personalInformation', 'Personal Information')}</CardTitle>
			<CardDescription>
				{i18next.t('updateYourPersonalDetailsAndProfileInformation', 'Update your personal details and profile information')}
			</CardDescription>
		</CardHeader>
		<CardContent className="space-y-4">
			<div className="grid gap-4 md:grid-cols-2">
				<div className="space-y-2">
					<Label htmlFor="username">{i18next.t('username2', 'Username')}</Label>
					<Input
						id="username"
						value={formData.username}
						onChange={(e) => onInputChange("username", e.target.value)}
						placeholder={i18next.t('enterUsername', 'Enter username')}
					/>
				</div>

				<div className="space-y-2">
					<Label htmlFor="previewName">{i18next.t('displayName', 'Display Name')}</Label>
					<Input
						id="previewName"
						value={formData.previewName}
						onChange={(e) => onInputChange("previewName", e.target.value)}
						placeholder={i18next.t('enterDisplayName', 'Enter display name')}
					/>
				</div>
			</div>

			<div className="space-y-2">
				<Label htmlFor="email">{i18next.t('email', 'Email')}</Label>
				<div className="flex gap-2">
					<Input
						id="email"
						type="email"
						value={formData.email}
						placeholder={i18next.t('enterEmail', 'Enter email')}
						disabled
						className="bg-muted"
					/>
					{onEmailClick && (
						<Button
							variant="outline"
							size="sm"
							onClick={() => onEmailClick(formData.email)}
							className="shrink-0"
						>
							<Edit2 className="h-4 w-4" />
						</Button>
					)}
				</div>
			</div>

			<div className="space-y-2">
				<Label htmlFor="description">{i18next.t('profileDescription', 'Profile Description')}</Label>
				<Textarea
					id="description"
					value={formData.description}
					onChange={(e) => onInputChange("description", e.target.value)}
					placeholder={i18next.t('tellUsAboutYourself', 'Tell us about yourself...')}
					className="min-h-[100px] resize-none"
					maxLength={2000}
				/>
				<div className="flex justify-between items-center text-xs text-muted-foreground">
					<span>{i18next.t('maximum2000Characters', 'Maximum 2000 characters')}</span>
					<span>{`${formData.description.length}/2000`}</span>
				</div>
			</div>
		</CardContent>
	</Card>
);

const TIER_COLORS: Record<string, string> = {
	FREE: "bg-muted text-muted-foreground",
	PREMIUM: "bg-gradient-to-r from-amber-500 to-orange-600 text-white",
	PRO: "bg-gradient-to-r from-violet-500 to-purple-600 text-white",
	ENTERPRISE: "bg-gradient-to-r from-blue-500 to-indigo-600 text-white",
};

interface SubscriptionCardProps {
	tier?: string;
	onViewSubscription?: () => Promise<void>;
	onViewBilling?: () => Promise<void>;
}

const SubscriptionCard: React.FC<SubscriptionCardProps> = ({
	tier,
	onViewSubscription,
	onViewBilling,
}) => {
	const { t } = useTranslation("common");
	if (!onViewSubscription && !onViewBilling) return null;

	const tierKey = tier?.toUpperCase() || "FREE";
	const colorClass = TIER_COLORS[tierKey] || TIER_COLORS.FREE;

	return (
		<Card>
			<CardHeader>
				<CardTitle className="flex items-center gap-2">
					<Crown className="h-5 w-5" />
					{t('subscription', 'Subscription')}
				</CardTitle>
			</CardHeader>
			<CardContent className="space-y-4">
				<div className="flex items-center justify-between">
					<span className="text-sm text-muted-foreground">{t('currentPlan', 'Current Plan')}</span>
					<Badge className={colorClass}>{tierKey}</Badge>
				</div>
				<div className="flex flex-col gap-2">
					{onViewSubscription && (
						<Button
							variant="outline"
							className="w-full"
							onClick={onViewSubscription}
						>
							{t('viewPlans', 'View Plans')}
						</Button>
					)}
					{onViewBilling && tierKey !== "FREE" && (
						<Button
							variant="ghost"
							size="sm"
							className="w-full"
							onClick={onViewBilling}
						>
							<CreditCard className="h-4 w-4 mr-2" />
							{t('manageBilling', 'Manage Billing')}
						</Button>
					)}
				</div>
			</CardContent>
		</Card>
	);
};

interface SecurityCardProps {
	onChangePassword?: () => Promise<void>;
}

const SecurityCard: React.FC<SecurityCardProps> = ({ onChangePassword }) => {
	const { t } = useTranslation("common");
	if (onChangePassword)
		return (
			<Card>
				<CardHeader>
					<CardTitle className="flex items-center gap-2">
						<Lock className="h-5 w-5" />
						{t('security', 'Security')}
					</CardTitle>
					<CardDescription>
						{t('manageYourPasswordAndSecuritySettings', 'Manage your password and security settings')}
					</CardDescription>
				</CardHeader>
				<CardContent>
					{onChangePassword && (
						<Button
							variant="outline"
							className="w-full"
							onClick={onChangePassword}
						>
							{t('changePassword', 'Change Password')}
						</Button>
					)}
				</CardContent>
			</Card>
		);

	return null;
};

interface ActionButtonsProps {
	onSave?: () => Promise<void>;
	onViewBilling?: () => Promise<void>;
	onPreviewProfile?: () => Promise<void>;
	isLoading: boolean;
	hasChanges?: boolean;
}

const ActionButtons: React.FC<ActionButtonsProps> = ({
	onSave,
	onViewBilling,
	onPreviewProfile,
	isLoading,
	hasChanges,
}) => (
	<div className="flex flex-col sm:flex-row gap-4">
		{onSave && (
			<Button
				onClick={onSave}
				disabled={isLoading || !hasChanges}
				className="flex-1"
			>
				{isLoading ? "Saving..." : i18next.t('saveChanges', 'Save Changes')}
			</Button>
		)}

		{(onViewBilling || onPreviewProfile) && onSave && (
			<Separator orientation="vertical" className="hidden sm:block h-auto" />
		)}

		{onViewBilling && (
			<Button
				variant="outline"
				className="flex items-center gap-2"
				onClick={onViewBilling}
			>
				<CreditCard className="h-4 w-4" />
				{i18next.t('billingSettings', 'Billing Settings')}
			</Button>
		)}

		{onPreviewProfile && (
			<Button
				variant="outline"
				className="flex items-center gap-2"
				onClick={onPreviewProfile}
			>
				<Eye className="h-4 w-4" />
				{i18next.t('previewProfile', 'Preview Profile')}
			</Button>
		)}
	</div>
);
