"use client";

import ChangeEmailDialog from "@flow-like/flow-like-ui/components/account/change-email-dialog";
import {
	confirmUserAttribute,
	sendUserAttributeVerificationCode,
	updateUserAttribute,
} from "aws-amplify/auth";

async function updateEmail(email: string) {
	const result = await updateUserAttribute({
		userAttribute: { attributeKey: "email", value: email },
	});
	return {
		needsVerification:
			result.nextStep.updateAttributeStep === "CONFIRM_ATTRIBUTE_WITH_CODE",
		destination: result.nextStep.codeDeliveryDetails?.destination,
	};
}

export default function AccountEmailDialog(props: {
	open: boolean;
	onOpenChange: (open: boolean) => void;
}) {
	return (
		<ChangeEmailDialog
			{...props}
			updateEmail={updateEmail}
			verifyEmail={(code) =>
				confirmUserAttribute({
					userAttributeKey: "email",
					confirmationCode: code,
				})
			}
			resendCode={() =>
				sendUserAttributeVerificationCode({ userAttributeKey: "email" })
			}
		/>
	);
}
