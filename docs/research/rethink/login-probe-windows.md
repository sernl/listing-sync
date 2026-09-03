# The login probe on Windows, 2026-09-03

D12 gates the desktop client on one question: can the webview stack Tauri v2 ships reach the TeachersPayTeachers and Tes login pages without meeting a bot challenge?
The founder ran the staged Windows build on their own Windows machine on 2026-09-03, once per marketplace, and both halves pass.
Tes is CLEAR outright.
TPT's report records CHALLENGE, but that verdict is a false positive: the one marker that fired matched a feature-flag name rather than any bot challenge, and the page carries a working login form.
The marker was narrowed the same day and the corrected reading of both runs is CLEAR.

- date: 2026-09-03
- method: two read-only runs of `tools/login-probe/dist/login-probe.exe` from PowerShell on the founder's own Windows machine, one per marketplace; the probe navigates, waits and reads, never fills or submits a form, and carries no credential
- evidence: the two JSON reports copied verbatim below, each scanned before copying and carrying no cookie, token, address or other secret; the page HTML each run wrote is not copied here, and is read below only to establish what the markers matched
- the verdict rules, and the webview stack: `tools/login-probe/README.md`

## The two reports

TeachersPayTeachers:

```json
{
  "schema": "login-probe/1",
  "timestamp_utc": "2026-09-03T03:45:37Z",
  "os": "windows",
  "arch": "x86_64",
  "webview_stack": "WebView2 (wry 0.55.1, tao 0.35.3)",
  "requested_url": "\nhttps://www.teacherspayteachers.com/Login",
  "wait_secs": 15,
  "verdict": "CHALLENGE",
  "outcome": "reported",
  "final_url": "https://www.teacherspayteachers.com/Login",
  "title": "Log In | Teachers Pay Teachers",
  "user_agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/152.0.0.0 Safari/537.36 Edg/152.0.0.0",
  "challenge_markers": ["recaptcha (html: recaptcha)"],
  "password_form_count": 1,
  "password_input_count": 1,
  "body_text_head": "Log in\nDon't have an account? Sign up\nEmail or username\nPassword\nRemember me\nLog in\nForgot username?\nForgot password?\nTPT is the largest marketplace for PreK-12 resources, powered by a community of educators.\nFacebook\nInstagram\nPinterest\nTwitter\nABOUT\nWho we are\nWe're hiring\nPress\nBlog\nGift Cards\nSUPPORT\nHelp & FAQ\nSecurity\nPrivacy policy\nStudent privacy\nTerms of service\nTell us what you think\nUPDATES\nGet our weekly newsletter with free resources, updates, and special offers.\nGet newsletter\nIXL FAMILY OF BRANDS\nIXL\nComprehensive K-12 personalized learning\nRosetta Stone\nImmersive learning for 25 languages\nWyzant\nTrusted tutors for 300 subjects\nEducation.com\n35,000 worksheets, games, and lesson plans\nVocabulary.com\nAdaptive learning for English vocabulary\nEmmersion\nFast and accurate language certification\nThesaurus.com\nEssential reference for synonyms and antonyms\nDictionary.com\nComprehensive resource for word definitions and usage\nSpanishDictionary.com\nSpanish-English dictionary, translator, and learning\nFrenchDictionary.com\nFrench-English dictionary, translator, and learning\nIngles.com\nDiccionario inglés-español, traductor y sitio de aprendizaje\nABCya\nFun educational games for kids\n© 2026 by IXL Learning",
  "page_html_bytes": 42329,
  "page_html_file": "report.html"
}
```

Tes:

```json
{
  "schema": "login-probe/1",
  "timestamp_utc": "2026-09-03T03:47:11Z",
  "os": "windows",
  "arch": "x86_64",
  "webview_stack": "WebView2 (wry 0.55.1, tao 0.35.3)",
  "requested_url": "\nhttps://www.tes.com/authn/sign-in?rtn=https%3A%2F%2Fwww.tes.com%2Fteaching-resources",
  "wait_secs": 15,
  "verdict": "CLEAR",
  "outcome": "reported",
  "final_url": "https://www.tes.com/authn/sign-in?rtn=https%3A%2F%2Fwww.tes.com%2Fteaching-resources",
  "title": "TES - Login and Registration",
  "user_agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/152.0.0.0 Safari/537.36 Edg/152.0.0.0",
  "challenge_markers": [],
  "password_form_count": 1,
  "password_input_count": 1,
  "body_text_head": "Please log in\nUsername or email\nPassword\nForgot password?\nLog in\nNot a member yet? Join us for free\nWe're no longer offering login via Facebook or Google, as we move towards a simpler and more consistent login experience across all our products. Learn more.",
  "page_html_bytes": 35772,
  "page_html_file": "report.html"
}
```

## What the reports say

The README's rule is that CHALLENGE alongside a password input is a widget the human satisfies while logging in, and that a wall is CHALLENGE with no password input at all.
Tes needs no rule: no marker fired, a password input is present, and the verdict is CLEAR.
TPT reports CHALLENGE on one marker, `recaptcha (html: recaptcha)`, with `password_form_count` 1 and `password_input_count` 1, which under that rule is a widget rather than a wall.

The page HTML is more specific than the rule, and it says the widget is not there either.
The string `recaptcha` occurs exactly once in the 42,329 bytes TPT returned, inside a JSON feature-flag blob as `"v-3-recaptcha-migration":true`.
The page carries no `g-recaptcha` element, no `data-sitekey`, no `grecaptcha` object and no reference to `google.com/recaptcha` or `gstatic.com/recaptcha`, so nothing rendered a reCAPTCHA on load.
`probe.js` matches the bare needle `recaptcha` anywhere in the HTML, and a flag named `v-3-recaptcha-migration` trips it.
The same scan finds no hCaptcha, DataDome, PerimeterX, Akamai or Kasada asset in either page.
What the Windows webview met on TPT was therefore an ordinary login form, which is a stronger result for D12 than widget-not-wall.
The needle was narrowed on 2026-09-03 on the team lead's decision, a probe heuristic rather than a founder-gated limit: `probe.js` now matches only what a rendered widget or its loader produces, `g-recaptcha`, `grecaptcha`, `data-sitekey`, `recaptcha/api.js` and `recaptcha/enterprise.js`, and a unit test holds the TPT flag string and asserts that no marker fires on it.
Re-reading the two captured pages under those needles, no marker fires on either, so the corrected verdict is CLEAR for both marketplaces.
That re-reading is of the HTML the two runs already wrote; nothing was fetched again, and the reports above are left exactly as the runs produced them.

Cloudflare is absent from the Windows evidence in a way it was not on Linux.
The README records two markers firing on TPT under Linux WebKitGTK, Cloudflare's bot-management script under `/cdn-cgi/challenge-platform` and reCAPTCHA.
Neither `cdn-cgi/challenge-platform` nor `cf-chl` nor `__cf_chl` appears anywhere in the Windows page HTML, so the difference is in what was served rather than in what the probe looked for.
`__cf_bm` would not appear either way, because it is HttpOnly and `document.cookie` cannot see it.
This is one run, on one network, on one day, and the finding is only that this run met no interstitial.

The user agent WebView2 presented is `Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/152.0.0.0 Safari/537.36 Edg/152.0.0.0`, the Edge 152 Chromium string, unrelated to the Safari 605 string WebKitGTK presents on Linux.
Windows is the half that decides D12, because it is the first shipping surface and the Linux result was only indicative.

## D12 passes for both marketplaces on Windows, 2026-09-03

Tes: CLEAR, outcome `reported`, a password form present, no marker.
TPT: recorded as CHALLENGE, outcome `reported`, a password form present, and the only marker a feature-flag name with no bot-protection asset anywhere in the page; under the narrowed needles the corrected reading is CLEAR.
Both are a pass, and the kill gate on the Windows desktop client is answered.

## The invocation defect the runs exposed

Both commands passed `--wait-secs 20` and `--out tpt.json` or `--out tes.json`, and both reports record `wait_secs` 15 and `page_html_file` `report.html`, the defaults; `requested_url` begins with a literal newline in both.
The newline survived because argument splitting on Windows treats only space and tab as separators, so a newline inside a quoted argument reaches `argv` intact, and the navigation still worked because the WHATWG URL parser strips leading control characters before resolving the address.
The flags were ignored because they never reached the process: `requested_url` holds the url and nothing else, so `argv` carried exactly one element and the shell delivered no flag at all.
What the shell did is not recoverable from the reports; a pasted command whose line broke inside the quotes is consistent with both symptoms, but the reports cannot distinguish it from any other loss.

Reproduced on Linux with the same source on 2026-09-03, against a local HTTP fixture rather than any marketplace.
A single argument shaped `"\nhttp://127.0.0.1:8765/login.html"` with no flags reproduces both symptoms exactly, `wait_secs` 15 and `page_html_file` `report.html` and the newline in `requested_url`, and exits 0.
The same argument followed by `--wait-secs 1 --out b.json` honours both flags, which rules out argument order and the newline itself as the cause and leaves only a shell that delivered nothing.

The probe was fixed on the same day.
It trims the url, accepts flags before or after it in either `--flag value` or `--flag=value` form, refuses an unrecognised argument, a missing flag value, a value that is itself a flag, and a url that still holds whitespace after trimming, each with exit status 2 before any window opens and naming the argument.
Every report now records under `argv` the arguments the process received, and the line printed before the window opens names the wait and the file to be written, so a mangled invocation is visible while it runs and provable afterwards.
The report schema is `login-probe/2` for the added field; the two reports above are `login-probe/1` and predate the fix.
The Windows command block in `tools/login-probe/README.md` now puts the flags before the url, so that nothing follows the closing quote for a broken line to strand.
