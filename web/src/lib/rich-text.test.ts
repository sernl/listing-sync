import { describe, expect, it } from 'vitest';
import {
	descriptionLength,
	htmlLoss,
	htmlText,
	htmlToMarkdown,
	markdownToHtml,
	sanitiseHtml,
	tesText
} from './rich-text';

describe('the allow-list', () => {
	it('keeps the eight elements the editor writes, exactly', () => {
		const clean =
			'<p>A <strong>bold</strong> and <em>quiet</em> word<br>on two lines.</p>' +
			'<ul><li>One</li><li>Two</li></ul><ol><li>First</li></ol>' +
			'<p><a href="https://example.com/a?b=1&amp;c=2">a link</a></p>';
		expect(sanitiseHtml(clean)).toBe(clean);
	});

	it('drops scripts, handlers, styles and unsafe links but keeps the words', () => {
		expect(
			sanitiseHtml(
				'<p onclick="x()" style="color:red">Hi <script>alert(1)</script><span class="c">there</span></p>' +
					'<p><a href="javascript:alert(1)">click</a> <img src=x onerror=alert(1)></p>' +
					'<style>p{}</style><iframe src="https://evil"></iframe>'
			)
		).toBe('<p>Hi there</p><p>click</p>');
	});

	it('reads a browser’s b, i and div as strong, em and paragraphs', () => {
		expect(sanitiseHtml('First line<div><b>Bold</b> <i>it</i></div><div><br></div><div>Last</div>')).toBe(
			'<p>First line</p><p><strong>Bold</strong> <em>it</em></p><p>Last</p>'
		);
	});

	it('turns headings and table rows into paragraphs', () => {
		expect(sanitiseHtml('<h2>Includes</h2><table><tr><td>a</td><td>b</td></tr></table>')).toBe(
			'<p>Includes</p><p>a b</p>'
		);
	});

	it('is empty when nothing visible is written', () => {
		expect(sanitiseHtml('<p><br></p><p>&nbsp; </p><ul><li></li></ul>')).toBe('');
	});

	it('escapes text that looks like markup', () => {
		expect(sanitiseHtml('<p>1 &lt; 2 &amp; &lt;b&gt;</p>')).toBe('<p>1 &lt; 2 &amp; &lt;b&gt;</p>');
	});

	it('moves spaces out of a bold edge, so Markdown can still bold it', () => {
		expect(sanitiseHtml('<p><b>bold </b>word</p>')).toBe('<p><strong>bold</strong> word</p>');
	});

	it('hangs a browser-indented list off the item above it', () => {
		expect(sanitiseHtml('<ul><li>a</li><ul><li>b</li></ul></ul>')).toBe(
			'<ul><li>a<ul><li>b</li></ul></li></ul>'
		);
	});

	it('is idempotent', () => {
		const once = sanitiseHtml('<div>x<b>y</b><ul><li><p>z</p><p>w</p></li></ul></div>');
		expect(sanitiseHtml(once)).toBe(once);
	});
});

describe('what rich text cannot keep', () => {
	it('names what a pasted or imported body loses', () => {
		expect(htmlLoss('<h2>T</h2><img src="a.png"><table><tr><td>x</td></tr></table>')).toEqual([
			'headings',
			'pictures',
			'tables'
		]);
	});

	it('names nothing for the editor’s own output', () => {
		expect(htmlLoss('<p><strong>a</strong> <a href="https://x.test">b</a></p><ol><li>c</li></ol>')).toEqual(
			[]
		);
	});
});

describe('Markdown to rich text', () => {
	it('renders what the Markdown toolbar writes', () => {
		const { html, lost } = markdownToHtml(
			'A **bold** and *quiet* word.\n\n- one\n- two\n\n1. first\n2. second\n\nSee [the site](https://example.com).'
		);
		expect(html).toBe(
			'<p>A <strong>bold</strong> and <em>quiet</em> word.</p>' +
				'<ul><li>one</li><li>two</li></ul><ol><li>first</li><li>second</li></ol>' +
				'<p>See <a href="https://example.com">the site</a>.</p>'
		);
		expect(lost).toEqual([]);
	});

	it('reads a soft break as a space and two trailing spaces as a line break', () => {
		expect(markdownToHtml('one\ntwo  \nthree').html).toBe('<p>one two<br>three</p>');
	});

	it('keeps a loose list’s paragraphs as lines of a tight one', () => {
		expect(markdownToHtml('- one\n\n  more\n- two').html).toBe('<ul><li>one<br>more</li><li>two</li></ul>');
	});

	it('nests an indented list', () => {
		expect(markdownToHtml('- a\n  - b').html).toBe('<ul><li>a<ul><li>b</li></ul></li></ul>');
	});

	it('escapes what is text and honours backslash escapes', () => {
		expect(markdownToHtml('1 < 2 \\*not em\\* & done').html).toBe('<p>1 &lt; 2 *not em* &amp; done</p>');
	});

	it('names what the allow-list will not carry', () => {
		const { html, lost } = markdownToHtml('# Title\n\n![pic](a.png)\n\n| a | b |\n|---|---|\n| 1 | 2 |');
		expect(lost.sort()).toEqual(['headings', 'pictures', 'tables']);
		expect(html).toBe('<p>Title</p><p>a b</p><p>1 2</p>');
	});

	it('drops a link that is not a web address and keeps its words', () => {
		expect(markdownToHtml('[x](javascript:alert(1))').html).not.toContain('href');
	});
});

describe('rich text to Markdown', () => {
	it('writes the toolbar’s own Markdown', () => {
		expect(
			htmlToMarkdown(
				'<p>A <strong>bold</strong> and <em>quiet</em> word<br>next.</p><ul><li>one</li><li>two</li></ul>' +
					'<ol><li>first</li><li>second</li></ol><p><a href="https://example.com">site</a></p>'
			)
		).toBe(
			'A **bold** and *quiet* word  \nnext.\n\n- one\n- two\n\n1. first\n2. second\n\n[site](https://example.com)'
		);
	});

	it('escapes words that would otherwise read as marks', () => {
		expect(htmlToMarkdown('<p>- not a list</p><p>2. not either, *nor* this</p>')).toBe(
			'\\- not a list\n\n2\\. not either, \\*nor\\* this'
		);
	});

	it('round-trips through both directions', () => {
		const html =
			'<p>Fractions &amp; <strong>decimals</strong>, <em>year 4</em>.</p>' +
			'<ul><li>Worksheet<ul><li>Answers</li></ul></li></ul><ol><li>Print</li><li>Teach</li></ol>' +
			'<p>1 &lt; 2 * 3 <a href="https://example.com/x_y">more</a></p>';
		expect(markdownToHtml(htmlToMarkdown(html)).html).toBe(html);
	});
});

describe('the character counter', () => {
	it('counts what a reader sees in rich text, not the tags', () => {
		expect(descriptionLength('<p><strong>Hi</strong> &amp; bye</p><ul><li>x</li></ul>', 'Html')).toBe(9);
		expect(htmlText('<p>a&nbsp;b</p>')).toBe('a b');
	});

	it('counts Markdown as typed', () => {
		expect(descriptionLength('**Hi**', 'Markdown')).toBe(6);
	});
});

describe('the Tes projection', () => {
	it('flattens paragraphs and lists to readable plain text', () => {
		expect(
			tesText(
				'<p>Fractions &amp; decimals 📐</p><p><strong>Includes:</strong></p>' +
					'<ul><li>Worksheet</li><li>Answers</li></ul><ol><li>Print</li><li>Teach</li></ol><p>Enjoy!</p>',
				'Html'
			)
		).toBe('Fractions & decimals\n\nIncludes:\n\n- Worksheet\n- Answers\n\n1. Print\n2. Teach\n\nEnjoy!');
	});

	it('keeps Markdown as written', () => {
		expect(tesText('Level 2 ⭐️ pack — **new**', 'Markdown')).toBe('Level 2 pack — **new**');
	});
});
