"use client";

import { SuiteDetail } from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import { useSearchParams } from "next/navigation";

export default function Page() {
	const { t } = useTranslation("common");
	const searchParams = useSearchParams();
	const id = searchParams.get("id");

	if (!id) {
		return (
			<div className="p-10 text-center text-muted-foreground">
				{t("noSuiteSelected", "No suite selected.")}
			</div>
		);
	}

	return <SuiteDetail groupId={id} />;
}
