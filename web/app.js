const $ = (id) => document.getElementById(id);

const worker = new Worker(new URL("./worker.js", import.meta.url), { type: "module" });

let mode = "convert";
let file = null;
let busy = false;
let ready = false;
let downloadUrl = null;

function setStatus(text, kind = "") {
  const status = $("status");
  status.textContent = text;
  status.className = `status ${kind}`;
}

function updateButton() {
  const run = $("run");
  run.disabled = !ready || busy || !file;
  if (!ready) run.textContent = "Loading converter…";
  else if (busy) run.textContent = mode === "repair" ? "Repairing…" : "Converting…";
  else run.textContent = mode === "repair" ? "Repair pack" : "Convert pack";
}

function fillList(id, items) {
  const list = $(id);
  list.replaceChildren(...items.map((text) => Object.assign(document.createElement("li"), { textContent: text })));
  $(`${id}-count`).textContent = `(${items.length})`;
  $(`${id}-box`).hidden = items.length === 0;
}

function formatSize(bytes) {
  return bytes < 1024 * 1024 ? `${(bytes / 1024).toFixed(0)} KB` : `${(bytes / 1024 / 1024).toFixed(1)} MB`;
}

function outputName() {
  const base = file.name.replace(/\.(zip|rar)$/i, "");
  return mode === "repair" ? `${base}-repaired.zip` : `${base}-${$("to").value}.zip`;
}

function showResult(zip, report) {
  if (downloadUrl) URL.revokeObjectURL(downloadUrl);
  downloadUrl = URL.createObjectURL(new Blob([zip], { type: "application/zip" }));
  const download = $("download");
  download.href = downloadUrl;
  download.download = outputName();

  if (mode === "repair") {
    $("result-title").textContent = report.repaired.length ? `Repaired ${report.repaired.length} file(s)` : "Nothing needed repairing";
    $("result-meta").textContent = `${report.files} files · ${formatSize(zip.byteLength)}`;
    fillList("steps", report.repaired.map((f) => `${f.path}: ${f.issues.join(", ")}`));
  } else {
    $("result-title").textContent = report.from === report.to ? `Already compatible with ${report.to}` : `Converted ${report.from} → ${report.to}`;
    $("result-meta").textContent = `pack format ${report.target_format} · ${report.files_in} → ${report.files_out} files · ${formatSize(zip.byteLength)}`;
    fillList("steps", report.steps);
  }
  fillList("warnings", report.warnings);
  $("result").hidden = false;
}

worker.onmessage = ({ data }) => {
  if (data.type === "ready") {
    const versions = [...data.versions].reverse();
    for (const id of ["from", "to"]) {
      $(id).append(...versions.map((v) => new Option(v, v)));
    }
    ready = true;
    updateButton();
  } else if (data.type === "fatal") {
    setStatus(`Could not load the converter: ${data.message}`, "error");
  } else {
    busy = false;
    updateButton();
    if (data.type === "done") {
      setStatus("");
      showResult(data.zip, data.report);
    } else {
      setStatus(data.message, "error");
    }
  }
};

function pickFile(picked) {
  if (!picked) return;
  file = picked;
  $("drop").classList.add("has-file");
  $("drop-title").textContent = file.name;
  $("drop-hint").textContent = `${formatSize(file.size)} · click to choose another`;
  $("result").hidden = true;
  setStatus("");
  updateButton();
}

$("file").addEventListener("change", (e) => pickFile(e.target.files[0]));

const drop = $("drop");
drop.addEventListener("dragover", (e) => {
  e.preventDefault();
  drop.classList.add("over");
});
drop.addEventListener("dragleave", () => drop.classList.remove("over"));
drop.addEventListener("drop", (e) => {
  e.preventDefault();
  drop.classList.remove("over");
  pickFile(e.dataTransfer.files[0]);
});

for (const button of document.querySelectorAll(".mode-btn")) {
  button.addEventListener("click", () => {
    mode = button.dataset.mode;
    for (const other of document.querySelectorAll(".mode-btn")) {
      other.setAttribute("aria-selected", String(other === button));
    }
    $("convert-fields").hidden = mode === "repair";
    $("repair-note").hidden = mode !== "repair";
    $("result").hidden = true;
    updateButton();
  });
}

$("run").addEventListener("click", async () => {
  busy = true;
  updateButton();
  $("result").hidden = true;
  setStatus(mode === "repair" ? "Repairing… large packs can take a while." : "Converting… large packs can take a while.", "busy");
  const buffer = await file.arrayBuffer();
  worker.postMessage({ mode, to: $("to").value, from: $("from").value, buffer }, [buffer]);
});
