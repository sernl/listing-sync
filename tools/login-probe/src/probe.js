(function () {
  var US = String.fromCharCode(31);
  var CHUNK = 60000;

  function post(s) { window.ipc.postMessage(s); }
  function clean(v) { return String(v == null ? '' : v).split(US).join(' '); }

  var href = '', title = '', ua = '', cookies = '', html = '';
  try { href = location.href; } catch (e) {}
  try { title = document.title; } catch (e) {}
  try { ua = navigator.userAgent; } catch (e) {}
  try { cookies = document.cookie; } catch (e) {}
  try { html = document.documentElement.outerHTML; } catch (e) {}
  try {
    if (document.doctype) { html = '<!DOCTYPE ' + document.doctype.name + '>\n' + html; }
  } catch (e) {}

  var lowerHtml = html.toLowerCase();
  var lowerHref = href.toLowerCase();
  var cookieNames = cookies.split(';').map(function (c) {
    return c.split('=')[0].trim().toLowerCase();
  }).filter(function (c) { return c.length > 0; });

  var markers = [];

  function scan(list, haystack, where) {
    for (var i = 0; i < list.length; i++) {
      var label = list[i][0], needles = list[i][1];
      for (var j = 0; j < needles.length; j++) {
        if (haystack.indexOf(needles[j]) !== -1) {
          markers.push(label + ' (' + where + ': ' + needles[j] + ')');
          break;
        }
      }
    }
  }

  scan([
    ['cloudflare-interstitial', ['just a moment', 'cf-chl', '__cf_chl', 'cf_chl_opt', 'cdn-cgi/challenge-platform', 'cf-browser-verification', 'checking your browser']],
    ['cloudflare-turnstile', ['challenges.cloudflare.com', 'cf-turnstile']],
    ['hcaptcha', ['hcaptcha.com', 'h-captcha']],
    ['recaptcha', ['g-recaptcha', 'grecaptcha', 'data-sitekey', 'recaptcha/api.js', 'recaptcha/enterprise.js']],
    ['datadome', ['captcha-delivery.com', 'datadome']],
    ['perimeterx', ['perimeterx', 'px-captcha', 'client.px-cloud.net']],
    ['akamai-bot-manager', ['_abck', 'bm_sz', 'ak_bmsc']],
    ['kasada', ['kasada', 'kpsdk']]
  ], lowerHtml, 'html');

  scan([
    ['cloudflare-interstitial', ['cdn-cgi/challenge-platform', '__cf_chl']],
    ['datadome', ['captcha-delivery.com']],
    ['perimeterx', ['px-captcha']]
  ], lowerHref, 'url');

  var cookieMarkers = [
    ['cloudflare-interstitial', ['cf_clearance', '__cf_chl']],
    ['datadome', ['datadome']],
    ['perimeterx', ['_px']],
    ['akamai-bot-manager', ['_abck', 'bm_sz', 'ak_bmsc']],
    ['kasada', ['kpsdk']]
  ];
  for (var i = 0; i < cookieMarkers.length; i++) {
    var label = cookieMarkers[i][0], prefixes = cookieMarkers[i][1], done = false;
    for (var j = 0; j < prefixes.length && !done; j++) {
      for (var k = 0; k < cookieNames.length; k++) {
        if (cookieNames[k].indexOf(prefixes[j]) === 0) {
          markers.push(label + ' (cookie: ' + prefixes[j] + ')');
          done = true;
          break;
        }
      }
    }
  }

  var pwForms = 0, pwInputs = 0;
  try {
    var inputs = document.querySelectorAll('input[type="password" i]');
    pwInputs = inputs.length;
    var forms = [];
    for (var n = 0; n < inputs.length; n++) {
      var f = inputs[n].form || (inputs[n].closest ? inputs[n].closest('form') : null);
      if (f && forms.indexOf(f) === -1) { forms.push(f); }
    }
    pwForms = forms.length;
  } catch (e) {}

  var bodyText = '';
  try { bodyText = (document.body && document.body.innerText) || ''; } catch (e) {}
  bodyText = bodyText.slice(0, 2000);

  post([
    'M', clean(href), clean(title), clean(ua),
    clean(markers.join(' | ')), pwForms, pwInputs, clean(bodyText)
  ].join(US));

  var total = Math.ceil(html.length / CHUNK);
  for (var c = 0; c < total; c++) {
    post('H' + US + c + US + html.slice(c * CHUNK, (c + 1) * CHUNK));
  }
  post('E' + US + total);
})();
