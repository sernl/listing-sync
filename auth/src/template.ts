/**
 * The one HTML template every tam-auth email is rendered through.
 *
 * Tables and inline CSS only: Outlook's Word renderer ignores flexbox, grid and
 * most of the box model, and Gmail strips `<style>` entirely on some clients, so
 * a rule that is not inline is a rule that may not arrive. The single `<style>`
 * block carries one `@media` rule for narrow screens and nothing the layout
 * depends on. That renderer also drops `border-radius` and does not implement
 * `display:inline-block`, so the primary button carries its padding on the
 * enclosing table cell rather than on the anchor: in Outlook it is a square
 * button of the right size, and the rounded corners are what is lost.
 *
 * The module is pure and imports no configuration: the origin arrives as an
 * argument so the template can be rendered and asserted without a configured
 * environment, and so the value stays the one `env` already holds.
 */

/** One message: everything the template renders comes from these fields. */
export interface Delivery {
  readonly to: string;
  readonly subject: string;
  /** The bold opening line. Build it with `greetingFor`, never by hand. */
  readonly greeting: string;
  /** One paragraph, at most two sentences. */
  readonly lead: string;
  /** The primary button's label. */
  readonly action: string;
  /** Where the button points. Must be http or https. */
  readonly url: string;
  /** Alt text for the header illustration, or undefined to leave it out. */
  readonly illustration: string | undefined;
  /**
   * The closing line, which differs by message: ignoring a verification email
   * you did not ask for is safe, ignoring a password change you did not make
   * is not.
   */
  readonly closing: string;
}

const GROUND = '#F7F2E9';
const SURFACE = '#FFFDF9';
const NAV = '#ECE1CD';
const PRIMARY = '#6B4423';
const ACCENT = '#C2543A';
const ADDITIVE = '#3E5A8C';
const TEXT = '#241A12';
const MUTED = '#7C6B5B';
const LINE = '#E4D8C4';

const DISPLAY = "Georgia,'Times New Roman',serif";
const BODY = "-apple-system,'Segoe UI',Helvetica,Arial,sans-serif";

const ESCAPES: Readonly<Record<string, string>> = {
  '&': '&amp;',
  '<': '&lt;',
  '>': '&gt;',
  '"': '&quot;',
  "'": '&#39;',
};

/**
 * Escape a value for both element text and a double-quoted attribute.
 *
 * Every interpolation in `render` goes through this. A display name is
 * attacker-chosen at registration, so an unescaped one would put its markup
 * into a message we send in our own name.
 */
const escapeHtml = (value: string): string =>
  value.replace(/[&<>"']/g, (character) => ESCAPES[character] ?? character);

const GREETING = 'Hey there';
const NAME_LIMIT = 80;

// Two classes, treated differently on purpose. Anything that breaks a line
// becomes a space, so a name split across lines stays two words rather than
// becoming one; U+0085, U+2028 and U+2029 are here because they break lines
// without being `\n`. Everything else unprintable is removed outright, because
// a zero-width character sits *inside* a word and spacing it would split one:
// Cf covers every zero-width and bidi control, including U+200B and the
// U+202A-U+202E overrides that can reverse the display of every line after
// them in a plain-text part, and Cs catches a lone surrogate. Cf is a
// deliberate over-reach — it also drops marks such as U+0600 that carry meaning
// in Arabic script — because a greeting is worth less than the guarantee that
// nothing inside it can move text it does not own.
const LINE_BREAKS = /[\t\n\v\f\r\u0085\u2028\u2029]/gu;
const UNPRINTABLE = /[\p{Cc}\p{Cf}\p{Cs}]/gu;

// Characters that render as nothing but are neither whitespace nor Cf, so
// neither rule above reaches them and a name made only of these would leave the
// greeting with a dangling comma. Braille blank, and the Hangul fillers.
const BLANK_LOOKING = /[\u115F\u1160\u2800\u3164\uFFA0]/gu;

const MARK = /\p{M}/u;
const MARKS_PER_GRAPHEME = 2;

/**
 * Bound how many combining marks may stack on one base character.
 *
 * Marks are not in `UNPRINTABLE` and must not be, since they carry the name in
 * Devanagari, Thai, Hebrew and Vietnamese written in NFD. But nothing else
 * bounds them: 300 combining acutes on one letter is one grapheme inside the
 * 80-code-point cap, and a client that honours the stacking paints them out of
 * the line box and over the illustration and the paragraphs around it.
 *
 * Two survive per base character, which is what the scripts above need: a
 * Devanagari nukta with a virama, a Thai vowel with a tone mark. NFC runs
 * first, so a precomposed Vietnamese or Māori letter arrives carrying no marks
 * at all and is untouched by this -- and, in the other direction, NFC may fold
 * the first loose mark into its base, so the ceiling on what a reader sees is
 * three diacritics rather than two. Bounded is the property that matters here;
 * the exact ceiling is not.
 */
const capMarks = (value: string): string => {
  let stacked = 0;
  let kept = '';
  for (const character of value) {
    if (!MARK.test(character)) {
      stacked = 0;
      kept += character;
      continue;
    }
    stacked += 1;
    if (stacked <= MARKS_PER_GRAPHEME) {
      kept += character;
    }
  }
  return kept;
};

/**
 * Reduce a display name to something safe to place in one line of a message.
 *
 * The name arrives from an unauthenticated sign-up, and the message carrying it
 * goes to an address the same request chose, signed with our DKIM key. An
 * unbounded name is therefore an attacker's own text block inside our email,
 * and an embedded newline makes that block look like a section of it. What
 * survives here is a single bounded line of printable characters, or nothing.
 */
const sanitiseName = (name: string | null | undefined): string => {
  const printable = capMarks(
    (name ?? '')
      .normalize('NFC')
      .replace(LINE_BREAKS, ' ')
      .replace(BLANK_LOOKING, ' ')
      .replace(UNPRINTABLE, ''),
  )
    .replace(/\s+/gu, ' ')
    .trim();
  return [...printable].slice(0, NAME_LIMIT).join('').trim();
};

/**
 * The greeting line: the one place a display name enters a message.
 *
 * Both parts of every message take the greeting from here, so the sanitising
 * above happens once and no caller can bypass it by formatting its own.
 */
export const greetingFor = (name: string | null | undefined): string => {
  const safe = sanitiseName(name);
  return safe === '' ? GREETING : `${GREETING}, ${safe}`;
};

// Built once: constructing a DateTimeFormat is the expensive part, and both are
// immutable. `hourCycle: 'h23'` rather than `hour12: false`, which some ICU
// versions render as 24:00 for midnight.
const UTC_DATE = new Intl.DateTimeFormat('en-NZ', { dateStyle: 'long', timeZone: 'UTC' });
const UTC_CLOCK = new Intl.DateTimeFormat('en-NZ', {
  hour: '2-digit',
  minute: '2-digit',
  hourCycle: 'h23',
  timeZone: 'UTC',
});

/**
 * A timestamp in UTC, with the zone named: "5 September 2026 at 20:00 UTC".
 *
 * UTC because sellers are in the US and the UK as much as here, and named
 * because a time whose zone the reader has to guess is worse than no time.
 * An invalid `Date` throws rather than producing a message with a hole in it.
 */
export const utcTime = (when: Date): string => {
  if (Number.isNaN(when.getTime())) {
    throw new Error('tam-auth: cannot write a timestamp for an invalid date');
  }
  return `${UTC_DATE.format(when)} at ${UTC_CLOCK.format(when)} UTC`;
};

interface FooterLine {
  readonly text: string;
  readonly href: string | undefined;
}

const footerLines = (site: string): readonly FooterLine[] => [
  { text: `© ${String(new Date().getUTCFullYear())} Teachouse`, href: undefined },
  { text: 'Auckland 1072, New Zealand', href: undefined },
  { text: 'Powered by PLE Group', href: site },
];

const originOf = (origin: string): string => origin.replace(/\/+$/, '');

// Zero-width filler after the preheader. Without it an inbox preview runs the
// preheader straight into the first visible text, which is the wordmark and
// then the greeting.
const FILLER = '&#847;&zwnj;&nbsp;'.repeat(30);

// The URL parser silently strips ASCII whitespace while parsing, so a link
// carrying a newline parses clean and the raw string still breaks the text part
// into an extra line the reader has no way to attribute. Checked before
// parsing, on the string as given.
const RAW_CONTROL = /[\u0000-\u001F\u007F]/u;

/**
 * Check an action link and return the form to emit.
 *
 * Every caller builds the link from `env.baseUrl`, so nothing hostile reaches
 * here today. Checking makes that a property of this module rather than of its
 * callers, and the check is worth only as much as it reads: parsing a URL and
 * keeping just the scheme leaves the host, the userinfo and the parser's own
 * normalisation unexamined, so `https://teachouse.example@evil.example/` would
 * pass while pointing at `evil.example`.
 *
 * So: no raw control characters, a scheme of http or https, no userinfo, and an
 * origin equal to the configured one. What is emitted is the parser's `href`
 * rather than the input, so the string in the message is the one that was
 * checked. The errors name the fault and never the link, which carries a token.
 */
const checkedUrl = (url: string, origin: string): string => {
  if (RAW_CONTROL.test(url)) {
    throw new Error('tam-auth: the action link carries a control character');
  }
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    throw new Error('tam-auth: the action link is not an absolute URL');
  }
  if (parsed.protocol !== 'https:' && parsed.protocol !== 'http:') {
    throw new Error(`tam-auth: the action link must be http or https, got ${parsed.protocol}`);
  }
  if (parsed.username !== '' || parsed.password !== '') {
    throw new Error('tam-auth: the action link carries userinfo');
  }
  if (parsed.origin !== originOf(origin)) {
    throw new Error(`tam-auth: the action link points at ${parsed.origin}, not the configured origin`);
  }
  return parsed.href;
};

/**
 * The plain-text alternative, carrying the same content as the HTML.
 *
 * Sent beside `html:` rather than instead of it: Resend accepts both and the
 * client picks, so a text-only reader loses nothing but the styling. The
 * footer's link is written out in full here, because a text part cannot carry
 * an anchor and the destination is part of the requirement, not of the styling.
 */
export const plainText = (delivery: Delivery, origin: string): string => {
  return [
    delivery.greeting,
    '',
    delivery.lead,
    '',
    `${delivery.action}:`,
    checkedUrl(delivery.url, origin),
    '',
    delivery.closing,
    '',
    ...footerLines(originOf(origin)).map((line) =>
      line.href === undefined ? line.text : `${line.text}: ${line.href}`,
    ),
  ].join('\n');
};

const paragraph = (style: string, content: string): string =>
  `<p style="margin:0;${style}">${content}</p>`;

// The illustration carries no instruction -- the greeting, the lead and the
// button say everything the reader must act on, and the message is complete
// with images blocked, which is most clients' default. It is described rather
// than given an empty alt, so a reader who cannot see it is told what the
// sighted reader is looking at instead of being skipped past a blank.
const illustrationRow = (site: string, alt: string): string =>
  `<tr>
              <td align="center" style="padding:28px 32px 0;font-size:0;line-height:0;">
                <img src="${site}/email/teachouse-delivery.png" width="520" height="260" alt="${alt}" style="display:block;width:520px;max-width:100%;height:auto;border:0;" />
              </td>
            </tr>
            `;

/**
 * Render one message as HTML.
 *
 * `origin` is the console's own origin, the value `env.baseUrl` already holds:
 * it serves the mark from its public static tree and is where "Powered by PLE
 * Group" points. Passing it keeps the origin out of this module's imports and
 * out of the markup as a literal.
 */
export const render = (delivery: Delivery, origin: string): string => {
  const url = escapeHtml(checkedUrl(delivery.url, origin));
  // Sliced on a code-point boundary: a UTF-16 cut can leave a lone surrogate,
  // and the preheader is the one string no client shows us before sending.
  const preheader = escapeHtml([...delivery.lead].slice(0, 140).join(''));
  const bare = originOf(origin);
  const site = escapeHtml(bare);
  const lines = footerLines(bare);
  const footer = lines
    .map((line, index) =>
      paragraph(
        `padding-bottom:${index === lines.length - 1 ? '0' : '4'}px;font-family:${BODY};font-size:12px;line-height:1.5;color:${MUTED};`,
        line.href === undefined
          ? escapeHtml(line.text)
          : `<a href="${escapeHtml(line.href)}" style="color:${MUTED};text-decoration:underline;">${escapeHtml(line.text)}</a>`,
      ),
    )
    .join('\n          ');

  return `<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width,initial-scale=1" />
    <meta name="color-scheme" content="light" />
    <meta name="supported-color-schemes" content="light" />
    <meta name="x-apple-disable-message-reformatting" />
    <title>${escapeHtml(delivery.subject)}</title>
    <style>
      @media only screen and (max-width: 620px) {
        .tam-column { width: 100% !important; }
        .tam-pad { padding: 24px !important; }
      }
    </style>
  </head>
  <body bgcolor="${GROUND}" style="margin:0;padding:0;width:100%;background-color:${GROUND};">
    <div style="display:none;max-height:0;max-width:0;overflow:hidden;opacity:0;font-size:1px;line-height:1px;color:${GROUND};">${preheader}${FILLER}</div>
    <table role="presentation" width="100%" cellpadding="0" cellspacing="0" border="0" bgcolor="${GROUND}" style="width:100%;background-color:${GROUND};">
      <tr>
        <td align="center" style="padding:32px 12px;">
          <table role="presentation" class="tam-column" width="600" cellpadding="0" cellspacing="0" border="0" style="width:600px;max-width:600px;background-color:${SURFACE};border:1px solid ${LINE};border-radius:14px;">
            <tr>
              <td style="padding:24px;background-color:${NAV};border-radius:13px 13px 0 0;">
                <table role="presentation" cellpadding="0" cellspacing="0" border="0">
                  <tr>
                    <td width="48" style="padding-right:12px;line-height:0;">
                      <img src="${site}/email/teachouse-mark.png" width="48" height="48" alt="" style="display:block;width:48px;height:48px;border:0;" />
                    </td>
                    <td style="font-family:${DISPLAY};font-size:22px;font-weight:600;letter-spacing:0.2px;color:${PRIMARY};">Teachouse</td>
                  </tr>
                </table>
              </td>
            </tr>
            ${delivery.illustration === undefined ? '' : illustrationRow(site, escapeHtml(delivery.illustration))}<tr>
              <td class="tam-pad" style="padding:32px;">
                ${paragraph(`padding-bottom:16px;font-family:${DISPLAY};font-size:20px;font-weight:700;line-height:1.35;color:${TEXT};`, escapeHtml(delivery.greeting))}
                ${paragraph(`padding-bottom:26px;font-family:${BODY};font-size:15px;line-height:1.6;color:${TEXT};`, escapeHtml(delivery.lead))}
                <table role="presentation" cellpadding="0" cellspacing="0" border="0">
                  <tr>
                    <td align="center" bgcolor="${PRIMARY}" style="border-radius:999px;background-color:${PRIMARY};padding:14px 28px;">
                      <a href="${url}" style="display:inline-block;font-family:${BODY};font-size:14px;font-weight:600;line-height:1.2;color:${SURFACE};text-decoration:none;mso-padding-alt:0;">${escapeHtml(delivery.action)}</a>
                    </td>
                  </tr>
                </table>
                ${paragraph(`padding:26px 0 4px;font-family:${BODY};font-size:13px;line-height:1.5;color:${MUTED};`, 'If the button does not work, copy this link into your browser:')}
                ${paragraph(`padding-bottom:20px;font-family:${BODY};font-size:13px;line-height:1.5;word-break:break-all;`, `<a href="${url}" style="color:${ADDITIVE};text-decoration:underline;">${url}</a>`)}
                ${paragraph(`font-family:${BODY};font-size:13px;line-height:1.5;color:${MUTED};`, escapeHtml(delivery.closing))}
              </td>
            </tr>
            <tr>
              <td align="center" style="padding:24px;border-top:1px solid ${LINE};border-radius:0 0 13px 13px;">
          ${footer}
              </td>
            </tr>
          </table>
        </td>
      </tr>
    </table>
  </body>
</html>
`;
};
