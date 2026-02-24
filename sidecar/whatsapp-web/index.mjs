/**
 * OpenCrust WhatsApp Web Sidecar
 *
 * Bridges WhatsApp Web (via Baileys) to the Rust gateway over stdin/stdout
 * using line-delimited JSON.
 *
 * Stdout events:
 *   {"type":"qr","data":"<qr-string>"}
 *   {"type":"ready"}
 *   {"type":"message","id":"<msg-id>","from":"<jid>","name":"<push-name>","text":"<body>"}
 *   {"type":"disconnected","reason":"..."}
 *
 * Stdin commands:
 *   {"type":"send","to":"<jid>","text":"<body>"}
 *   {"type":"ping"} -> responds {"type":"pong"}
 */

import { makeWASocket, useMultiFileAuthState, DisconnectReason, fetchLatestBaileysVersion } from "@whiskeysockets/baileys";
import qrcode from "qrcode-terminal";
import pino from "pino";
import { createInterface } from "node:readline";
import { join } from "node:path";
import { homedir } from "node:os";

const AUTH_DIR = process.env.WHATSAPP_AUTH_DIR || join(homedir(), ".opencrust", "whatsapp-web-auth");

const logger = pino({ level: "silent" });

/** Send a JSON event to stdout (one line). */
function emit(obj) {
  process.stdout.write(JSON.stringify(obj) + "\n");
}

/** Start the Baileys socket and wire up events. */
async function start() {
  const { state, saveCreds } = await useMultiFileAuthState(AUTH_DIR);
  const { version } = await fetchLatestBaileysVersion();

  const sock = makeWASocket({
    version,
    auth: state,
    logger,
    printQRInTerminal: false,
    markOnlineOnConnect: true,
  });

  sock.ev.on("creds.update", saveCreds);

  sock.ev.on("connection.update", (update) => {
    const { connection, lastDisconnect, qr } = update;

    if (qr) {
      qrcode.generate(qr, { small: true }, (art) => {
        // Print QR art to stderr so the terminal shows it but stdout stays clean JSON
        process.stderr.write("\n" + art + "\n");
        process.stderr.write("Scan this QR code with WhatsApp on your phone\n\n");
      });
      emit({ type: "qr", data: qr });
    }

    if (connection === "open") {
      emit({ type: "ready" });
    }

    if (connection === "close") {
      const statusCode = lastDisconnect?.error?.output?.statusCode;
      const reason = DisconnectReason[statusCode] || String(statusCode || "unknown");

      if (statusCode === DisconnectReason.loggedOut) {
        emit({ type: "disconnected", reason: "logged_out" });
        process.exit(0);
      }

      emit({ type: "disconnected", reason });
      // Baileys handles reconnection internally for most disconnect reasons.
      // Re-create the socket for a clean restart.
      setTimeout(() => start(), 3000);
    }
  });

  sock.ev.on("messages.upsert", ({ messages, type: upsertType }) => {
    if (upsertType !== "notify") return;

    for (const msg of messages) {
      // Ignore own messages, status broadcasts, and group messages
      if (msg.key.fromMe) continue;
      if (msg.key.remoteJid === "status@broadcast") continue;
      if (msg.key.remoteJid?.endsWith("@g.us")) continue;

      const text =
        msg.message?.conversation ||
        msg.message?.extendedTextMessage?.text ||
        "";

      if (!text) continue;

      emit({
        type: "message",
        id: msg.key.id || "",
        from: msg.key.remoteJid || "",
        name: msg.pushName || "",
        text,
      });
    }
  });

  // Read commands from stdin
  const rl = createInterface({ input: process.stdin });

  rl.on("line", async (line) => {
    let cmd;
    try {
      cmd = JSON.parse(line);
    } catch {
      return;
    }

    if (cmd.type === "ping") {
      emit({ type: "pong" });
      return;
    }

    if (cmd.type === "send" && cmd.to && cmd.text) {
      try {
        await sock.sendMessage(cmd.to, { text: cmd.text });
      } catch (err) {
        // Log send errors to stderr, don't crash
        process.stderr.write(`send error: ${err.message}\n`);
      }
    }
  });

  rl.on("close", () => {
    // stdin closed - parent process is shutting down
    process.exit(0);
  });

  // Graceful shutdown on SIGTERM
  process.on("SIGTERM", () => {
    sock.end(undefined);
    process.exit(0);
  });
}

start().catch((err) => {
  process.stderr.write(`fatal: ${err.message}\n`);
  process.exit(1);
});
