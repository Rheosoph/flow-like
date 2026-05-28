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
	useBackend,
	useInvoke,
} from "@flow-like/flow-like-ui";
import {
	confirmUserAttribute,
	sendUserAttributeVerificationCode,
	updateUserAttribute,
} from "aws-amplify/auth";
import { Mail } from "lucide-react";
import { useCallback, useState } from "react";

interface ChangeEmailDialogProps {
	open: boolean;
	onOpenChange: (open: boolean) => void;
}

const ChangeEmailDialog: React.FC<ChangeEmailDialogProps> = ({
	open,
	onOpenChange,
}) => {
	const backend = useBackend();
	const info = useInvoke(backend.userState.getInfo, backend.userState, []);

	const [step, setStep] = useState<"email" | "verification">("email");
	const [formData, setFormData] = useState({
		newEmail: "",
		confirmationCode: "",
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

	const validateEmail = useCallback((email: string) => {
		const emailRegex = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
		return emailRegex.test(email);
	}, []);

	const handleEmailSubmit = useCallback(async () => {
		if (!formData.newEmail) {
			setError("Email is required");
			return;
		}
		if (!validateEmail(formData.newEmail)) {
			setError("Please enter a valid email address");
			return;
		}
		if (formData.newEmail === info.data?.email) {
			setError("New email must be different from current email");
			return;
		}

		const output = await updateUserAttribute({
			userAttribute: {
				attributeKey: "email",
				value: formData.newEmail,
			},
		});

		switch (output.nextStep.updateAttributeStep) {
			case "CONFIRM_ATTRIBUTE_WITH_CODE":
				setStep("verification");
				break;
			case "DONE":
				setFormData({ newEmail: "", confirmationCode: "" });
				await info.refetch();
				handleClose();
				return;
		}
	}, [formData.newEmail, info.data, validateEmail]);

	const handleVerificationSubmit = useCallback(async () => {
		if (!formData.confirmationCode) {
			setError("Confirmation code is required");
			return;
		}

		try {
			setIsLoading(true);
			setError("");
			await confirmUserAttribute({
				confirmationCode: formData.confirmationCode,
				userAttributeKey: "email",
			});
			await info.refetch();
			handleClose();
		} catch (error) {
			console.error("Failed to confirm email change:", error);
			setError("Invalid confirmation code. Please try again.");
		} finally {
			setIsLoading(false);
		}
	}, [formData]);

	const handleClose = useCallback(() => {
		setStep("email");
		setFormData({ newEmail: "", confirmationCode: "" });
		setError("");
		onOpenChange(false);
	}, [onOpenChange]);

	const handleBack = useCallback(() => {
		setStep("email");
		setError("");
	}, []);

	return (
		<Dialog open={open} onOpenChange={handleClose}>
			<DialogContent className="sm:max-w-md">
				<DialogHeader>
					<DialogTitle className="flex items-center gap-2">
						<Mail className="h-5 w-5" />
						Change Email Address
					</DialogTitle>
					<DialogDescription>
						{step === "email"
							? "Enter your new email address"
							: "Enter the confirmation code sent to your new email"}
					</DialogDescription>
				</DialogHeader>

				<div className="space-y-4">
					{error && (
						<Alert variant="destructive">
							<AlertDescription>{error}</AlertDescription>
						</Alert>
					)}

					{step === "email" ? (
						<>
							<div className="space-y-2">
								<Label htmlFor="current-email">Current Email</Label>
								<Input
									id="current-email"
									value={info.data?.email}
									disabled
									className="bg-muted"
								/>
							</div>

							<div className="space-y-2">
								<Label htmlFor="new-email">New Email Address</Label>
								<Input
									id="new-email"
									type="email"
									value={formData.newEmail}
									onChange={(e) =>
										handleInputChange("newEmail", e.target.value)
									}
									placeholder="Enter new email address"
								/>
							</div>

							<div className="flex gap-2 pt-4">
								<Button
									variant="outline"
									onClick={handleClose}
									className="flex-1"
								>
									Cancel
								</Button>
								<Button onClick={handleEmailSubmit} className="flex-1">
									Continue
								</Button>
							</div>
						</>
					) : (
						<>
							<div className="space-y-2">
								<Label htmlFor="new-email-display">New Email</Label>
								<Input
									id="new-email-display"
									value={formData.newEmail}
									disabled
									className="bg-muted"
								/>
							</div>

							<div className="space-y-2">
								<Label htmlFor="confirmation-code">Confirmation Code</Label>
								<Input
									id="confirmation-code"
									value={formData.confirmationCode}
									onChange={(e) =>
										handleInputChange("confirmationCode", e.target.value)
									}
									placeholder="Enter confirmation code"
								/>
								<p className="text-sm text-muted-foreground">
									Check your email for a confirmation code
								</p>
							</div>

							<div className="flex gap-2 pt-4">
								<Button
									variant="outline"
									onClick={async () => {
										await sendUserAttributeVerificationCode({
											userAttributeKey: "email",
										});
									}}
									className="flex-1"
								>
									Resend
								</Button>
								<Button
									variant="outline"
									onClick={handleBack}
									className="flex-1"
								>
									Back
								</Button>
								<Button
									onClick={handleVerificationSubmit}
									disabled={isLoading}
									className="flex-1"
								>
									{isLoading ? "Verifying..." : "Verify & Change"}
								</Button>
							</div>
						</>
					)}
				</div>
			</DialogContent>
		</Dialog>
	);
};

export default ChangeEmailDialog;
