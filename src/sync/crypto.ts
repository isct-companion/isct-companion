import type { PayloadCipher } from "../connectors/cloud";

const encoder = new TextEncoder();
const HEADER = encoder.encode("ISCT1");

export type RecoveryKey = { salt: string; keyId: string };

const base64 = (bytes: Uint8Array) => btoa(String.fromCharCode(...bytes));
const unbase64 = (value: string) =>
  Uint8Array.from(atob(value), (character) => character.charCodeAt(0));
const buffer = (value: Uint8Array): ArrayBuffer =>
  Uint8Array.from(value).buffer;

/** Creates AES-GCM backup encryption from a user-held recovery phrase. */
export async function createRecoveryCipher(
  phrase: string,
  recovery?: RecoveryKey,
): Promise<{ cipher: PayloadCipher; recovery: RecoveryKey }> {
  if (phrase.trim().length < 12)
    throw new Error("Use a recovery phrase of at least 12 characters.");
  const salt = recovery
    ? unbase64(recovery.salt)
    : crypto.getRandomValues(new Uint8Array(16));
  const source = await crypto.subtle.importKey(
    "raw",
    encoder.encode(phrase),
    "PBKDF2",
    false,
    ["deriveKey"],
  );
  const key = await crypto.subtle.deriveKey(
    {
      name: "PBKDF2",
      hash: "SHA-256",
      salt: buffer(salt),
      iterations: 310_000,
    },
    source,
    { name: "AES-GCM", length: 256 },
    false,
    ["encrypt", "decrypt"],
  );
  const digest = new Uint8Array(
    await crypto.subtle.digest(
      "SHA-256",
      encoder.encode(`${base64(salt)}:${phrase}`),
    ),
  );
  const keyId = base64(digest.slice(0, 12));
  if (recovery && recovery.keyId !== keyId)
    throw new Error("The recovery phrase does not match this backup key.");
  const cipher: PayloadCipher = {
    keyId,
    async encrypt(cleartext) {
      const iv = crypto.getRandomValues(new Uint8Array(12));
      const body = new Uint8Array(
        await crypto.subtle.encrypt(
          { name: "AES-GCM", iv: buffer(iv) },
          key,
          buffer(cleartext),
        ),
      );
      const output = new Uint8Array(HEADER.length + iv.length + body.length);
      output.set(HEADER);
      output.set(iv, HEADER.length);
      output.set(body, HEADER.length + iv.length);
      return output;
    },
    async decrypt(ciphertext) {
      if (!HEADER.every((byte, index) => ciphertext[index] === byte))
        throw new Error(
          "This backup does not use the supported encrypted format.",
        );
      const iv = ciphertext.slice(HEADER.length, HEADER.length + 12);
      const body = ciphertext.slice(HEADER.length + 12);
      try {
        return new Uint8Array(
          await crypto.subtle.decrypt(
            { name: "AES-GCM", iv: buffer(iv) },
            key,
            buffer(body),
          ),
        );
      } catch {
        throw new Error("Backup decryption failed. Check the recovery phrase.");
      }
    },
  };
  return { cipher, recovery: { salt: base64(salt), keyId } };
}
