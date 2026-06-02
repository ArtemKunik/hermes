import { createOpencodeClient } from "@opencode-ai/sdk/v2";

const TOKEN_THRESHOLD = 120_000;
const CHARS_PER_TOKEN = 3.5;
const TOKEN_POLL_MIN_INTERVAL_MS = 15_000;

// Track which (session, token-bucket) pairs have already triggered compaction.
const compactedBuckets = new Set<string>();

// Cache the model info per session to avoid API calls on every message transform.
const sessionModels = new Map<string, { providerID?: string; id?: string }>();

// Track sessions with pending compact requests to avoid duplicates.
const pendingCompacts = new Set<string>();
const lastTokenChecks = new Map<string, number>();

let v2Client: any;

function v2SessionApi() {
  return v2Client?.v2?.session ?? v2Client?.session;
}

function estimateTokens(messages: Array<{ info: { role?: string }; parts: Array<{ type?: string; text?: string }> }>): number {
  let totalChars = 0;
  for (const msg of messages) {
    if (!msg.info || !msg.parts) continue;
    for (const part of msg.parts) {
      if ((part.type === "text" || part.type === "reasoning") && typeof part.text === "string") {
        totalChars += part.text.length;
      }
    }
  }
  return Math.floor(totalChars / CHARS_PER_TOKEN);
}

function rememberModel(sessionID: string | undefined, model?: { providerID?: string; id?: string; modelID?: string }) {
  if (!sessionID || !model) return;
  sessionModels.set(sessionID, {
    ...(sessionModels.get(sessionID) || {}),
    providerID: model.providerID,
    id: model.id ?? model.modelID,
  });
}

function eventSessionID(event: any): string | undefined {
  return event?.properties?.sessionID ?? event?.sessionID ?? event?.payload?.properties?.sessionID;
}

function eventSession(event: any): any {
  return event?.properties ?? event?.session ?? event?.payload?.properties;
}

function sessionTokens(session: any): number {
  const tokens = session?.tokens;
  if (!tokens) return 0;
  return Number(tokens.input || 0)
    + Number(tokens.output || 0)
    + Number(tokens.reasoning || 0)
    + Number(tokens.cache?.read || 0)
    + Number(tokens.cache?.write || 0);
}

function isLMStudioSession(sessionID: string, session?: any): boolean {
  const cached = sessionModels.get(sessionID) || {};
  const provider = String(cached.providerID ?? session?.model?.providerID ?? session?.providerID ?? "").toLowerCase();
  const modelID = String(cached.id ?? session?.model?.id ?? session?.modelID ?? "").toLowerCase();
  return provider.includes("lmstudio") || modelID.includes("lmstudio") || provider === "lmstudio";
}

async function contextTokens(sessionID: string): Promise<number> {
  const now = Date.now();
  const last = lastTokenChecks.get(sessionID) ?? 0;
  if (now - last < TOKEN_POLL_MIN_INTERVAL_MS) return 0;
  lastTokenChecks.set(sessionID, now);

  const context = await v2SessionApi().context({ sessionID });
  const messages = Array.isArray(context) ? context : [];
  return estimateTokens(messages as any);
}

async function tryCompact(sessionID: string) {
  if (pendingCompacts.has(sessionID)) return;
  
  const bucket = Math.floor(Date.now() / 60000); // 1-minute bucket to avoid rapid re-triggers
  const key = `${sessionID}:${bucket}`;
  if (compactedBuckets.has(key)) return;

  pendingCompacts.add(sessionID);
  try {
    console.log(`[auto-compact] compacting session ${sessionID}`);
    await v2SessionApi().compact({ sessionID });
    compactedBuckets.add(key);
  } catch (err: any) {
    console.error(`[auto-compact] failed to compact session ${sessionID}:`, err?.message ?? err);
  } finally {
    pendingCompacts.delete(sessionID);
  }
}

export default async function (input?: { serverUrl?: URL; directory?: string }) {
  v2Client = createOpencodeClient({
    baseUrl: input?.serverUrl?.toString(),
    directory: input?.directory,
  });

  return {
    "chat.message": async (input: { sessionID: string; model?: { providerID: string; modelID: string } }): Promise<void> => {
      rememberModel(input.sessionID, input.model);
    },

    "chat.params": async (input: { sessionID: string; model?: { providerID: string; id: string } }): Promise<void> => {
      rememberModel(input.sessionID, input.model);
    },

    "experimental.chat.system.transform": async (input: { sessionID?: string; model?: { providerID: string; id: string } }): Promise<void> => {
      rememberModel(input.sessionID, input.model);
    },

    "experimental.chat.messages.transform": async (
      _input: {},
      output: { messages?: Array<{ info: { role?: string; sessionID?: string }; parts: Array<{ type?: string; text?: string }> }> }
    ): Promise<void> => {
      // DO NOT compact here - this fires mid-generation and causes model crashes.
      // Only track session IDs for later compaction.
      const messages = output.messages || [];
      
      let sessionID: string | undefined;
      for (const msg of messages) {
        if (msg.info?.sessionID) {
          sessionID = msg.info.sessionID;
          break;
        }
      }
      
      if (!sessionID) return;
      
      const totalTokens = estimateTokens(messages);
      if (totalTokens < TOKEN_THRESHOLD) return;
      
      // Store token count for this session to check later
      sessionModels.set(sessionID, {
        ...(sessionModels.get(sessionID) || {}),
        _pendingCompact: totalTokens >= TOKEN_THRESHOLD,
      } as any);
    },

    event: async (input: { event: any }): Promise<void> => {
      if (input.event?.type === "session.compacted") {
        const sessionID = eventSessionID(input.event);
        if (sessionID) {
          for (const key of Array.from(compactedBuckets)) {
            if (key.startsWith(`${sessionID}:`)) {
              compactedBuckets.delete(key);
            }
          }
          pendingCompacts.delete(sessionID);
          lastTokenChecks.delete(sessionID);
        }
      }

      if (input.event?.type === "session.updated") {
        const session = eventSession(input.event);
        const sessionID = session?.id ?? eventSessionID(input.event);
        if (sessionID && session?.model) rememberModel(sessionID, session.model);
      }
      
      // Trigger compact AFTER generation completes, not during
      if (
        input.event?.type === "message.assistant.completed" ||
        input.event?.type === "generation.completed" ||
        input.event?.type === "session.next.step.ended"
      ) {
        const sessionID = eventSessionID(input.event);
        if (!sessionID) return;

        if (!isLMStudioSession(sessionID)) return;

        let shouldCompact = Boolean((sessionModels.get(sessionID) as any)?._pendingCompact);
        if (!shouldCompact) {
          const eventTokens = sessionTokens(eventSession(input.event));
          if (eventTokens >= TOKEN_THRESHOLD) shouldCompact = true;
        }
        if (!shouldCompact) {
          try {
            shouldCompact = (await contextTokens(sessionID)) >= TOKEN_THRESHOLD;
          } catch (err: any) {
            console.error(`[auto-compact] failed to read context for ${sessionID}:`, err?.message ?? err);
          }
        }
        if (!shouldCompact) return;

        await tryCompact(sessionID);
      }
    },

    "session.compacted": async (): Promise<void> => {
      compactedBuckets.clear();
      pendingCompacts.clear();
    },
  };
}
