// ULIDs for notes and blocks made on this device: time-ordered, unique without asking the hub.

const ALPHABET = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

export function ulid(now = Date.now()): string {
  let time = "";
  let t = now;
  for (let i = 0; i < 10; i++) {
    time = ALPHABET[t % 32] + time;
    t = Math.floor(t / 32);
  }
  const random = new Uint8Array(16);
  crypto.getRandomValues(random);
  let rest = "";
  for (const b of random) rest += ALPHABET[b % 32];
  return time + rest;
}
