import { readFileSync } from 'node:fs';
import { readdirSync, statSync } from 'node:fs';
import { resolve, join } from 'node:path';
import { describe, it, expect } from 'vitest';

/**
 * Every text-entry control in the app must carry `.form-control`.
 *
 * The reason is a rule that has been in the stylesheet since the first commit:
 *
 *     button, input, select, textarea { color: inherit; }
 *
 * `color: inherit` overrides the user-agent's `color: fieldtext`, so a bare control inherits
 * the page's text colour — which in this dark theme is `--text-primary: #f8fafc`, near white.
 * The control's own background stays the UA default `field`, i.e. white. Near-white text on a
 * white field: the control looks empty while you are typing into it.
 *
 * `.form-control` is what prevents that, by setting a dark translucent background. It also
 * sets `padding: 12px 16px`, which is why a sibling that lacks it is visibly shorter than one
 * that has it — the two defects reported on the Access Tokens screen were the same defect
 * twice: an `<input>` and a `<select>` rendered bare, so one was unreadable and the other was
 * a different height from its neighbour.
 *
 * This is the frontend's equivalent of the backend's `check_access_gate.sh`: not a review note
 * about one screen, a scan that fails for any screen. It reads source rather than rendered DOM
 * because vitest does not load `index.css` into jsdom — the same reason `NavRailCss.test.ts`
 * and `PrintCss.test.ts` read the stylesheet.
 */

const SRC = resolve(__dirname, '../..');

function tsxFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) return tsxFiles(p);
    return p.endsWith('.tsx') ? [p] : [];
  });
}

/**
 * The stylesheets the app actually loads: `main.tsx` imports `index.css` and `print.css`.
 * `App.css` is imported by nothing — 184 lines of Vite-template leftovers whose own rules
 * reference seven custom properties that exist nowhere. Counting it as "defined" would let a
 * class that lives only there pass as styled, which is the exact mistake this file exists to
 * stop, so it is left out on purpose rather than by oversight.
 */
const LOADED_CSS = ['index.css', 'print.css']
  .map((f) => readFileSync(resolve(SRC, f), 'utf8'))
  .join('\n')
  .replace(/\/\*[\s\S]*?\*\//g, '');

/**
 * Every `<input>` / `<select>` opening tag in the tree, with the attributes that belong to it.
 *
 * The tag is read up to the first `>` outside braces, because `onChange={(e) => ...}` puts a
 * `=>` inside the tag and a naive `[^>]*` stops there — which would silently under-report and
 * make this test vacuous. Comments are stripped first for the same class of reason: a prose
 * mention of `<select>` in a comment is not a control, and two of them were in `App.tsx`.
 */
function controls(file: string) {
  const raw = readFileSync(file, 'utf8');
  const src = raw
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/^\s*\/\/.*$/gm, '');
  const found: { line: number; tag: string; kind: string; type: string }[] = [];
  for (const m of src.matchAll(/<(input|select)\b/g)) {
    let j = (m.index ?? 0) + m[0].length;
    let depth = 0;
    while (j < src.length) {
      const c = src[j];
      if (c === '{') depth += 1;
      else if (c === '}') depth -= 1;
      else if (c === '>' && depth === 0) break;
      j += 1;
    }
    const tag = src.slice(m.index ?? 0, j);
    const kind = m[1];
    const type = tag.match(/type=["'{]?(\w+)/)?.[1] ?? (kind === 'select' ? 'select' : 'text');
    // Line number in the file a human will open, not in the comment-stripped text this walker
    // matched against — stripping comments shifts every line below one.
    const at = raw.indexOf(tag.slice(0, 40));
    const line = at === -1 ? 0 : raw.slice(0, at).split('\n').length;
    found.push({ line, tag, kind, type });
  }
  return found;
}

/** Controls whose emptiness is invisible are the text-entry ones. A checkbox, radio, file
 * picker or button does not render typed text, so the contrast rule does not apply to it. */
const TEXTUAL = ['text', 'number', 'password', 'email', 'date', 'search', 'tel', 'url', 'time', 'select'];

describe('text-entry controls are styled', () => {
  const files = tsxFiles(SRC);

  it('finds the controls it claims to find', () => {
    // A scan that matches nothing passes for the wrong reason. This pins that the walker works
    // at all, so a regex regression shows up here rather than as a green run.
    const total = files.flatMap((f) => controls(f)).length;
    expect(total).toBeGreaterThan(20);
  });

  it('every text input and select carries form-control', () => {
    const bare = files.flatMap((f) =>
      controls(f)
        .filter((c) => TEXTUAL.includes(c.type))
        .filter((c) => !c.tag.includes('className'))
        .map((c) => `${f.replace(SRC + '/', '')}:${c.line}  <${c.kind} type=${c.type}>`),
    );
    expect(
      bare,
      `These controls inherit the page's near-white text onto the browser's white field, so typed\n` +
        `text is invisible. Give them className="form-control":\n\n${bare.join('\n')}`,
    ).toEqual([]);
  });

  it('form-control is still the rule that makes that survivable', () => {
    // The scan above is only meaningful while the class does what it is being relied on for.
    // Strip the background and every "styled" control is invisible again, with a green test.
    const css = readFileSync(resolve(SRC, 'index.css'), 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
    const body = css.match(/\.form-control\s*\{([^}]*)\}/)?.[1];
    if (!body) throw new Error('no .form-control rule in index.css');
    expect(body).toMatch(/background/);
    expect(body).toMatch(/padding/);
  });
});

describe('buttons carry a variant that paints them', () => {
  /**
   * `.btn` sets shape only — padding, radius, cursor. It sets **no background and no
   * colour**, so a button whose class list is `btn` and nothing else gets the user-agent's
   * light `buttonface` behind the same inherited near-white text that made the inputs
   * unreadable. The Access Tokens panel hit this twice — the modal's "Done" and every row's
   * delete — while the "Copy" beside them was fine, because `btn-primary` does paint.
   *
   * So the rule is per class **list**, not per token: `btn` is the base class and is always
   * present, so flagging its presence would flag every correct button in the app. What has
   * to be true is that the same list carries a `btn-*` variant, and that the variant names a
   * rule that exists. Checked against the stylesheet rather than a hardcoded list, so a
   * typo'd `btn-primery` fails here instead of rendering as an invisible button.
   */
  const css = readFileSync(resolve(SRC, 'index.css'), 'utf8').replace(/\/\*[\s\S]*?\*\//g, '');
  const defined = new Set([...css.matchAll(/\.([a-z][\w-]*)/g)].map((m) => m[1]));

  const lists = tsxFiles(SRC).flatMap((f) => {
    const src = readFileSync(f, 'utf8');
    const out: { where: string; cls: string }[] = [];
    for (const m of src.matchAll(/className=(?:"([^"]*)"|\{[^}]*?"([^"]+)"[^}]*?\})/g)) {
      const cls = m[1] ?? m[2] ?? '';
      if (!/(?:^|\s)btn(?:\s|$)/.test(cls)) continue;
      out.push({ where: `${f.replace(SRC + '/', '')}:${src.slice(0, m.index).split('\n').length}`, cls });
    }
    return out;
  });

  it('finds the buttons it claims to find', () => {
    // A scan matching nothing passes for the wrong reason.
    expect(lists.length).toBeGreaterThan(10);
  });

  it('every button class list carries a variant that is defined', () => {
    const bad = lists
      .filter((l) => {
        const tokens = l.cls.split(/\s+/);
        const variant = tokens.find((t) => t.startsWith('btn-'));
        return !variant || !defined.has(variant);
      })
      .map((l) => `${l.where}  className="${l.cls}"`);
    expect(
      bad,
      `A list of just \`btn\` has no background, and an undefined variant has no rule at all —\n` +
        `either way the inherited near-white text sits on the browser's own light button face.\n\n${bad.join('\n')}`,
    ).toEqual([]);
  });
});

describe('classes used in JSX exist in a stylesheet', () => {
  /**
   * The general form of the three defects this file exists for. Each one — the inputs, the
   * buttons, the staging panels' `card` — was a component asking for a style that had never
   * been written, and rendering as the browser's own default instead. Nothing failed, no
   * test caught it, and an operator found them by looking at the screen.
   *
   * So: every class named in JSX must resolve to a rule in some stylesheet. The allowlist is
   * the debt, written down rather than discovered again, and a stale entry fails so the list
   * can only shrink.
   */
  const defined = new Set([...LOADED_CSS.matchAll(/\.([a-zA-Z][\w-]*)/g)].map((m) => m[1]));

  // Six of these were cleared when `.text-muted` got a rule and the print statement's dead
  // hooks were deleted; the list went to two in one change, which is the point of it.
  const KNOWN_UNSTYLED: Record<string, string> = {
    'analytics-container': 'a layout hook that was never given a layout; the div stacks naturally',
    'portfolio-detail-container': 'as above',
  };

  const used = new Map<string, string>();
  for (const f of tsxFiles(SRC)) {
    const src = readFileSync(f, 'utf8');
    for (const m of src.matchAll(/className=(?:"([^"]*)"|\{[^}]*?"([^"]+)"[^}]*?\})/g)) {
      for (const token of (m[1] ?? m[2] ?? '').split(/\s+/).filter(Boolean)) {
        if (!defined.has(token) && !used.has(token)) {
          used.set(token, `${f.replace(SRC + '/', '')}:${src.slice(0, m.index).split('\n').length}`);
        }
      }
    }
  }

  it('names no class that has no rule', () => {
    const undeclared = [...used.entries()]
      .filter(([cls]) => !(cls in KNOWN_UNSTYLED))
      .map(([cls, where]) => `${cls}  (first used at ${where})`);
    expect(
      undeclared,
      `These classes appear in JSX and in no stylesheet, so the element gets the browser's\n` +
        `default instead. Either write the rule or add the class to KNOWN_UNSTYLED with a\n` +
        `reason — that list is the debt, and it is meant to shrink.\n\n${undeclared.join('\n')}`,
    ).toEqual([]);
  });

  it('keeps the debt list honest', () => {
    // An allowlist that cannot fail is how an allowlist becomes a lie.
    const stale = Object.keys(KNOWN_UNSTYLED).filter((cls) => !used.has(cls));
    expect(
      stale,
      `These are allowed to be unstyled but are no longer used anywhere: ${stale.join(', ')}. Remove them.`,
    ).toEqual([]);
  });
});

describe('custom properties used in JSX exist', () => {
  /**
   * The same defect one layer down. `color: 'var(--color-error)'` is not a syntax error and
   * not a React error — the declaration is invalid at computed-value time, the element keeps
   * its inherited colour, and nothing complains. Three were live: `--color-error` (three
   * sites in the CSV import report) and `--color-info` rendered the "Rows with Errors" and
   * "New Assets" figures in the same colour as the small grey labels under them, and
   * `--danger-color` survived only because it carried a fallback — which painted a different
   * red from the app's own `--color-danger`, forking the palette in one button.
   *
   * A fallback is not an exemption here. It hides the missing token, which is the part that
   * made these invisible to whoever was looking for them.
   */
  const declared = new Set([...LOADED_CSS.matchAll(/(--[\w-]+)\s*:/g)].map((m) => m[1]));

  const used = tsxFiles(SRC).flatMap((f) => {
    const src = readFileSync(f, 'utf8');
    return [...src.matchAll(/var\((--[\w-]+)/g)].map((m) => ({
      name: m[1],
      where: `${f.replace(SRC + '/', '')}:${src.slice(0, m.index).split('\n').length}`,
    }));
  });

  it('finds the usages it claims to find', () => {
    expect(declared.size).toBeGreaterThan(10);
    expect(used.length).toBeGreaterThan(20);
  });

  it('names no custom property that a loaded stylesheet fails to declare', () => {
    const missing = used
      .filter((u) => !declared.has(u.name))
      .map((u) => `${u.name}  (first used at ${u.where})`);
    expect(
      missing,
      `These are referenced from JSX and declared in no stylesheet the app loads, so the\n` +
        `declaration is dropped and the element keeps an inherited colour nobody chose.\n\n${missing.join('\n')}`,
    ).toEqual([]);
  });
});
