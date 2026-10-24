import { createI18n } from "@flow-like/locales";
import { loader } from "@monaco-editor/react";
import { createPortal } from "react-dom";
import { createRoot } from "react-dom/client";
import { Toaster } from "sonner";
import { DevModePanel } from "../../packages/ui/components/builder/DevModePanel";
import { Toolbar } from "../../packages/ui/components/builder/Toolbar";
import { BuilderFixture, qa } from "./builder-fixture";
import "./style.css";

createI18n({ language: "en" });
loader.config({ paths: { vs: "/monaco/0.54.0/vs" } });
const monaco = await loader.init();
Object.assign(window, { rawJsonQa: qa, monaco });
document.addEventListener("click", (event) => {
	if (
		event.target instanceof Element &&
		event.target.closest("[data-underlying]")
	) {
		qa.underlyingClicks++;
	}
});

function App() {
	return (
		<BuilderFixture>
			<Toaster />
			<main style={{ position: "relative", isolation: "isolate", zIndex: 0 }}>
				<h1>Page Builder</h1>
				<DevModePanel />
			</main>
			{createPortal(
				<>
					<div data-underlying="chrome" className="fixture-chrome">
						Underlying selectable page builder text across the editor's empty
						space
					</div>
					<div data-underlying="toolbar" className="fixture-toolbar">
						Underlying toolbar text
						<button type="button" onClick={() => qa.open()}>
							Open Raw JSON
						</button>
						<Toolbar />
					</div>
				</>,
				document.body,
			)}
		</BuilderFixture>
	);
}

const root = document.getElementById("root");
if (!root) throw new Error("Fixture root is missing");
createRoot(root).render(<App />);
