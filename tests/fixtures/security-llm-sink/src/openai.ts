// Positive (OpenAI, member-call): untrusted request input flowing into
// openai.chat.completions.create() is a prompt-injection candidate (CWE-1427).
// `req.body` is an HTTP-request-input source (receiver `req` is allowlisted), so
// the taint gate (requires_source) fires.
import OpenAI from "openai";

const openai = new OpenAI();

export async function ask(req: { body: { prompt: string } }): Promise<string> {
  const userPrompt = req.body.prompt;
  const completion = await openai.chat.completions.create({
    model: "gpt-4o",
    messages: [{ role: "user", content: userPrompt }],
  });
  return completion.choices[0]?.message?.content ?? "";
}
