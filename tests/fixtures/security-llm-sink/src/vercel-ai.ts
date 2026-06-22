// Positive (Vercel AI SDK, bare call): untrusted request input flowing into the
// top-level generateText() prompt is a prompt-injection candidate (CWE-1427).
import { generateText } from "ai";
import { openai } from "@ai-sdk/openai";

export async function complete(req: { body: { question: string } }): Promise<string> {
  const userQuestion = req.body.question;
  const { text } = await generateText({
    model: openai("gpt-4o"),
    prompt: userQuestion,
  });
  return text;
}
