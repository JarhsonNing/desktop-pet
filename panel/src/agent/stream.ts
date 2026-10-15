// Pi 1.2 RPC omits cumulative message snapshots. Rebuild only the live presentation.
export function streamUpdate(partial: any, record: any): any {
  if (record.message) return record.message;
  const event = record.assistantMessageEvent;
  if (!event) return partial;
  if (event.message || event.error) return event.message || event.error;
  if (!partial) return null;
  if (record.usage) partial.usage = record.usage;
  const index = event.contentIndex;
  if (!Number.isInteger(index) || index < 0 || index >= 10000) return partial;
  const blocks = partial.content;
  switch (event.type) {
    case "text_start": blocks[index] = { type: "text", text: "" }; break;
    case "thinking_start": blocks[index] = { type: "thinking", thinking: "" }; break;
    case "toolcall_start": blocks[index] = { type: "toolCall", id: event.id, name: event.toolName, arguments: {}, _mochiArguments: "" }; break;
    case "text_delta": if (blocks[index]) blocks[index].text += event.delta; break;
    case "thinking_delta": if (blocks[index]) blocks[index].thinking += event.delta; break;
    case "toolcall_delta":
      if (blocks[index]) {
        blocks[index]._mochiArguments += event.delta;
        try { blocks[index].arguments = JSON.parse(blocks[index]._mochiArguments); } catch { /* Partial JSON remains buffered until complete. */ }
      }
      break;
    case "text_end": blocks[index].text = event.content; break;
    case "thinking_end": blocks[index].thinking = event.content; break;
    case "toolcall_end": blocks[index] = event.toolCall; break;
  }
  return partial;
}
