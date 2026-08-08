import { api } from "../generated/renvo.js";

function bytes(value) {
  if (value instanceof Uint8Array) return value;
  if (value instanceof ArrayBuffer) return new Uint8Array(value);
  if (ArrayBuffer.isView(value)) {
    return new Uint8Array(value.buffer, value.byteOffset, value.byteLength);
  }
  throw new TypeError("firmware must be a Uint8Array, ArrayBuffer, or typed-array view");
}

function integer(value, name) {
  const integerValue = BigInt(value);
  if (integerValue < 0n) throw new RangeError(`${name} must not be negative`);
  return integerValue;
}

function options(value = {}) {
  return {
    maxInstructions: integer(value.maxInstructions ?? 1_000_000, "maxInstructions"),
    deadlineTicks:
      value.deadlineTicks === undefined || value.deadlineTicks === null
        ? undefined
        : integer(value.deadlineTicks, "deadlineTicks"),
    stimuli: (value.stimuli ?? []).map((stimulus) => ({
      at: integer(stimulus.at, "stimulus.at"),
      pin: stimulus.pin,
      value: stimulus.value,
    })),
  };
}

function call(operation) {
  try {
    return operation();
  } catch (error) {
    throw error instanceof Error ? error : new Error(String(error));
  }
}

/** Stable JavaScript facade over the versioned Renvo WIT interface. */
export class Renvo {
  static listTargetsJson() {
    return call(() => api.listTargets());
  }

  static listTargets() {
    return JSON.parse(this.listTargetsJson());
  }

  static inspectElfJson(firmware) {
    return call(() => api.inspectElf(bytes(firmware)));
  }

  static inspectElf(firmware) {
    return JSON.parse(this.inspectElfJson(firmware));
  }

  static runElfJson(target, firmware, runOptions) {
    return call(() => api.runElf(target, bytes(firmware), options(runOptions)));
  }

  static runElf(target, firmware, runOptions) {
    return JSON.parse(this.runElfJson(target, firmware, runOptions));
  }

  static runIntelHexJson(target, firmware, runOptions) {
    return call(() => api.runIntelHex(target, bytes(firmware), options(runOptions)));
  }

  static runIntelHex(target, firmware, runOptions) {
    return JSON.parse(this.runIntelHexJson(target, firmware, runOptions));
  }
}
