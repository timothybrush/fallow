// Positive (Google GenAI, member-call): untrusted request input flowing into
// model.generateContent() is a prompt-injection candidate (CWE-1427).
import { GoogleGenerativeAI } from "@google/generative-ai";

const genai = new GoogleGenerativeAI("api-key");
const model = genai.getGenerativeModel({ model: "gemini-1.5-pro" });

export async function reply(req: { body: { message: string } }): Promise<string> {
  const userMessage = req.body.message;
  const result = await model.generateContent(userMessage);
  return result.response.text();
}
