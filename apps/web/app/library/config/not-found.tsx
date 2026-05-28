"use client";

import { Button } from "@flow-like/flow-like-ui";
import { ArrowBigLeftIcon } from "lucide-react";

export default function NotFound() {
	return (
		<main className="w-full h-full flex-grow flex flex-col items-center justify-center">
			<img src="/404.svg" alt="404" className="w-56 h-56 dark:h-0 dark:w-0" />
			<img
				src="/404_dark.svg"
				alt="404"
				className="w-0 h-0 dark:w-56 dark:h-56"
			/>
			<h1>Coming soon...</h1>
			<Button
				className="mt-4"
				onClick={() => {
					window.history.back();
				}}
			>
				<ArrowBigLeftIcon /> Go Back
			</Button>
		</main>
	);
}
