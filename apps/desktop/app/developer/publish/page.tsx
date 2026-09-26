"use client";

import {
	Button,
	Card,
	CardContent,
	CardDescription,
	CardHeader,
	CardTitle,
} from "@flow-like/flow-like-ui";
import { useTranslation } from "@flow-like/locales";
import { ArrowLeft, RefreshCw, Upload } from "lucide-react";
import Link from "next/link";
import { useRouter, useSearchParams } from "next/navigation";
import { Suspense } from "react";
import {
	PublishWizard,
	useOpenPublishedListing,
} from "../../../components/packages/workspace/publish-wizard";
import {
	ToolPageHeader,
	toolBackHref,
} from "../../../components/packages/workspace/tool-page-header";

function DeveloperPublishPageContent() {
	const { t } = useTranslation("common");
	const router = useRouter();
	const projectPath = useSearchParams().get("project");
	const openListing = useOpenPublishedListing(projectPath);
	const backHref = toolBackHref(projectPath);

	if (!projectPath) {
		return (
			<div className="flex-col flex grow items-center justify-center">
				<Card className="max-w-md">
					<CardHeader>
						<CardTitle>
							{t("noProjectSelected", "No Project Selected")}
						</CardTitle>
						<CardDescription>
							{t(
								"goBackToYourPackagesAndSelectOneToPublish",
								"Go back to your packages and select one to publish.",
							)}
						</CardDescription>
					</CardHeader>
					<CardContent>
						<Button variant="outline" asChild>
							<Link href={backHref}>
								<ArrowLeft className="mr-2 h-4 w-4" />
								{t("backToPackages", "Back to packages")}
							</Link>
						</Button>
					</CardContent>
				</Card>
			</div>
		);
	}

	return (
		<div className="flex-col flex grow max-h-full overflow-auto min-h-0 w-full">
			<div className="mx-auto w-full max-w-3xl space-y-6 pb-12">
				<ToolPageHeader
					icon={Upload}
					title={t("publishPackage", "Publish Package")}
					description={t(
						"publishYourLocalPackageNodesAndWidgetsAsAPrivateProjectToTheRegistry",
						"Publish your local package (nodes and widgets) as a private project to the registry",
					)}
					backHref={backHref}
				/>
				<PublishWizard
					projectPath={projectPath}
					onPublished={openListing}
					onCancel={() => router.push(backHref)}
				/>
			</div>
		</div>
	);
}

export default function DeveloperPublishPage() {
	return (
		<Suspense
			fallback={
				<div className="flex items-center justify-center h-full">
					<RefreshCw className="h-6 w-6 animate-spin text-muted-foreground/60" />
				</div>
			}
		>
			<DeveloperPublishPageContent />
		</Suspense>
	);
}
