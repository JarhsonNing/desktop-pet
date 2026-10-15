import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import ts from "typescript";
const source = await readFile(new URL("../src/agent/stream.ts", import.meta.url), "utf8");
const code = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ES2022 } }).outputText;
const { streamUpdate } = await import(`data:text/javascript;base64,${Buffer.from(code).toString("base64")}`);
let message = { role: "assistant", content: [] };
for (const event of [
  { type: "text_start", contentIndex: 0 },
  { type: "text_delta", contentIndex: 0, delta: "中" },
  { type: "text_delta", contentIndex: 0, delta: "文" },
  { type: "text_end", contentIndex: 0, content: "中文完成" },
  { type: "thinking_start", contentIndex: 1 },
  { type: "thinking_delta", contentIndex: 1, delta: "思考" },
  { type: "toolcall_start", contentIndex: 2, id: "call", toolName: "read" },
  { type: "toolcall_delta", contentIndex: 2, delta: '{"path":' },
  { type: "toolcall_delta", contentIndex: 2, delta: '"fixture"}' },
]) message = streamUpdate(message, { assistantMessageEvent: event });
assert.equal(message.content[0].text, "中文完成");
assert.equal(message.content[1].thinking, "思考");
assert.deepEqual(message.content[2].arguments, { path: "fixture" });
const final = { role: "assistant", content: [{ type: "text", text: "权威内容" }] };
assert.equal(streamUpdate(message, { message: final }), final);
console.log("PASS Pi 1.2 delta-only presentation reconstruction");
