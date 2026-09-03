// Loaded both by the service worker (importScripts) and into a tab's isolated
// world (scripting.executeScript files:), so the three routes classify a page
// by identical code.
globalThis.tamAnalyse = function (html, url, cookieString, liveCounts) {
  const lowerHtml = (html || "").toLowerCase();
  const lowerUrl = (url || "").toLowerCase();
  const cookieNames = (cookieString || "")
    .split(";")
    .map((c) => c.split("=")[0].trim().toLowerCase())
    .filter((c) => c.length > 0);

  const markers = [];
  const scan = (table, haystack, where) => {
    for (const [label, needles] of table) {
      for (const needle of needles) {
        if (haystack.indexOf(needle) !== -1) {
          markers.push(label + " (" + where + ": " + needle + ")");
          break;
        }
      }
    }
  };

  scan(
    [
      [
        "cloudflare-interstitial",
        [
          "just a moment",
          "cf-chl",
          "__cf_chl",
          "cf_chl_opt",
          "cdn-cgi/challenge-platform",
          "cf-browser-verification",
          "checking your browser",
        ],
      ],
      ["cloudflare-turnstile", ["challenges.cloudflare.com", "cf-turnstile"]],
      ["hcaptcha", ["hcaptcha.com", "h-captcha"]],
      ["recaptcha", ["recaptcha", "g-recaptcha"]],
      ["datadome", ["captcha-delivery.com", "datadome"]],
      ["perimeterx", ["perimeterx", "px-captcha", "client.px-cloud.net"]],
      ["akamai-bot-manager", ["_abck", "bm_sz", "ak_bmsc"]],
      ["kasada", ["kasada", "kpsdk"]],
    ],
    lowerHtml,
    "html",
  );
  scan(
    [
      ["cloudflare-interstitial", ["cdn-cgi/challenge-platform", "__cf_chl"]],
      ["datadome", ["captcha-delivery.com"]],
      ["perimeterx", ["px-captcha"]],
    ],
    lowerUrl,
    "url",
  );

  const cookieTable = [
    ["cloudflare-interstitial", ["cf_clearance", "__cf_chl"]],
    ["datadome", ["datadome"]],
    ["perimeterx", ["_px"]],
    ["akamai-bot-manager", ["_abck", "bm_sz", "ak_bmsc"]],
    ["kasada", ["kpsdk"]],
  ];
  for (const [label, prefixes] of cookieTable) {
    const hit = prefixes.find((prefix) => cookieNames.some((name) => name.indexOf(prefix) === 0));
    if (hit) markers.push(label + " (cookie: " + hit + ")");
  }

  // Cloudflare's edge refusal, which classify.rs treats as a different
  // condition from the interstitial.
  const edgeBlock =
    lowerHtml.indexOf("/cdn-cgi/error") !== -1 ||
    lowerHtml.indexOf("sorry, you have been blocked") !== -1 ||
    lowerHtml.indexOf("attention required! | cloudflare") !== -1 ||
    lowerHtml.indexOf("error 1020") !== -1;

  // The five hidden inputs and the credentials block that form.rs scrapes; the
  // create form is "served" exactly when a write could scrape it.
  const anchors = {
    "data[_Token][key]": 'name="data[_Token][key]"',
    "data[_Token][fields]": 'name="data[_Token][fields]"',
    "data[_Token][unlocked]": 'name="data[_Token][unlocked]"',
    "data[_Csrf][csrfKey]": 'name="data[_Csrf][csrfKey]"',
    "data[_Csrf][csrfToken]": 'name="data[_Csrf][csrfToken]"',
    "aws credentials block": '"credentials":{"key":"',
  };
  const formAnchors = {};
  for (const [label, needle] of Object.entries(anchors)) {
    formAnchors[label] = (html || "").indexOf(needle) !== -1;
  }
  const formServed = Object.entries(formAnchors)
    .filter(([label]) => label.startsWith("data["))
    .every(([, present]) => present);

  const passwordInputs =
    liveCounts && typeof liveCounts.passwordInputs === "number"
      ? liveCounts.passwordInputs
      : (lowerHtml.match(/type=["']?password/g) || []).length;

  const titleMatch = (html || "").match(/<title[^>]*>([\s\S]{0,200}?)<\/title>/i);
  const title = titleMatch ? titleMatch[1].trim() : null;

  let verdict = "UNKNOWN";
  if (markers.length > 0) verdict = "CHALLENGE";
  else if (passwordInputs > 0 || formServed) verdict = "CLEAR";

  // A wall is a challenge with nothing usable behind it; a widget is a
  // challenge alongside a form the human can still satisfy.
  const wall = markers.length > 0 && passwordInputs === 0 && !formServed;

  return {
    url,
    title,
    verdict,
    wall,
    edgeBlock,
    markers,
    cookieNamesSeen: cookieNames,
    csrfTokenReadableFromDocumentCookie: cookieNames.includes("csrftoken"),
    passwordInputs,
    formServed,
    formAnchors,
    htmlBytes: (html || "").length,
    // Body text is captured only when a challenge fired, because that page is
    // Cloudflare's rather than the seller's. See the README.
    challengeBodyText:
      markers.length > 0 && liveCounts && liveCounts.bodyText
        ? String(liveCounts.bodyText).slice(0, 300)
        : null,
  };
};
