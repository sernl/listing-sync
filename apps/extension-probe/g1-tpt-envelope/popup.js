const out = document.getElementById("out");
const status = document.getElementById("status");

function target() {
  return {
    origin: document.getElementById("origin").value.trim().replace(/\/$/, ""),
    loginPath: document.getElementById("loginPath").value.trim(),
    createPath: document.getElementById("createPath").value.trim(),
  };
}

function show(value) {
  out.value = JSON.stringify(value, null, 2);
}

function summarise(wrapped) {
  const report = wrapped.report || {};
  if (report.error) return report.route + ": " + report.error;
  const page = report.page || {};
  const envelope = report.envelope || {};
  return [
    "route " + report.route,
    "status " + envelope.responseStatus,
    page.verdict || "?",
    page.formServed ? "form served" : "no form",
    page.wall ? "WALL" : "",
  ]
    .filter(Boolean)
    .join(" | ");
}

for (const button of document.querySelectorAll("button[data-command]")) {
  button.addEventListener("click", async () => {
    status.textContent = "running " + button.dataset.command + "…";
    out.value = "";
    try {
      const wrapped = await chrome.runtime.sendMessage(
        Object.assign({ command: button.dataset.command }, target()),
      );
      status.textContent = summarise(wrapped);
      show(wrapped);
    } catch (error) {
      status.textContent = String(error);
    }
  });
}

document.getElementById("collect").addEventListener("click", async () => {
  const everything = await chrome.runtime.sendMessage({ command: "collect" });
  status.textContent = Object.keys(everything).length + " route report(s) recorded";
  show(everything);
});

document.getElementById("copy").addEventListener("click", async () => {
  out.select();
  try {
    await navigator.clipboard.writeText(out.value);
    status.textContent = "copied";
  } catch (error) {
    status.textContent = "clipboard refused; the text is selected, press Ctrl+C";
  }
});

document.getElementById("save").addEventListener("click", () => {
  const blob = new Blob([out.value], { type: "application/json" });
  const anchor = document.createElement("a");
  anchor.href = URL.createObjectURL(blob);
  anchor.download = "g1-tpt-envelope.json";
  anchor.click();
  status.textContent = "saved as g1-tpt-envelope.json";
});

// The local verification harness drives the popup by opening it with
// ?autorun=<comma-separated commands>&report=<url>. The report URL is refused
// unless it addresses 127.0.0.1, so this affordance cannot send a report off
// the machine even if a crafted link reached the founder.
async function autorun() {
  const params = new URLSearchParams(location.search);
  const commands = (params.get("autorun") || "").split(",").filter(Boolean);
  if (commands.length === 0) return;
  const origin = params.get("origin");
  if (origin) document.getElementById("origin").value = origin;
  if (params.get("open") === "1") {
    const tab = await chrome.tabs.create({ url: target().origin + "/", active: false });
    for (let waited = 0; waited < 20000; waited += 500) {
      const current = await chrome.tabs.get(tab.id);
      if (current.status === "complete") break;
      await new Promise((resolve) => setTimeout(resolve, 500));
    }
  }
  const collected = {};
  for (const command of commands) {
    collected[command] = await chrome.runtime.sendMessage(
      Object.assign({ command }, target()),
    );
  }
  show(collected);
  const report = params.get("report");
  if (report && /^http:\/\/127\.0\.0\.1(:\d+)?\//.test(report)) {
    await fetch(report, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(collected, null, 2),
    });
    status.textContent = "autorun complete, report posted";
  }
}

autorun();
