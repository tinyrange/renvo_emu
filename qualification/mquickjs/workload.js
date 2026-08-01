"use strict";
var check_index = 0, failure = 0;
function check(value, name) { check_index++; if (!value && failure === 0) failure = check_index; }
function fold(values) {
  var total = 2166136261 | 0;
  for (var i = 0; i < values.length; i++) {
    total ^= values[i] | 0;
    total = Math.imul(total, 16777619);
  }
  return total | 0;
}
function closure(seed) { return function(v) { return seed + v * v; }; }
var square = closure(3);
var a = [];
for (var i = 0; i < 24; i++) a.push(square(i));
check(a[0] === 3 && a[7] === 52 && a[23] === 532, "closure-array");
var mapped = a.map(function(v, i) { return (v ^ (i * 17)) & 0xffff; });
var filtered = mapped.filter(function(v) { return (v & 3) !== 0; });
check(filtered.length === 24, "array-filter");
var object = {alpha: 7, beta: 11, nested: {value: 19}};
object.gamma = object.alpha * object.beta + object.nested.value;
check(JSON.parse(JSON.stringify(object)).gamma === 96, "json-object");
var text = "Renvo Emulator-1234-mquickjs";
check(/^Renvo Emulator-[0-9]+-mquickjs$/.test(text), "regexp");
check(text.replace(/[0-9]/g, "x") === "Renvo Emulator-xxxx-mquickjs", "replace");
var bytes = new Uint8Array(64);
var byte_values = [];
for (i = 0; i < bytes.length; i++) { bytes[i] = (i * 29 + 7) & 255; byte_values.push(bytes[i]); }
check(bytes[0] === 7 && bytes[63] === 42, "typed-array");
var caught = false;
try { throw new TypeError("remu"); } catch (e) { caught = e.name === "TypeError"; }
check(caught, "exceptions");
var recursive = function f(n) { return n < 2 ? n : f(n - 1) + f(n - 2); };
check(recursive(12) === 144, "recursion");
var digest = fold(mapped) ^ fold(filtered) ^ fold(byte_values);
check(digest === 960719237, "digest");
failure;
