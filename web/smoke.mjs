import { api } from "./generated-node/renvo.js";

const targets = JSON.parse(api.listTargets());

if (!Array.isArray(targets) || targets.length === 0) {
  throw new Error("WASI component returned an empty target manifest");
}

for (const expected of ["esp32s3", "esp32c6"]) {
  if (!targets.some(({ id }) => id === expected)) {
    throw new Error(`WASI component target manifest is missing ${expected}`);
  }
}

let rejectedInvalidElf = false;
try {
  api.inspectElf(new Uint8Array());
} catch (error) {
  rejectedInvalidElf = String(error).includes("ELF");
}

if (!rejectedInvalidElf) {
  throw new Error("WASI component did not reject an invalid ELF image");
}

console.log(`WASI JavaScript smoke test passed for ${targets.length} targets`);
