import assert from 'node:assert/strict';
import { test } from 'node:test';
import { type Delivery, greetingFor, plainText, render, utcTime } from './template.ts';

const ORIGIN = 'https://teachouse.example';

const delivery = (overrides: Partial<Delivery> = {}): Delivery => ({
  to: 'kaiako@example.school.nz',
  subject: 'Welcome to Teachouse: confirm your email address',
  greeting: greetingFor('Aroha'),
  lead: 'Welcome to Teachouse. We keep your teaching resources in one place.',
  action: 'Confirm your email address',
  url: `${ORIGIN}/api/auth/verify-email?token=abc`,
  illustration: undefined,
  closing: 'If you did not request this, you can ignore this message.',
  ...overrides,
});

const bothParts = (name: string): { html: string; text: string } => {
  const message = delivery({ greeting: greetingFor(name) });
  return { html: render(message, ORIGIN), text: plainText(message, ORIGIN) };
};

test('a display name carrying markup is escaped rather than rendered', () => {
  const { html } = bothParts('<script>alert("x")</script>');
  assert.ok(!html.includes('<script>alert'), 'the name reached the document as markup');
  assert.ok(html.includes('Hey there, &lt;script&gt;alert(&quot;x&quot;)&lt;/script&gt;'));
});

test('a display name carrying an attribute breakout is escaped', () => {
  const { html } = bothParts('a"><b>b</b>');
  assert.ok(!html.includes('<b>b</b>'));
  assert.ok(html.includes('a&quot;&gt;&lt;b&gt;b&lt;/b&gt;'));
});

test('a newline in the middle of a name cannot open a block of its own', () => {
  const { html, text } = bothParts('Aroha\nBcc: attacker@evil.example');
  assert.ok(text.startsWith('Hey there, Aroha Bcc: attacker@evil.example\n'));
  assert.equal(text.split('\n')[1], '', 'the name broke the greeting across two lines');
  assert.ok(!html.includes('Aroha\nBcc'));
  assert.ok(html.includes('Hey there, Aroha Bcc: attacker@evil.example'));
});

// Built from code points rather than written literally: U+2028 and U+2029 end a
// regular expression literal, and none of these are visible in a diff.
const glyph = (code: number): string => String.fromCodePoint(code);

test('every line break in a name is normalised to a single space', () => {
  // Line feed, carriage return, next line, line separator, paragraph separator.
  for (const code of [0x0a, 0x0d, 0x85, 0x2028, 0x2029]) {
    const point = `U+${code.toString(16).toUpperCase().padStart(4, '0')}`;
    const { html, text } = bothParts(`Aroha${glyph(code)}Bcc: x`);
    assert.equal(text.split('\n')[0], 'Hey there, Aroha Bcc: x', point);
    assert.equal(text.split('\n')[1], '', `${point} broke the greeting in two`);
    assert.ok(html.includes('Hey there, Aroha Bcc: x'), `${point} was not normalised`);
  }
});

test('a 200,000-character name cannot inflate the message', () => {
  const { html, text } = bothParts('A'.repeat(200_000));
  assert.equal(text.split('\n')[0], `Hey there, ${'A'.repeat(80)}`);
  assert.ok(html.includes(`Hey there, ${'A'.repeat(80)}<`));
  assert.ok(!html.includes('A'.repeat(81)));
  // Gmail clips at roughly 102 KB, and the clip would fall above the button.
  assert.ok(html.length < 40_000, `the rendered message grew to ${String(html.length)} bytes`);
});

test('a name of only zero-width or bidi characters falls back to the bare greeting', () => {
  // Zero-width space, ZWNJ, ZWJ, byte-order mark, right-to-left override,
  // right-to-left isolate.
  const invisible = [0x200b, 0x200c, 0x200d, 0xfeff, 0x202e, 0x2067].map(glyph);
  for (const raw of [...invisible, invisible.join(''), `  ${invisible.join('')}  `]) {
    const { html, text } = bothParts(raw);
    assert.equal(text.split('\n')[0], 'Hey there', `${JSON.stringify(raw)} left a dangling comma`);
    assert.ok(html.includes('>Hey there<'));
  }
});

test('a bidi override inside a name cannot reverse what follows it', () => {
  const override = glyph(0x202e);
  const { html, text } = bothParts(`Aroha${override}moc.live`);
  assert.ok(!text.includes(override), 'a bidi override survived into the text part');
  assert.ok(!html.includes(override), 'a bidi override survived into the HTML part');
  assert.equal(text.split('\n')[0], 'Hey there, Arohamoc.live');
});

test('a zero-width character inside a name is removed without splitting the word', () => {
  const { text } = bothParts(`Ar${glyph(0x200b)}oha`);
  assert.equal(text.split('\n')[0], 'Hey there, Aroha');
});

test('a name of only blank-looking characters falls back to the bare greeting', () => {
  // Braille blank and the Hangul fillers: neither whitespace nor format
  // characters, so neither of the other two rules reaches them.
  const blanks = [0x2800, 0x3164, 0x115f, 0x1160, 0xffa0].map(glyph);
  for (const raw of [...blanks, blanks.join(''), `${blanks.join('')}   `]) {
    const { text } = bothParts(raw);
    assert.equal(text.split('\n')[0], 'Hey there', `${JSON.stringify(raw)} left a dangling comma`);
  }
});

test('a stack of combining marks is cut to two per base character', () => {
  const acute = glyph(0x0301);
  const { text } = bothParts(`a${acute.repeat(300)}`);
  const greeting = text.split('\n')[0] ?? '';
  // NFC folds the first acute into the base, so the base is a precomposed
  // U+00E1 and two loose marks follow it: three code points, not 301.
  assert.equal(greeting, `Hey there, ${'á'}${acute.repeat(2)}`);
  assert.equal([...greeting].filter((character) => character === acute).length, 2);
  assert.ok([...greeting].length < 20, 'the stack was not bounded');
});

test('the mark cap counts per base character, not per name', () => {
  const acute = glyph(0x0301);
  const { text } = bothParts(`x${acute.repeat(5)}q${acute.repeat(5)}z`);
  // Neither x nor q has a precomposed acute form, so NFC folds nothing here and
  // exactly two marks survive after each base.
  assert.equal(`x${acute}`.normalize('NFC').length, 2, 'the premise of this test has changed');
  assert.equal(text.split('\n')[0], `Hey there, x${acute.repeat(2)}q${acute.repeat(2)}z`);
});

test('names in scripts that need their marks survive intact', () => {
  const names = [
    'Nguyễn Thị Ánh', // Vietnamese, precomposed by NFC
    'नमस्ते शर्मा', // Devanagari: virama and vowel signs
    'ที่รัก', // Thai: vowel above with a tone mark
    'Tāmaki Makaurau', // Māori macrons
    'Ægir Þórsdóttir',
    'محمد الأحمد',
  ];
  for (const name of names) {
    const { text } = bothParts(name);
    assert.equal(text.split('\n')[0], `Hey there, ${name.normalize('NFC')}`, name);
  }
});

test('a name in decomposed form is composed before anything else touches it', () => {
  // NFD "Nguyễn": without NFC first, the mark cap would see the marks as loose.
  const decomposed = 'Nguyễn'.normalize('NFD');
  assert.ok(decomposed.length > 'Nguyễn'.normalize('NFC').length, 'the premise has changed');
  const { text } = bothParts(decomposed);
  assert.equal(text.split('\n')[0], `Hey there, ${'Nguyễn'.normalize('NFC')}`);
});

test('the greeting falls back when better-auth knows no name', () => {
  assert.equal(greetingFor('Aroha'), 'Hey there, Aroha');
  assert.equal(greetingFor('  Aroha  '), 'Hey there, Aroha');
  assert.equal(greetingFor(''), 'Hey there');
  assert.equal(greetingFor('   '), 'Hey there');
  assert.equal(greetingFor(null), 'Hey there');
  assert.equal(greetingFor(undefined), 'Hey there');
});

test('the footer carries the copyright, the address and the PLE Group link in both parts', () => {
  const message = delivery();
  const html = render(message, ORIGIN);
  const text = plainText(message, ORIGIN);
  assert.match(html, /© 20\d\d Teachouse/u);
  assert.ok(html.includes('Auckland 1072, New Zealand'));
  assert.match(html, /<a href="https:\/\/teachouse\.example"[^>]*>Powered by PLE Group<\/a>/u);
  assert.match(text, /© 20\d\d Teachouse/u);
  assert.ok(text.includes('Auckland 1072, New Zealand'));
  assert.ok(text.includes(`Powered by PLE Group: ${ORIGIN}`));
});

test('the mark and the footer link are taken from the configured origin', () => {
  const message = delivery({ url: 'https://other.example/verify' });
  const html = render(message, 'https://other.example/');
  assert.ok(html.includes('src="https://other.example/email/teachouse-mark.png"'));
  assert.ok(!html.includes('other.example//email'), 'a trailing slash reached the asset path');
  assert.ok(!html.includes('teachouse.example'), 'an origin was hard-coded');
  assert.ok(plainText(message, 'https://other.example/').includes('Group: https://other.example'));
});

test('the button and the written-out link both carry the action url', () => {
  const message = delivery();
  const hrefs = [...render(message, ORIGIN).matchAll(/href="([^"]*)"/gu)].map((match) => match[1]);
  assert.equal(hrefs.filter((href) => href === message.url).length, 2);
  assert.ok(hrefs.includes(ORIGIN));
});

test('the button holds its padding on the table cell, where Outlook honours it', () => {
  const html = render(delivery(), ORIGIN);
  assert.match(html, /<td align="center" bgcolor="#6B4423"[^>]*padding:14px 28px;"/u);
  assert.ok(html.includes('mso-padding-alt:0;'), 'Outlook would double the padding');
});

test('an ampersand in the link is escaped in the attribute and left whole in the text', () => {
  const message = delivery({ url: `${ORIGIN}/verify?token=a&callbackURL=/` });
  assert.ok(render(message, ORIGIN).includes(`href="${ORIGIN}/verify?token=a&amp;callbackURL=/"`));
  assert.ok(plainText(message, ORIGIN).includes(message.url));
});

test('an action link that is not http or https is refused by both parts', () => {
  for (const url of ['javascript:alert(1)', 'data:text/html,<h1>x', 'file:///etc/passwd', 'nope']) {
    const message = delivery({ url });
    assert.throws(() => render(message, ORIGIN), /http or https|absolute URL/u);
    assert.throws(() => plainText(message, ORIGIN), /http or https|absolute URL/u);
  }
});

test('an action link on another host is refused, however it is spelled', () => {
  const elsewhere = [
    'https://evil.example/verify?token=t',
    'https://teachouse.example.evil.example/verify',
    'http://teachouse.example/verify',
    'https://teachouse.example:8443/verify',
  ];
  for (const url of elsewhere) {
    const message = delivery({ url });
    assert.throws(() => render(message, ORIGIN), /configured origin/u, url);
    assert.throws(() => plainText(message, ORIGIN), /configured origin/u, url);
  }
});

test('an action link carrying userinfo is refused, not resolved', () => {
  // Reads as teachouse.example and resolves to evil.example.
  const url = 'https://teachouse.example@evil.example/verify';
  assert.equal(new URL(url).host, 'evil.example', 'the premise of this test has changed');
  const message = delivery({ url });
  assert.throws(() => render(message, ORIGIN), /userinfo|configured origin/u);
  assert.throws(() => plainText(message, ORIGIN), /userinfo|configured origin/u);
});

test('an action link carrying a raw control character is refused', () => {
  // The URL parser strips these while parsing, so a scheme-and-host check sees
  // a clean link and the raw string still breaks the text part into two lines.
  for (const code of [0x0a, 0x0d, 0x09, 0x00]) {
    const url = `${ORIGIN}/verify${String.fromCodePoint(code)}Injected: line`;
    const message = delivery({ url });
    assert.throws(() => render(message, ORIGIN), /control character/u);
    assert.throws(() => plainText(message, ORIGIN), /control character/u);
  }
});

test('a refused link never reaches the message, whatever the fault', () => {
  const faults = [
    'https://evil.example/verify?token=SECRET-TOKEN',
    'https://teachouse.example@evil.example/?token=SECRET-TOKEN',
    'javascript:steal("SECRET-TOKEN")',
  ];
  for (const url of faults) {
    assert.throws(
      () => render(delivery({ url }), ORIGIN),
      (error: Error) => !error.message.includes('SECRET-TOKEN'),
      url,
    );
  }
});

test('the emitted link is the parsed one, so what shipped is what was checked', () => {
  // A raw space would break the link in half in the text part; the parser
  // encodes it, and emitting the parsed href is what carries that through.
  const spaced = delivery({ url: `${ORIGIN}/verify?token=a b` });
  assert.ok(plainText(spaced, ORIGIN).includes(`${ORIGIN}/verify?token=a%20b`));
  assert.ok(!plainText(spaced, ORIGIN).includes('token=a b'));

  // A single slash after a special scheme resolves to the same origin, so it is
  // accepted and written out in the form a client would resolve it to.
  const sloppy = delivery({ url: 'https:/teachouse.example' });
  assert.ok(plainText(sloppy, ORIGIN).includes('https://teachouse.example/'));
});

test('the refusal names the scheme and never the link, which carries a token', () => {
  const message = delivery({ url: 'javascript:steal("SECRET-TOKEN")' });
  assert.throws(
    () => render(message, ORIGIN),
    (error: Error) => !error.message.includes('SECRET-TOKEN'),
  );
});

test('the illustration appears only when the message asks for it', () => {
  const plain = render(delivery(), ORIGIN);
  assert.ok(!plain.includes('teachouse-delivery.png'));

  const alt = 'A courier hands a book to someone at their door.';
  const illustrated = render(delivery({ illustration: alt }), ORIGIN);
  assert.ok(illustrated.includes(`src="${ORIGIN}/email/teachouse-delivery.png"`));
  assert.ok(illustrated.includes(`alt="${alt}"`));
  assert.ok(
    illustrated.indexOf('teachouse-delivery.png') < illustrated.indexOf('Hey there'),
    'the illustration must sit above the greeting',
  );
});

test('the closing line is the one the message chose', () => {
  const closing = 'If this was not you, reset your password straight away.';
  const message = delivery({ closing });
  assert.ok(render(message, ORIGIN).includes(closing));
  assert.ok(plainText(message, ORIGIN).includes(closing));
  assert.ok(!render(message, ORIGIN).includes('you can ignore this message'));
});

test('a timestamp is written in UTC with the zone named', () => {
  assert.equal(utcTime(new Date('2026-09-05T09:12:00Z')), '5 September 2026 at 09:12 UTC');
  // Same instant, already the 6th in Auckland and still the 4th in Honolulu:
  // the zone we state is the zone we convert to, for every reader.
  assert.equal(utcTime(new Date('2026-09-05T20:00:00Z')), '5 September 2026 at 20:00 UTC');
  assert.equal(utcTime(new Date('2026-01-01T00:05:00Z')), '1 January 2026 at 00:05 UTC');
  assert.equal(utcTime(new Date('2026-12-31T23:59:00Z')), '31 December 2026 at 23:59 UTC');
});

test('a timestamp for an invalid date fails rather than printing a hole', () => {
  assert.throws(() => utcTime(new Date('not a date')), /invalid date/u);
});

test('the text alternative carries the same content without markup', () => {
  const message = delivery();
  const text = plainText(message, ORIGIN);
  assert.ok(text.includes(message.greeting));
  assert.ok(text.includes(message.lead));
  assert.ok(text.includes(message.action));
  assert.ok(text.includes(message.url));
  assert.ok(text.includes(message.closing));
  assert.ok(!text.includes('<'), 'markup leaked into the text alternative');
});
