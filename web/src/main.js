const worker = new Worker(new URL("./worker.js", import.meta.url), { type: "module" });
const pending = new Map();
let nextId = 1;

worker.onmessage = ({ data }) => {
  const request = pending.get(data.id);
  if (!request) return;
  pending.delete(data.id);
  if (data.error) request.reject(new Error(data.error));
  else request.resolve(data.result);
};

function request(operation, payload = {}, transfer = []) {
  const id = nextId++;
  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
    worker.postMessage({ id, operation, ...payload }, transfer);
  });
}

const form = document.querySelector("#runner");
const target = document.querySelector("#target");
const status = document.querySelector("#status");
const output = document.querySelector("#result");

try {
  const targets = await request("targets");
  for (const manifest of targets) {
    const option = document.createElement("option");
    option.value = manifest.id;
    option.textContent = `${manifest.id} — ${manifest.name}`;
    target.append(option);
  }
  status.textContent = `${targets.length} targets ready`;
} catch (error) {
  status.textContent = "Component failed to load";
  output.textContent = error.stack ?? String(error);
}

form.addEventListener("submit", async (event) => {
  event.preventDefault();
  const file = document.querySelector("#firmware").files[0];
  if (!file) return;
  const firmware = await file.arrayBuffer();
  status.textContent = "Running…";
  output.textContent = "";
  try {
    const result = await request(
      "run",
      {
        target: target.value,
        format: document.querySelector("#format").value,
        firmware,
        options: {
          maxInstructions: document.querySelector("#instructions").value,
          stimuli: [],
        },
      },
      [firmware],
    );
    status.textContent = "Complete";
    output.textContent = JSON.stringify(result, null, 2);
  } catch (error) {
    status.textContent = "Run failed";
    output.textContent = error.stack ?? String(error);
  }
});
