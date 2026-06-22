// Negative (constant prompt): an LLM call whose prompt is a hardcoded constant,
// with NO untrusted source flowing in, must NOT fire. The taint gate
// (requires_source) keeps this quiet.
import OpenAI from "openai";
import { generateText } from "ai";
import { openai as aiOpenai } from "@ai-sdk/openai";

const openai = new OpenAI();

export async function tagline(): Promise<string> {
  const completion = await openai.chat.completions.create({
    model: "gpt-4o",
    messages: [{ role: "user", content: "Write a one-line tagline for a static site." }],
  });
  return completion.choices[0]?.message?.content ?? "";
}

export async function staticPrompt(): Promise<string> {
  const { text } = await generateText({
    model: aiOpenai("gpt-4o"),
    prompt: "Summarize the project README in one sentence.",
  });
  return text;
}
