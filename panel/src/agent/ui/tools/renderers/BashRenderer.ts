// Adapted from Pi web UI 0.75.3 (MIT). See ../../LICENSE.
import type { ToolResultMessage } from "@earendil-works/pi-ai";
import "@mariozechner/mini-lit/dist/CodeBlock.js";
import { html } from "lit";
import type { ToolRenderer, ToolRenderResult } from "../types.js";

interface BashParams {
	command: string;
}

// CodeBlock accepts base64 and otherwise guesses; always encode so literal output
// such as "YWJj" cannot be mistaken for encoded text and silently changed.
function encodeCode(text: string): string {
	const bytes = new TextEncoder().encode(text);
	let binary = "";
	for (let offset = 0; offset < bytes.length; offset += 8192) {
		binary += String.fromCharCode(...bytes.subarray(offset, offset + 8192));
	}
	return btoa(binary);
}

// Bash tool has undefined details (only uses output)
export class BashRenderer implements ToolRenderer<BashParams, undefined> {
	render(params: BashParams | undefined, result: ToolResultMessage<undefined> | undefined, isStreaming?: boolean): ToolRenderResult {
		const output = result?.content
			?.filter((block) => block.type === "text")
			.map((block) => block.text)
			.join("\n") ?? "";
		return {
			content: html`
				<div class="space-y-3">
					<div class="bash-command">
						<div class="text-xs font-medium mb-1 text-muted-foreground">命令</div>
						${params?.command
							? html`<code-block .code=${encodeCode(params.command)} language="bash"></code-block>`
							: html`<div class="text-xs text-muted-foreground">${result ? "未记录命令参数" : "等待命令参数…"}</div>`}
					</div>
					<div class="bash-output">
						<div class="text-xs font-medium mb-1 text-muted-foreground">输出</div>
						${output
							? html`<code-block .code=${encodeCode(output)} language="text"></code-block>`
							: html`<div class="text-xs text-muted-foreground">${result ? "（无输出）" : isStreaming ? "等待命令输出…" : "未收到输出"}</div>`}
					</div>
				</div>
			`,
			isCustom: false,
		};
	}
}
