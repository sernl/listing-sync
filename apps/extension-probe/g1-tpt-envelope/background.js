importScripts("analysis.js");

const DEFAULT_ORIGIN = "https://www.teacherspayteachers.com";
const LOGIN_PATH = "/Login";
const CREATE_PATH = "/My-Products/New/Digital-Next";

// Values recorded verbatim; everything else is recorded by name only, so no
// credential leaves the browser in a report the founder sends back.
const REQUEST_VALUE_ALLOWLIST = [
  "origin",
  "referer",
  "user-agent",
  "accept-language",
  "accept",
  "accept-encoding",
  "sec-fetch-site",
  "sec-fetch-mode",
  "sec-fetch-dest",
  "sec-fetch-user",
  "sec-ch-ua",
  "sec-ch-ua-mobile",
  "sec-ch-ua-platform",
  "upgrade-insecure-requests",
  "x-requested-with",
  "content-type",
];
const RESPONSE_VALUE_ALLOWLIST = [
  "location",
  "content-type",
  "server",
  "status",
  "cache-control",
  "cross-origin-opener-policy",
];
const EXPECTED_REQUEST_HEADERS = [
  "origin",
  "referer",
  "user-agent",
  "accept-language",
  "sec-fetch-site",
  "sec-fetch-mode",
  "sec-fetch-dest",
  "sec-fetch-user",
];

let capture = [];
let capturing = false;

function redactRequestHeaders(headers) {
  const named = [];
  for (const header of headers || []) {
    const name = header.name.toLowerCase();
    if (name === "cookie") {
      const names = header.value
        .split(";")
        .map((c) => c.split("=")[0].trim().toLowerCase())
        .filter(Boolean);
      named.push({ name, valueRedacted: true, cookieNames: names });
    } else if (REQUEST_VALUE_ALLOWLIST.includes(name)) {
      named.push({ name, value: header.value });
    } else {
      named.push({ name, valueRedacted: true });
    }
  }
  return named;
}

function redactResponseHeaders(headers) {
  const named = [];
  for (const header of headers || []) {
    const name = header.name.toLowerCase();
    if (name === "set-cookie") {
      named.push({ name, valueRedacted: true, cookieName: header.value.split("=")[0].trim() });
    } else if (name.startsWith("cf-") || RESPONSE_VALUE_ALLOWLIST.includes(name)) {
      named.push({ name, value: header.value });
    } else {
      named.push({ name, valueRedacted: true });
    }
  }
  return named;
}

const FILTER = {
  urls: ["https://www.teacherspayteachers.com/*", "http://127.0.0.1/*"],
};

chrome.webRequest.onSendHeaders.addListener(
  (d) => {
    if (!capturing) return;
    capture.push({
      stage: "onSendHeaders",
      requestId: d.requestId,
      url: d.url,
      method: d.method,
      type: d.type,
      requestHeaders: redactRequestHeaders(d.requestHeaders),
    });
  },
  FILTER,
  ["requestHeaders", "extraHeaders"],
);

chrome.webRequest.onHeadersReceived.addListener(
  (d) => {
    if (!capturing) return;
    capture.push({
      stage: "onHeadersReceived",
      requestId: d.requestId,
      url: d.url,
      type: d.type,
      statusCode: d.statusCode,
      statusLine: d.statusLine,
      responseHeaders: redactResponseHeaders(d.responseHeaders),
    });
  },
  FILTER,
  ["responseHeaders", "extraHeaders"],
);

chrome.webRequest.onBeforeRedirect.addListener(
  (d) => {
    if (!capturing) return;
    capture.push({
      stage: "onBeforeRedirect",
      requestId: d.requestId,
      url: d.url,
      statusCode: d.statusCode,
      redirectUrl: d.redirectUrl,
    });
  },
  FILTER,
  ["responseHeaders", "extraHeaders"],
);

chrome.webRequest.onCompleted.addListener(
  (d) => {
    if (!capturing) return;
    capture.push({ stage: "onCompleted", requestId: d.requestId, url: d.url, statusCode: d.statusCode });
  },
  FILTER,
);

chrome.webRequest.onErrorOccurred.addListener(
  (d) => {
    if (!capturing) return;
    capture.push({ stage: "onErrorOccurred", requestId: d.requestId, url: d.url, error: d.error });
  },
  FILTER,
);

function envelopeFor(target, events) {
  const sent = events.find((e) => e.stage === "onSendHeaders" && e.url === target);
  const primary = sent || events.find((e) => e.stage === "onSendHeaders");
  const seen = {};
  const absent = [];
  if (primary) {
    for (const header of primary.requestHeaders) {
      if (header.value !== undefined) seen[header.name] = header.value;
    }
    for (const name of EXPECTED_REQUEST_HEADERS) {
      if (!(name in seen)) absent.push(name);
    }
  }
  const received = events.find((e) => e.stage === "onHeadersReceived" && e.url === target)
    || events.find((e) => e.stage === "onHeadersReceived");
  return {
    joinedRequestId: primary ? primary.requestId : null,
    requestType: primary ? primary.type : null,
    headersSent: seen,
    expectedHeadersAbsent: absent,
    allRequestHeaderNames: primary ? primary.requestHeaders.map((h) => h.name) : [],
    // Names only. Whether the session cookie rode along is the whole point of
    // the host-permission same-site rule, and the value must never be recorded.
    requestCookieNames: primary
      ? (primary.requestHeaders.find((h) => h.name === "cookie") || {}).cookieNames || []
      : [],
    responseStatus: received ? received.statusCode : null,
    responseStatusLine: received ? received.statusLine : null,
    cloudflareResponseHeaders: received
      ? received.responseHeaders.filter((h) => h.name.startsWith("cf-"))
      : [],
    setCookieNames: received
      ? received.responseHeaders.filter((h) => h.name === "set-cookie").map((h) => h.cookieName)
      : [],
    allResponseHeaderNames: received ? received.responseHeaders.map((h) => h.name) : [],
  };
}

async function withCapture(run) {
  capture = [];
  capturing = true;
  try {
    return await run();
  } finally {
    capturing = false;
  }
}

async function findTptTab(origin) {
  const tabs = await chrome.tabs.query({ url: origin + "/*" });
  return tabs.length > 0 ? tabs[0] : null;
}

async function waitForComplete(tabId, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const tab = await chrome.tabs.get(tabId);
    if (tab.status === "complete") return tab;
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  return chrome.tabs.get(tabId);
}

async function readPage(tabId) {
  await chrome.scripting.executeScript({ target: { tabId }, files: ["analysis.js"] });
  const [result] = await chrome.scripting.executeScript({
    target: { tabId },
    func: () => {
      let bodyText = "";
      try {
        bodyText = (document.body && document.body.innerText) || "";
      } catch (error) {
        bodyText = "";
      }
      return globalThis.tamAnalyse(
        document.documentElement.outerHTML,
        location.href,
        document.cookie,
        { passwordInputs: document.querySelectorAll('input[type="password" i]').length, bodyText },
      );
    },
  });
  return result.result;
}

async function fetchInTab(tabId, url) {
  await chrome.scripting.executeScript({ target: { tabId }, files: ["analysis.js"] });
  const [result] = await chrome.scripting.executeScript({
    target: { tabId },
    func: async (target) => {
      try {
        const response = await fetch(target, { credentials: "include" });
        const text = await response.text();
        const analysed = globalThis.tamAnalyse(text, response.url, document.cookie, null);
        analysed.fetchStatus = response.status;
        analysed.fetchRedirected = response.redirected;
        analysed.fetchFinalUrl = response.url;
        return analysed;
      } catch (error) {
        return { fetchError: String(error), url: target };
      }
    },
    args: [url],
  });
  return result.result;
}

async function routeA(origin, path) {
  const url = origin + path;
  return withCapture(async () => {
    let page;
    try {
      const response = await fetch(url, { credentials: "include" });
      const text = await response.text();
      page = globalThis.tamAnalyse(text, response.url, "", null);
      page.fetchStatus = response.status;
      page.fetchRedirected = response.redirected;
      page.fetchFinalUrl = response.url;
    } catch (error) {
      page = { fetchError: String(error), url };
    }
    await new Promise((resolve) => setTimeout(resolve, 500));
    return {
      route: "A",
      description: "fetch from the service worker with credentials: include",
      target: url,
      envelope: envelopeFor(url, capture),
      page,
      trail: capture.map((e) => ({ stage: e.stage, url: e.url, statusCode: e.statusCode, error: e.error })),
    };
  });
}

async function routeB(origin, path) {
  const url = origin + path;
  return withCapture(async () => {
    const tab = await findTptTab(origin);
    if (!tab) {
      return {
        route: "B",
        target: url,
        error: "no open tab on " + origin + " — open one, sign in, and run route B again",
      };
    }
    const page = await fetchInTab(tab.id, url);
    await new Promise((resolve) => setTimeout(resolve, 500));
    return {
      route: "B",
      description: "fetch from a content script in an already-open marketplace tab",
      target: url,
      hostTabUrl: tab.url,
      envelope: envelopeFor(url, capture),
      page,
      trail: capture.map((e) => ({ stage: e.stage, url: e.url, statusCode: e.statusCode, error: e.error })),
    };
  });
}

// The navigation is triggered from inside the marketplace document so the
// browser records that document as the initiator; a navigation the extension
// starts itself has no initiator and is sent Sec-Fetch-Site: none instead.
async function navigateFromPage(tabId, url) {
  try {
    await chrome.scripting.executeScript({
      target: { tabId },
      func: (target) => {
        location.href = target;
      },
      args: [url],
    });
  } catch (error) {
    // The frame navigates away before the injection resolves, which is the
    // intended effect rather than a failure.
  }
}

async function waitForUrl(tabId, url, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const tab = await chrome.tabs.get(tabId);
    if (tab.status === "complete" && tab.url && tab.url.startsWith(url)) return tab;
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  return chrome.tabs.get(tabId);
}

async function routeD(origin, path) {
  const url = origin + path;
  return withCapture(async () => {
    const tab = await findTptTab(origin);
    if (!tab) {
      return {
        route: "D",
        target: url,
        error: "no open tab on " + origin + " — open one, sign in, and run route D again",
      };
    }
    await navigateFromPage(tab.id, url);
    const settled = await waitForUrl(tab.id, url, 45000);
    await new Promise((resolve) => setTimeout(resolve, 2000));
    const page = await readPage(tab.id);
    return {
      route: "D",
      description: "a navigation started from inside the marketplace page, read by a content script",
      target: url,
      startedFromTabUrl: tab.url,
      finalTabUrl: settled.url,
      envelope: envelopeFor(url, capture),
      page,
      trail: capture.map((e) => ({ stage: e.stage, url: e.url, statusCode: e.statusCode, error: e.error })),
    };
  });
}

async function routeC(origin, path) {
  const url = origin + path;
  return withCapture(async () => {
    const tab = await chrome.tabs.create({ url, active: true });
    const settled = await waitForComplete(tab.id, 45000);
    await new Promise((resolve) => setTimeout(resolve, 2000));
    const page = await readPage(tab.id);
    return {
      route: "C",
      description: "a real top-level navigation, read by a content script in that tab",
      target: url,
      finalTabUrl: settled.url,
      envelope: envelopeFor(url, capture),
      page,
      trail: capture.map((e) => ({ stage: e.stage, url: e.url, statusCode: e.statusCode, error: e.error })),
    };
  });
}

async function store(key, report) {
  const wrapped = {
    gate: "G1",
    key,
    recordedAt: new Date().toISOString(),
    browser: navigator.userAgent,
    report,
  };
  await chrome.storage.local.set({ [key]: wrapped });
  return wrapped;
}

chrome.runtime.onMessage.addListener((message, sender, respond) => {
  (async () => {
    const origin = message.origin || DEFAULT_ORIGIN;
    if (message.command === "route-a-login") {
      respond(await store("route-a-login", await routeA(origin, message.loginPath || LOGIN_PATH)));
    } else if (message.command === "route-a-create") {
      respond(await store("route-a-create", await routeA(origin, message.createPath || CREATE_PATH)));
    } else if (message.command === "route-b-login") {
      respond(await store("route-b-login", await routeB(origin, message.loginPath || LOGIN_PATH)));
    } else if (message.command === "route-b-create") {
      respond(await store("route-b-create", await routeB(origin, message.createPath || CREATE_PATH)));
    } else if (message.command === "route-c-create") {
      respond(await store("route-c-create", await routeC(origin, message.createPath || CREATE_PATH)));
    } else if (message.command === "route-d-create") {
      respond(await store("route-d-create", await routeD(origin, message.createPath || CREATE_PATH)));
    } else if (message.command === "collect") {
      respond(await chrome.storage.local.get(null));
    } else {
      respond({ error: "unknown command " + message.command });
    }
  })();
  return true;
});
