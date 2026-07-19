import { describe, expect, test } from "bun:test";
import { renderToStaticMarkup } from "react-dom/server";
import { ChatAppearance } from "./appearance";

describe("ChatAppearance", () => {
	test("applies event appearance and keeps custom CSS inside its instance", () => {
		const markup = renderToStaticMarkup(
			<ChatAppearance
				appId="app-id"
				eventId="chat-event"
				config={{
					background_image: "https://example.com/background.webp",
					color_scheme: "dark",
					custom_css:
						':root { --primary: hotpink; } [data-fl-chat-message="assistant"] { opacity: .9; }',
				}}
			>
				<div data-fl-chat-message="assistant">Hello</div>
			</ChatAppearance>,
		);

		expect(markup).toContain('data-fl-chat-color-scheme="dark"');
		expect(markup).toContain("https://example.com/background.webp");
		expect(markup).toContain("--primary: hotpink");
		expect(markup).toContain(
			'[data-fl-chat-message="assistant"] { opacity: .9; }',
		);
		expect(markup).not.toContain(":root");
	});

	test("does not send an unresolved storage path to the browser", () => {
		const markup = renderToStaticMarkup(
			<ChatAppearance
				appId="app-id"
				eventId="chat-event"
				config={{ background_image: "images/private-background.webp" }}
			>
				<div>Hello</div>
			</ChatAppearance>,
		);

		expect(markup).not.toContain("images/private-background.webp");
		expect(markup).not.toContain("data-fl-chat-has-background");
	});
});
