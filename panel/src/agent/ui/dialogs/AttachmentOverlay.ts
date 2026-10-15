// Adapted from Pi web UI 0.75.3 (MIT). See ../LICENSE.
import { icon } from "@mariozechner/mini-lit";
import { Button } from "@mariozechner/mini-lit/dist/Button.js";
import { html, LitElement } from "lit";
import { state } from "lit/decorators.js";
import { Download, X } from "lucide";
import type { Attachment } from "../utils/attachment-utils.js";

export class AttachmentOverlay extends LitElement {
	@state() private attachment?: Attachment;

	private onCloseCallback?: () => void;
	private boundHandleKeyDown?: (e: KeyboardEvent) => void;

	protected override createRenderRoot(): HTMLElement | DocumentFragment {
		return this;
	}

	static open(attachment: Attachment, onClose?: () => void) {
		const overlay = new AttachmentOverlay();
		overlay.attachment = attachment;
		overlay.onCloseCallback = onClose;
		document.body.appendChild(overlay);
		overlay.setupEventListeners();
	}

	private setupEventListeners() {
		this.boundHandleKeyDown = (e: KeyboardEvent) => {
			if (e.key === "Escape") {
				this.close();
			}
		};
		window.addEventListener("keydown", this.boundHandleKeyDown);
	}

	private close() {
		if (this.boundHandleKeyDown) {
			window.removeEventListener("keydown", this.boundHandleKeyDown);
		}
		this.onCloseCallback?.();
		this.remove();
	}

	private handleBackdropClick = () => {
		this.close();
	};

	private handleDownload = () => {
		if (!this.attachment) return;

		// Create a blob from the base64 content
		const byteCharacters = atob(this.attachment.content);
		const byteNumbers = new Array(byteCharacters.length);
		for (let i = 0; i < byteCharacters.length; i++) {
			byteNumbers[i] = byteCharacters.charCodeAt(i);
		}
		const byteArray = new Uint8Array(byteNumbers);
		const blob = new Blob([byteArray], { type: this.attachment.mimeType });

		// Create download link
		const url = URL.createObjectURL(blob);
		const a = document.createElement("a");
		a.href = url;
		a.download = this.attachment.fileName;
		document.body.appendChild(a);
		a.click();
		document.body.removeChild(a);
		URL.revokeObjectURL(url);
	};

	override render() {
		if (!this.attachment) return html``;

		return html`
			<!-- Full screen overlay -->
			<div class="fixed inset-0 bg-black/90 z-50 flex flex-col" @click=${this.handleBackdropClick}>
				<!-- Compact header bar -->
				<div class="bg-background/95 backdrop-blur border-b border-border" @click=${(e: Event) => e.stopPropagation()}>
					<div class="px-4 py-2 flex items-center justify-between">
						<div class="flex items-center gap-3 min-w-0">
							<span class="text-sm font-medium text-foreground truncate">${this.attachment.fileName}</span>
						</div>
						<div class="flex items-center gap-2">
							${Button({
								variant: "ghost",
								size: "icon",
								onClick: this.handleDownload,
								children: icon(Download, "sm"),
								className: "h-8 w-8",
							})}
							${Button({
								variant: "ghost",
								size: "icon",
								onClick: () => this.close(),
								children: icon(X, "sm"),
								className: "h-8 w-8",
							})}
						</div>
					</div>
				</div>

				<!-- Content container -->
				<div class="flex-1 flex items-center justify-center overflow-auto" @click=${(e: Event) => e.stopPropagation()}>
					${this.renderContent()}
				</div>
			</div>
		`;
	}

	private renderContent() {
		if (!this.attachment) return html``;
		return html`<img src="data:${this.attachment.mimeType};base64,${this.attachment.content}"
			alt=${this.attachment.fileName} class="max-w-full max-h-full object-contain" />`;
	}
}

// Register the custom element only once
if (!customElements.get("attachment-overlay")) {
	customElements.define("attachment-overlay", AttachmentOverlay);
}
