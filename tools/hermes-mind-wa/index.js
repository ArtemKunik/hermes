'use strict';

// hermes-mind-wa — Baileys WhatsApp sidecar
//
// Usage:
//   node index.js --creds <dir>            stream new messages forever
//   node index.js --creds <dir> --auth-only  QR-auth then exit (first-run setup)
//   node index.js --creds <dir> --since <unix_ts>  only emit messages after timestamp
//
// stdout : newline-delimited JSON events (one per message)
// stderr : logs, QR code, connection status

const {
  default: makeWASocket,
  useMultiFileAuthState,
  DisconnectReason,
  fetchLatestBaileysVersion,
} = require('@whiskeysockets/baileys');
const qrcode = require('qrcode-terminal');
const pino = require('pino');
const fs = require('fs');
const path = require('path');

// ── CLI args ──────────────────────────────────────────────────────────────────

const args = process.argv.slice(2);
const flag = (name) => {
  const i = args.indexOf(name);
  return i !== -1 ? args[i + 1] : null;
};
const hasFlag = (name) => args.includes(name);

const credsDir = flag('--creds') || path.join(process.env.HOME || '.', '.hermes-mind', 'whatsapp');
const sinceTs  = flag('--since') ? parseInt(flag('--since'), 10) : 0;
const authOnly = hasFlag('--auth-only');

fs.mkdirSync(credsDir, { recursive: true });

// ── helpers ───────────────────────────────────────────────────────────────────

const log  = (msg) => process.stderr.write(`[hermes-mind-wa] ${msg}\n`);
const emit = (obj) => process.stdout.write(JSON.stringify(obj) + '\n');

function extractText(msg) {
  return (
    msg.conversation ||
    msg.extendedTextMessage?.text ||
    msg.imageMessage?.caption ||
    msg.videoMessage?.caption ||
    msg.documentMessage?.title ||
    null
  );
}

function mediaType(msg) {
  if (msg.imageMessage)    return 'image';
  if (msg.videoMessage)    return 'video';
  if (msg.audioMessage)    return 'audio';
  if (msg.documentMessage) return 'document';
  if (msg.stickerMessage)  return 'sticker';
  return null;
}

// ── connection ────────────────────────────────────────────────────────────────

async function connect() {
  const { state, saveCreds } = await useMultiFileAuthState(credsDir);
  const { version } = await fetchLatestBaileysVersion();
  log(`using Baileys ${version.join('.')}`);

  const sock = makeWASocket({
    version,
    auth: state,
    printQRInTerminal: false,                    // we render to stderr ourselves
    logger: pino({ level: 'silent' }),           // suppress Baileys internal logs
    syncFullHistory: false,
    markOnlineOnConnect: false,
  });

  sock.ev.on('creds.update', saveCreds);

  // ── connection state ───────────────────────────────────────────────────────

  sock.ev.on('connection.update', ({ connection, lastDisconnect, qr }) => {
    if (qr) {
      log('scan QR code to authenticate:');
      qrcode.generate(qr, { small: true }, (art) => {
        process.stderr.write(art + '\n');
      });
    }

    if (connection === 'open') {
      log('connected');
      if (authOnly) {
        log('--auth-only: credentials saved, exiting');
        process.exit(0);
      }
    }

    if (connection === 'close') {
      const code = lastDisconnect?.error?.output?.statusCode;
      const loggedOut = code === DisconnectReason.loggedOut;
      log(`disconnected (code=${code}) loggedOut=${loggedOut}`);
      if (loggedOut) {
        log('logged out — delete creds dir and re-run to re-authenticate');
        process.exit(1);
      }
      log('reconnecting in 5s...');
      setTimeout(connect, 5000);
    }
  });

  // ── messages ───────────────────────────────────────────────────────────────

  sock.ev.on('messages.upsert', ({ messages, type }) => {
    // 'notify' = new incoming, 'append' = history sync
    if (type !== 'notify' && type !== 'append') return;

    for (const raw of messages) {
      if (!raw.message) continue;
      if (raw.key.fromMe) continue;                // skip own messages

      const ts = Number(raw.messageTimestamp ?? 0);
      if (ts < sinceTs) continue;

      const text   = extractText(raw.message);
      const media  = mediaType(raw.message);
      const chatId = raw.key.remoteJid ?? '';
      const isGroup = chatId.endsWith('@g.us');

      // participant is set for group messages
      const from = isGroup
        ? (raw.key.participant ?? raw.participant ?? chatId)
        : chatId;

      emit({
        id:        raw.key.id,
        from,
        chat:      chatId,
        is_group:  isGroup,
        body:      text ?? `<media:${media ?? 'unknown'}>`,
        has_media: media !== null,
        media_type: media,
        timestamp: ts,
      });
    }
  });
}

connect().catch((err) => {
  log(`fatal: ${err}`);
  process.exit(1);
});
