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
