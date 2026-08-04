import { mountFlowWidget } from "@flow-like/widget-sdk";
import { StoreController } from "@nanostores/lit";
import { LitElement, css, html } from "lit";
import { customElement, state } from "lit/decorators.js";
import widget from "./widget.config";

const bridge = mountFlowWidget(widget);

@customElement("hello-widget")
export class HelloWidget extends LitElement {
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
	private count = bridge.$props.get().count;

	private lastHostCount = bridge.$props.get().count;
	private unbindProps?: () => void;
	private unbindQuery?: () => void;

	override connectedCallback() {
		super.connectedCallback();
		this.unbindProps = bridge.$props.subscribe((props) => {
			if (props.count !== this.lastHostCount) {
				this.lastHostCount = props.count;
				this.count = props.count;
			}
		});
		this.unbindQuery = bridge.onQuery("getCount", () => this.count);
	}

	override disconnectedCallback() {
		this.unbindProps?.();
		this.unbindQuery?.();
		super.disconnectedCallback();
	}

	private increase() {
		this.count += 1;
		bridge.emit("increased", { value: this.count });
	}

	override render() {
		return html`
			<h1>${this.props.value.title}</h1>
			<button type="button" @click=${this.increase}>
				Count: ${this.count}
			</button>
		`;
	}
}

document.getElementById("root")?.appendChild(new HelloWidget());
