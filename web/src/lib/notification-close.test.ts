// Two things the type system cannot check on its own, in the manner
// `icons.test.ts` already establishes: the size of the control that closes a
// notification, which with no rendering lane is otherwise held by eye alone,
// and that Escape closes it by the same call its click makes.

import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

const SOURCE = new URL("../", import.meta.url);

/** Every declaration block written for a selector, so a rule added later
 *  inside a media query cannot displace the one carrying the metric. */
function blocksFor(sheet: string, selector: string): string[] {
  const found: string[] = [];
  let from = 0;
  for (;;) {
    const start = sheet.indexOf(`${selector} {`, from);
    if (start === -1) {
      return found;
    }
    const end = sheet.indexOf("}", start);
    found.push(sheet.slice(start, end));
    from = end;
  }
}

describe("the close controls", () => {
  // Crude on purpose. `svelte-check` cannot see a stylesheet and no rendering
  // lane gates a merge here, so a grep over the two sheets is the only
  // automated hold on the metric these controls were raised to meet.
  const SHEETS: [string, string][] = [
    ["lib/styles/components.css", ".banner-close"],
    ["app.css", ".toast-close"],
  ];

  for (const [file, selector] of SHEETS) {
    it(`gives ${selector} the full control height and width`, () => {
      const sheet = readFileSync(new URL(file, SOURCE), "utf8");
      const blocks = blocksFor(sheet, selector);
      expect(blocks.length).toBeGreaterThan(0);
      const sized = blocks.filter(
        (block) =>
          /min-width:\s*var\(--control-h\)/.test(block) &&
          /min-height:\s*var\(--control-h\)/.test(block),
      );
      expect(sized).toHaveLength(1);
    });
  }
});

describe("closing a notification with the keyboard", () => {
  // Crude for the same reason as the block above: what holds Escape on these
  // two surfaces is where the handler is attached, and no lane here renders
  // them. The pairing below is the whole of the claim -- the key is answered
  // on the notification's own element, it runs the call the click runs, and
  // no listener is fitted to the window or the document, which is what would
  // take Escape away from a dialog open over the page.
  //
  // The picker's needle is the statement run rather than the verb. It closes
  // from four places, so a window wide enough to hold the Escape path also
  // reaches one of the others, and `closes(` contains the verb as well;
  // either would let an inlined `open = false` pass. The run opens with the
  // default prevented: Escape in a search box holding text clears it, and
  // the `input` that clearing dispatches would reopen the picker.
  const SURFACES: [string, string, string][] = [
    ["lib/Banner.svelte", "banner", "close"],
    ["routes/+layout.svelte", "toast", "closeToast"],
    [
      "lib/FacetPicker.svelte",
      "field fp",
      "event.preventDefault();\n\t\tevent.stopPropagation();\n\t\tvoid close();",
    ],
  ];

  for (const [file, marker, dismisses] of SURFACES) {
    const source = readFileSync(new URL(file, SOURCE), "utf8");

    it(`answers Escape on the ${marker}'s own element`, () => {
      // `[^<]*` cannot cross into another tag, so this is the handler on
      // that element rather than one anywhere in the file.
      expect(new RegExp(`class="${marker}[^<]*onkeydown=`).test(source)).toBe(
        true,
      );
    });

    it(`closes the ${marker} on Escape by the call its click makes`, () => {
      const at = source.indexOf("event.key !== 'Escape'");
      expect(at).toBeGreaterThan(-1);
      expect(source.slice(at, at + 200)).toContain(dismisses);
    });

    it(`leaves every other Escape to whatever is over the ${marker}`, () => {
      // The tag rather than the adjacency: a surface that fits a second
      // window handler puts the key one attribute away from the first. The
      // scan runs to the tag's own `/>` rather than to the first `>`, which
      // an arrow function in an earlier attribute supplies early.
      expect(/<svelte:window(?:(?!\/>)[\s\S])*onkeydown/.test(source)).toBe(
        false,
      );
      expect(source).not.toContain("addEventListener('keydown'");
    });
  }
});
