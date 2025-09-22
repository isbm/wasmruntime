function readAll() {
  const buf = [];
  const chunk = new Uint8Array(1024);
  while (true) {
    const n = Javy.IO.readSync(0, chunk);
    if (n === 0) break;
    buf.push(chunk.slice(0, n));
  }
  let len = 0; for (const b of buf) len += b.length;
  const out = new Uint8Array(len);
  let o=0; for (const b of buf) { out.set(b, o); o+=b.length; }
  return new TextDecoder().decode(out);
}

const input = readAll();
// const msg = { ok: true, msg: "hello world from JavaScript", input: input };
const msg = { ok: true, msg: "hello world from JavaScript"};
const bytes = new TextEncoder().encode(JSON.stringify(msg));
Javy.IO.writeSync(1, bytes);
