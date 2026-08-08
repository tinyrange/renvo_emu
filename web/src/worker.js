import { Renvo } from "./remu.js";

self.onmessage = ({ data }) => {
  try {
    if (data.operation === "targets") {
      self.postMessage({ id: data.id, result: Renvo.listTargets() });
      return;
    }
    if (data.operation === "run") {
      const firmware = new Uint8Array(data.firmware);
      const result =
        data.format === "ihex"
          ? Renvo.runIntelHex(data.target, firmware, data.options)
          : Renvo.runElf(data.target, firmware, data.options);
      self.postMessage({ id: data.id, result });
      return;
    }
    throw new Error(`unknown worker operation ${data.operation}`);
  } catch (error) {
    self.postMessage({
      id: data.id,
      error: error instanceof Error ? error.message : String(error),
    });
  }
};
