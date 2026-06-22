// Negative (LangChain .invoke): `*.invoke` is a deliberately EXCLUDED callee
// (too generic a member name: RxJS, validators, Function.prototype.call, etc.),
// so even with untrusted input flowing in this must NOT fire. Documented blind
// spot, mirroring the `*.find` exclusion in nosql-injection / xpath-injection.
import { ChatOpenAI } from "@langchain/openai";

const chain = new ChatOpenAI({ model: "gpt-4o" });

export async function run(req: { body: { prompt: string } }): Promise<unknown> {
  const userPrompt = req.body.prompt;
  return chain.invoke(userPrompt);
}
