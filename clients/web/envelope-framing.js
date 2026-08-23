/** Adds a protobuf varint length before an envelope. */
export function lengthDelimit(envelopeBytes) {
  const prefix = [];
  let value = envelopeBytes.length;
  for (;;) {
    let byte = value & 0x7f;
    value >>>= 7;
    if (value !== 0) {
      prefix.push(byte | 0x80);
    } else {
      prefix.push(byte);
      break;
    }
  }
  const framed = new Uint8Array(prefix.length + envelopeBytes.length);
  framed.set(prefix, 0);
  framed.set(envelopeBytes, prefix.length);
  return framed;
}
