// Positive (Anthropic, member-call): untrusted request input flowing into
// anthropic.messages.create() is a prompt-injection candidate (CWE-1427).
import Anthropic from "@anthropic-ai/sdk";

const anthropic = new Anthropic();

export async function summarize(req: { body: { text: string } }): Promise<string> {
  const userText = req.body.text;
  const message = await anthropic.messages.create({
    model: "claude-3-5-sonnet-latest",
    max_tokens: 1024,
    messages: [{ role: "user", content: userText }],
  });
  return JSON.stringify(message.content);
}
