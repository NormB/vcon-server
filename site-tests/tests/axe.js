// The axe-core rules every page is held to, in one place.

// Every WCAG 2.0, 2.1 and 2.2 rule at levels A and AA, plus axe's best
// practices, which hold the landmark rules: one main, content inside
// landmarks, unique landmarks, one h1, headings in order.
export const TAGS = ["wcag2a", "wcag2aa", "wcag21a", "wcag21aa", "wcag22aa", "best-practice"];

// One line per failing rule, then each element and what would fix it.
export function describe(violations) {
  return violations
    .map((v) => `${v.id}: ${v.help}\n${v.nodes.map((n) => `  ${n.target.join(" ")}: ${n.failureSummary}`).join("\n")}`)
    .join("\n");
}

// The contrast ratio (WCAG 2.x) of an element's border against the
// background behind it, for the boundary of a control (1.4.11 asks 3:1).
// Runs in the page: pass it to locator.evaluate. Colors are opaque here.
export function borderContrast(el) {
  const rgb = (c) => c.match(/[\d.]+/g).slice(0, 3).map(Number);
  const lum = ([r, g, b]) => {
    const f = (v) => ((v /= 255) <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
    return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
  };
  let behind = el.parentElement;
  while (behind && /rgba\(.*, 0\)|transparent/.test(getComputedStyle(behind).backgroundColor)) behind = behind.parentElement;
  const a = lum(rgb(getComputedStyle(el).borderTopColor));
  const b = lum(rgb(getComputedStyle(behind ?? document.body).backgroundColor));
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}

// The boxes that would cut the focus ring of the focused element `el`: each
// ancestor that clips or scrolls its contents and whose edge is nearer than
// the ring reaches (the outline's width plus its offset) on the left, the
// right or the top; below, a scrolling box scrolls. Runs in the page: pass
// it to locator.evaluate.
export function ringCutBy(el) {
  const c = getComputedStyle(el);
  const reach = parseFloat(c.outlineWidth) + parseFloat(c.outlineOffset);
  const r = el.getBoundingClientRect();
  const found = [];
  for (let box = el.parentElement; box && box !== document.body; box = box.parentElement) {
    const o = getComputedStyle(box);
    if (o.overflowX === "visible" && o.overflowY === "visible") continue;
    const k = box.getBoundingClientRect();
    if (r.left - reach < k.left - 0.5 || r.right + reach > k.right + 0.5 || r.top - reach < k.top - 0.5) {
      found.push(`${box.tagName.toLowerCase()}.${box.className} (overflow ${o.overflowX} ${o.overflowY})`);
    }
  }
  return found;
}
