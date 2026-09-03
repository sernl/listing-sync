// Chrome delivers an event to a terminated MV3 service worker only when the
// listener was registered synchronously during the worker's first evaluation.
// Every addListener below therefore runs at top level, before the module is
// instantiated; the handlers wait on `whenReady()` rather than assume it. This
// ordering is the defect that makes oxichrome's generated worker unusable,
// which registers inside an async function after an await.
import init, { health, version } from "./extension.js";

const ALARM_NAME = "tam-heartbeat";
const CADENCE_MINUTES = 60;

let ready = null;

function whenReady() {
  ready ??= init();
  return ready;
}

function recordFailure(failure) {
  chrome.storage.local.set({ lastFailure: String(failure) });
}

chrome.runtime.onInstalled.addListener(() => {
  chrome.alarms.create(ALARM_NAME, { periodInMinutes: CADENCE_MINUTES });
});

chrome.alarms.onAlarm.addListener((alarm) => {
  if (alarm.name !== ALARM_NAME) {
    return;
  }
  whenReady()
    .then(() => chrome.storage.local.set({ lastHeartbeat: health() }))
    .catch(recordFailure);
});

chrome.runtime.onMessage.addListener((message, _sender, respond) => {
  if (message?.kind !== "health") {
    return false;
  }
  whenReady()
    .then(() => respond({ ok: true, health: health(), version: version() }))
    .catch((failure) => respond({ ok: false, error: String(failure) }));
  // Keeping the channel open for an asynchronous reply is the other thing
  // oxichrome's generated handlers cannot express, because their closures
  // return unit.
  return true;
});

whenReady().catch(recordFailure);
