const ORIGIN = "http://127.0.0.1:8732";
const HOST = new URLSearchParams(location.search).get("host") || "unknown";
const OPENED_AT = Date.now();
const line = (text) => {
  document.getElementById("log").textContent += text + "\n";
};

async function event(name, extra) {
  await fetch(ORIGIN + "/event", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(Object.assign({ name, from: HOST }, extra || {})),
  });
}

function heap() {
  if (typeof performance === "undefined" || !performance.memory) return null;
  return {
    usedJsHeapMiB: Math.round(performance.memory.usedJSHeapSize / 1048576),
    totalJsHeapMiB: Math.round(performance.memory.totalJSHeapSize / 1048576),
  };
}

const workerTimeline = [];
let polling = true;

async function pollWorkerPresence() {
  const available = typeof chrome.runtime.getContexts === "function";
  workerTimeline.push({ at: 0, getContextsAvailable: available });
  if (!available) return;
  while (polling) {
    let present = null;
    try {
      const contexts = await chrome.runtime.getContexts({ contextTypes: ["BACKGROUND"] });
      present = contexts.length;
    } catch (error) {
      present = String(error);
    }
    workerTimeline.push({ at: Date.now() - OPENED_AT, serviceWorkerContexts: present });
    await new Promise((resolve) => setTimeout(resolve, 2000));
  }
}

async function main() {
  const config = await fetch(ORIGIN + "/config").then((r) => r.json());
  await event("driver-alive", { host: HOST, config, heap: heap() });
  line("driver alive: " + HOST);

  const allocStart = Date.now();
  let payload = null;
  let allocError = null;
  try {
    payload = new Uint8Array(config.totalBytes);
    for (let i = 0; i < payload.length; i += 65536) payload[i] = (i / 65536) & 0xff;
    payload[0] = 0x41;
    payload[payload.length - 1] = 0x5a;
  } catch (error) {
    allocError = String(error);
  }
  const allocMs = Date.now() - allocStart;
  await event("payload-allocated", { bytes: config.totalBytes, allocMs, allocError, heap: heap() });
  if (allocError) {
    await fetch(ORIGIN + "/result", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ host: HOST, allocError, workerTimeline }),
    });
    return;
  }

  pollWorkerPresence();

  const partBytes = config.partBytes;
  const partCount = Math.ceil(config.totalBytes / partBytes);
  const uploadStart = Date.now();
  const partLog = [];
  let failure = null;
  for (let index = 0; index < partCount; index += 1) {
    const start = index * partBytes;
    const end = Math.min(start + partBytes, config.totalBytes);
    const attemptedAt = Date.now();
    try {
      const response = await fetch(ORIGIN + "/part/" + index, {
        method: "PUT",
        body: payload.subarray(start, end),
      });
      partLog.push({
        index,
        bytes: end - start,
        status: response.status,
        ms: Date.now() - attemptedAt,
        atMs: attemptedAt - uploadStart,
      });
    } catch (error) {
      failure = { index, error: String(error), atMs: attemptedAt - uploadStart };
      break;
    }
    if (index % 10 === 0) line("part " + index + " done");
  }
  const uploadMs = Date.now() - uploadStart;
  polling = false;

  const bytesSent = partLog.reduce((sum, p) => sum + p.bytes, 0);
  const report = {
    gate: "G3",
    host: HOST,
    variant: config.variant,
    label: config.label,
    userAgent: navigator.userAgent,
    totalBytes: config.totalBytes,
    partBytes,
    partCount,
    partsSent: partLog.length,
    bytesSent,
    allocMs,
    uploadMs,
    uploadThroughputMiBps: Math.round((bytesSent / 1048576 / (uploadMs / 1000)) * 100) / 100,
    slowestPartMs: partLog.reduce((m, p) => Math.max(m, p.ms), 0),
    partLog,
    failure,
    heapAfterUpload: heap(),
    driverSurvived: true,
    workerTimeline,
  };

  await event("upload-finished", { partsSent: report.partsSent, uploadMs, failure });
  try {
    await chrome.runtime.sendMessage({ done: true, host: HOST, partsSent: report.partsSent });
    report.wakeMessageSent = true;
  } catch (error) {
    report.wakeMessageSent = String(error);
  }
  await new Promise((resolve) => setTimeout(resolve, 2000));
  try {
    const contexts = await chrome.runtime.getContexts({ contextTypes: ["BACKGROUND"] });
    report.serviceWorkerContextsAfterWake = contexts.length;
  } catch (error) {
    report.serviceWorkerContextsAfterWake = String(error);
  }

  await fetch(ORIGIN + "/result", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(report),
  });
  line("done");
}

main();
