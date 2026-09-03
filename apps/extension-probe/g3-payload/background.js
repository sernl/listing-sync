const ORIGIN = "http://127.0.0.1:8732";
const SPAWN_AT = Date.now();

async function beacon(name, extra) {
  await fetch(ORIGIN + "/event", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(Object.assign({ name, from: "worker", spawnAt: SPAWN_AT }, extra || {})),
  });
}

chrome.runtime.onStartup.addListener(() => {});
chrome.runtime.onInstalled.addListener(() => {});

chrome.runtime.onMessage.addListener((message) => {
  beacon("worker-woken-by-message", { message });
  return false;
});

async function openOffscreen() {
  const reasons = ["BLOBS", "WORKERS"];
  for (const reason of reasons) {
    try {
      await chrome.offscreen.createDocument({
        url: "driver.html?host=offscreen",
        reasons: [reason],
        justification: "Drives a long multipart upload independent of the service worker.",
      });
      return { host: "offscreen", reasonAccepted: reason };
    } catch (error) {
      await beacon("offscreen-reason-rejected", { reason, error: String(error) });
    }
  }
  return { host: "offscreen", reasonAccepted: null };
}

async function openRunnerTab() {
  const tab = await chrome.tabs.create({
    url: chrome.runtime.getURL("driver.html?host=tab"),
    active: false,
  });
  let autoDiscardableSet = null;
  try {
    const updated = await chrome.tabs.update(tab.id, { autoDiscardable: false });
    autoDiscardableSet = updated.autoDiscardable;
  } catch (error) {
    autoDiscardableSet = String(error);
  }
  return { host: "tab", tabId: tab.id, autoDiscardable: autoDiscardableSet };
}

async function main() {
  const reply = await fetch(ORIGIN + "/worker-start", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ spawnAt: SPAWN_AT }),
  }).then((r) => r.json());
  if (!reply.start) return;
  const opened =
    reply.config.variant === "tab" ? await openRunnerTab() : await openOffscreen();
  await beacon("driver-opened", opened);
}

main();
