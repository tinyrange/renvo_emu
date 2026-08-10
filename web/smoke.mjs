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

let rejectedSubstitutedRadioRom = false;
try {
  api.runRadioElf("esp32c6", new Uint8Array(), new Uint8Array(), {
    run: {
      maxInstructions: 1n,
      deadlineTicks: undefined,
      stimuli: [],
    },
    radioFrames: [],
  });
} catch (error) {
  rejectedSubstitutedRadioRom = String(error).includes(
    "requires the pinned real mask-ROM ELF",
  );
}

if (!rejectedSubstitutedRadioRom) {
  throw new Error("WASI radio API did not reject a substituted mask ROM");
}

console.log(`WASI JavaScript smoke test passed for ${targets.length} targets`);
