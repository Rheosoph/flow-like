"use client";

import {
	Alert,
	AlertDescription,
	Button,
	Dialog,
	DialogContent,
	DialogDescription,
	DialogHeader,
	DialogTitle,
	Input,
	Label,
} from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import { updatePassword } from "aws-amplify/auth";
import { Eye, EyeOff } from "lucide-react";
import { useCallback, useState } from "react";

interface ChangePasswordDialogProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
	onPasswordChange: (
		currentPassword: string,
		newPassword: string,
	) => Promise<void>;
}

const ChangePasswordDialog: React.FC<ChangePasswordDialogProps> = ({
	open,
	onOpenChange,
	onPasswordChange,
}) => {
	const { t } = useTranslation("common");
	const [formData, setFormData] = useState({
		currentPassword: "",
		newPassword: "",
		confirmPassword: "",
	});
	const [showPasswords, setShowPasswords] = useState({
		current: false,
		new: false,
		confirm: false,
	});
	const [isLoading, setIsLoading] = useState(false);
	const [error, setError] = useState("");

	const handleInputChange = useCallback(
		(field: keyof typeof formData, value: string) => {
			setFormData((prev) => ({ ...prev, [field]: value }));
			setError("");
		},
		[],
	);

	const togglePasswordVisibility = useCallback(
		(field: keyof typeof showPasswords) => {
			setShowPasswords((prev) => ({ ...prev, [field]: !prev[field] }));
		},
		[],
	);

	const validateForm = useCallback(() => {
		if (!formData.currentPassword) {
			setError("Current password is required");
			return false;
		}
		if (!formData.newPassword) {
			setError("New password is required");
			return false;
		}
		if (formData.newPassword.length < 8) {
			setError("New password must be at least 8 characters long");
			return false;
		}
		if (formData.newPassword !== formData.confirmPassword) {
			setError("New passwords do not match");
			return false;
		}
		if (formData.currentPassword === formData.newPassword) {
			setError("New password must be different from current password");
			return false;
		}
		return true;
	}, [formData]);

	const handleSubmit = useCallback(async () => {
		if (!validateForm()) return;

		try {
			setIsLoading(true);
			setError("");
			await updatePassword({
				newPassword: formData.newPassword,
				oldPassword: formData.currentPassword,
			});
			setFormData({
				currentPassword: "",
				newPassword: "",
				confirmPassword: "",
			});
		} catch (error) {
			console.error("Failed to change password:", error);
			setError(
				t(
					"failedToChangePasswordPleaseCheckYourCurrentPassword",
					"Failed to change password. Please check your current password.",
				),
			);
		} finally {
			setIsLoading(false);
		}
	}, [formData, onPasswordChange, validateForm]);

	const handleClose = useCallback(() => {
		setFormData({ currentPassword: "", newPassword: "", confirmPassword: "" });
		setError("");
		setShowPasswords({ current: false, new: false, confirm: false });
		onOpenChange(false);
	}, [onOpenChange]);

	return (
		<Dialog open={open} onOpenChange={handleClose}>
			<DialogContent className="sm:max-w-md">
				<DialogHeader>
					<DialogTitle>{t("changePassword", "Change Password")}</DialogTitle>
					<DialogDescription>
						{`Enter your current password and choose a new one.`}
					</DialogDescription>
				</DialogHeader>

				<div className="space-y-4">
					{error && (
						<Alert variant="destructive">
							<AlertDescription>{error}</AlertDescription>
						</Alert>
					)}

					<div className="space-y-2">
						<Label htmlFor="current-password">
							{t("currentPassword", "Current Password")}
						</Label>
						<div className="relative">
							<Input
								id="current-password"
								type={showPasswords.current ? "text" : "password"}
								value={formData.currentPassword}
								onChange={(e) =>
									handleInputChange("currentPassword", e.target.value)
								}
								placeholder={t(
									"enterCurrentPassword",
									"Enter current password",
								)}
							/>
							<Button
								type="button"
								variant="ghost"
								size="sm"
								className="absolute right-0 top-0 h-full px-3 py-2 hover:bg-transparent"
								onClick={() => togglePasswordVisibility("current")}
							>
								{showPasswords.current ? (
									<EyeOff className="h-4 w-4" />
								) : (
									<Eye className="h-4 w-4" />
								)}
							</Button>
						</div>
					</div>

					<div className="space-y-2">
						<Label htmlFor="new-password">
							{t("newPassword", "New Password")}
						</Label>
						<div className="relative">
							<Input
								id="new-password"
								type={showPasswords.new ? "text" : "password"}
								value={formData.newPassword}
								onChange={(e) =>
									handleInputChange("newPassword", e.target.value)
								}
								placeholder={`Enter new password`}
							/>
							<Button
								type="button"
								variant="ghost"
								size="sm"
								className="absolute right-0 top-0 h-full px-3 py-2 hover:bg-transparent"
								onClick={() => togglePasswordVisibility("new")}
							>
								{showPasswords.new ? (
									<EyeOff className="h-4 w-4" />
								) : (
									<Eye className="h-4 w-4" />
								)}
							</Button>
						</div>
					</div>

					<div className="space-y-2">
						<Label htmlFor="confirm-password">
							{t("confirmNewPassword", "Confirm New Password")}
						</Label>
						<div className="relative">
							<Input
								id="confirm-password"
								type={showPasswords.confirm ? "text" : "password"}
								value={formData.confirmPassword}
								onChange={(e) =>
									handleInputChange("confirmPassword", e.target.value)
								}
								placeholder={`Confirm new password`}
							/>
							<Button
								type="button"
								variant="ghost"
								size="sm"
								className="absolute right-0 top-0 h-full px-3 py-2 hover:bg-transparent"
								onClick={() => togglePasswordVisibility("confirm")}
							>
								{showPasswords.confirm ? (
									<EyeOff className="h-4 w-4" />
								) : (
									<Eye className="h-4 w-4" />
								)}
							</Button>
						</div>
					</div>

					<div className="flex gap-2 pt-4">
						<Button variant="outline" onClick={handleClose} className="flex-1">
							{t("cancel", "Cancel")}
						</Button>
						<Button
							onClick={handleSubmit}
							disabled={isLoading}
							className="flex-1"
						>
							{isLoading ? "Changing..." : "Change Password"}
						</Button>
					</div>
				</div>
			</DialogContent>
		</Dialog>
	);
};

export default ChangePasswordDialog;
