// jsdom does not provide TextEncoder/TextDecoder, which the streaming API client
// (response.body reader + TextDecoder) relies on. Polyfill them from Node.
const { TextEncoder, TextDecoder } = require("util");

if (typeof global.TextEncoder === "undefined") {
  global.TextEncoder = TextEncoder;
}
if (typeof global.TextDecoder === "undefined") {
  global.TextDecoder = TextDecoder;
}
