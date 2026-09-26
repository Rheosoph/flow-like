import { mountFlowWidget } from "@flow-like/widget-sdk";
import { StoreController } from "@nanostores/lit";
import { LitElement, css, html } from "lit";
import { customElement, state } from "lit/decorators.js";
import {
	type WeatherState,
	createWeatherLoader,
	describeWeatherState,
} from "./weather";
import widget from "./widget.config";

const bridge = mountFlowWidget(widget);

@customElement("weather-widget")
export class WeatherWidget extends LitElement {
	static override styles = css`
		:host {
			display: grid;
			gap: 0.75rem;
			justify-items: start;
			padding: 1rem;
			color: var(--foreground);
			font-family: var(--font-sans, system-ui, sans-serif);
		}

		h1 {
			margin: 0;
			font-size: 1.25rem;
		}

		p {
			margin: 0;
			color: var(--muted-foreground);
		}

		button {
			background: var(--primary);
			color: var(--primary-foreground);
			border: 1px solid var(--border);
			border-radius: var(--radius);
			padding: 0.5rem 1rem;
			font: inherit;
			cursor: pointer;
		}
	`;

	private readonly props = new StoreController(this, bridge.$props);

	@state()
	private weather: WeatherState = { status: "loading" };

	private readonly loader = createWeatherLoader({
		onState: (next) => {
			this.weather = next;
		},
		onLoaded: (reading) => bridge.emit("loaded", reading),
	});

	private lastLocation = "";
	private unbindProps?: () => void;

	override connectedCallback() {
		super.connectedCallback();
		this.unbindProps = bridge.$props.subscribe((props) => {
			const location = `${props.latitude},${props.longitude}`;
			if (location !== this.lastLocation) {
				this.lastLocation = location;
				this.loader.load(props.latitude, props.longitude);
			}
		});
	}

	override disconnectedCallback() {
		this.unbindProps?.();
		this.loader.dispose();
		this.lastLocation = "";
		super.disconnectedCallback();
	}

	private refresh() {
		const { latitude, longitude } = bridge.$props.get();
		this.loader.load(latitude, longitude);
	}

	override render() {
		return html`
			<h1>${this.props.value.label}</h1>
			<p>${describeWeatherState(this.weather)}</p>
			<button type="button" @click=${this.refresh}>Refresh</button>
		`;
	}
}

document.getElementById("root")?.appendChild(new WeatherWidget());
