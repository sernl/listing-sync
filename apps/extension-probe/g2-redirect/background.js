const ORIGIN = "http://127.0.0.1:8731";
const SOURCE = ORIGIN + "/redirect-source";
const FILTER = { urls: [ORIGIN + "/*"] };

const events = [];
const startedAt = Date.now();

function note(stage, details) {
  events.push({
    stage,
    at: Date.now() - startedAt,
    requestId: details.requestId,
    url: details.url,
    method: details.method,
    type: details.type,
    statusCode: details.statusCode,
    statusLine: details.statusLine,
    redirectUrl: details.redirectUrl,
    error: details.error,
    responseHeaders: (details.responseHeaders || []).map((h) => [h.name, h.value]),
  });
}

chrome.webRequest.onBeforeRequest.addListener((d) => note("onBeforeRequest", d), FILTER);
chrome.webRequest.onSendHeaders.addListener((d) => note("onSendHeaders", d), FILTER);
chrome.webRequest.onHeadersReceived.addListener(
  (d) => note("onHeadersReceived", d),
  FILTER,
  ["responseHeaders"],
);
chrome.webRequest.onBeforeRedirect.addListener((d) => note("onBeforeRedirect", d), FILTER, [
  "responseHeaders",
]);
chrome.webRequest.onCompleted.addListener((d) => note("onCompleted", d), FILTER, [
  "responseHeaders",
]);
chrome.webRequest.onErrorOccurred.addListener((d) => note("onErrorOccurred", d), FILTER);

function locationFrom(entry) {
  for (const [name, value] of entry.responseHeaders) {
    if (name.toLowerCase() === "location") return value;
  }
  return null;
}

function summarise(mode, url, response, sliceOfEvents) {
  const received = sliceOfEvents.filter((e) => e.stage === "onHeadersReceived");
  const first = received.find((e) => e.statusCode === 302) || received[0] || null;
  const joinedById = first
    ? sliceOfEvents.filter((e) => e.requestId === first.requestId).map((e) => e.stage)
    : [];
  const joinedByUrl = sliceOfEvents.filter((e) => e.url === url).map((e) => e.stage);
  return {
    mode,
    requestUrl: url,
    fetchResolved: response.ok !== undefined,
    responseType: response.type,
    responseStatus: response.status,
    responseUrl: response.url,
    responseHeadersVisibleToFetch: [...response.headers.keys()],
    locationFromListener: first ? locationFrom(first) : null,
    joinRequestId: first ? first.requestId : null,
    joinByRequestIdStages: joinedById,
    joinByUrlStages: joinedByUrl,
    onBeforeRedirectFired: sliceOfEvents.some((e) => e.stage === "onBeforeRedirect"),
    stages: sliceOfEvents.map((e) => ({
      stage: e.stage,
      requestId: e.requestId,
      url: e.url,
      statusCode: e.statusCode,
      redirectUrl: e.redirectUrl,
      error: e.error,
    })),
    rawHeadersReceived: received,
  };
}

async function runOne(mode) {
  const mark = events.length;
  const url = SOURCE + "?mode=" + mode;
  let response;
  let failure = null;
  try {
    response = await fetch(url, { redirect: mode, credentials: "include" });
  } catch (error) {
    failure = String(error);
    response = { type: null, status: null, url: null, headers: new Headers() };
  }
  await new Promise((resolve) => setTimeout(resolve, 400));
  const summary = summarise(mode, url, response, events.slice(mark));
  summary.fetchThrew = failure;
  return summary;
}

async function main() {
  const report = {
    gate: "G2",
    manifestVersion: chrome.runtime.getManifest().manifest_version,
    declaredPermissions: chrome.runtime.getManifest().permissions,
    userAgent: navigator.userAgent,
    webRequestBlockingDeclared: (chrome.runtime.getManifest().permissions || []).includes(
      "webRequestBlocking",
    ),
    runs: [],
  };
  report.runs.push(await runOne("manual"));
  report.runs.push(await runOne("follow"));
  await fetch(ORIGIN + "/result", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(report, null, 2),
  });
}

main();
