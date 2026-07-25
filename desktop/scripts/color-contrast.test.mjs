import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";

const css = await readFile(new URL("../src/index.css", import.meta.url), "utf8");

function token(name) {
  const match = css.match(new RegExp(`--${name}:\\s*(#[0-9a-fA-F]{6})\\s*;`));
  assert.ok(match, `missing --${name} color token`);
  return match[1];
}

function selectorDeclaration(selector, property) {
  const escapedSelector = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const block = css.match(new RegExp(`${escapedSelector}\\s*\\{([^}]*)\\}`));
  assert.ok(block, `missing ${selector} selector`);
  const declaration = block[1].match(
    new RegExp(`(?:^|;)\\s*${property}:\\s*([^;]+)\\s*;`),
  );
  assert.ok(declaration, `missing ${property} in ${selector}`);
  return declaration[1].trim();
}

function resolvedColor(value) {
  if (/^#[0-9a-fA-F]{6}$/.test(value)) return value;
  const variable = value.match(/^var\(--([a-z-]+)\)$/);
  assert.ok(variable, `unsupported color declaration: ${value}`);
  return token(variable[1]);
}

function rgb(hex) {
  return [1, 3, 5].map((offset) => Number.parseInt(hex.slice(offset, offset + 2), 16));
}

function luminance(hex) {
  const channels = rgb(hex).map((value) => {
    const normalized = value / 255;
    return normalized <= 0.04045
      ? normalized / 12.92
      : ((normalized + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2];
}

function contrast(foreground, background) {
  const [lighter, darker] = [luminance(foreground), luminance(background)].sort(
    (left, right) => right - left,
  );
  return (lighter + 0.05) / (darker + 0.05);
}

test("small secondary text and status pills meet WCAG AA contrast", () => {
  const pairs = [
    ["muted", "card"],
    ["faint", "card"],
    ["muted", "bg"],
    ["accent-text", "accent-soft"],
    ["green-text", "green-soft"],
    ["amber-text", "amber-soft"],
    ["red-text", "red-soft"],
  ];

  for (const [foreground, background] of pairs) {
    assert.ok(
      contrast(token(foreground), token(background)) >= 4.5,
      `${foreground} on ${background} is below 4.5:1`,
    );
  }
});

test("primary button text meets WCAG AA contrast", () => {
  assert.ok(
    contrast("#ffffff", token("accent-text")) >= 4.5,
    "white primary button text is below 4.5:1",
  );
});

test("keyboard focus ring meets WCAG non-text contrast", () => {
  for (const background of ["card", "bg"]) {
    assert.ok(
      contrast(token("focus-ring"), token(background)) >= 3,
      `focus-ring on ${background} is below 3:1`,
    );
  }
});

test("actual feedback and destructive selectors meet WCAG AA contrast", () => {
  const selectorPairs = [
    [".polish-test-success", "color", ".polish-test-result", "background"],
    [".danger-quiet", "color", ".btn", "background"],
  ];

  for (const [
    foregroundSelector,
    foregroundProperty,
    backgroundSelector,
    backgroundProperty,
  ] of selectorPairs) {
    const foreground = resolvedColor(
      selectorDeclaration(foregroundSelector, foregroundProperty),
    );
    const background = resolvedColor(
      selectorDeclaration(backgroundSelector, backgroundProperty),
    );
    assert.ok(
      contrast(foreground, background) >= 4.5,
      `${foregroundSelector} on ${backgroundSelector} is below 4.5:1`,
    );
  }
});

test("privacy detail inherits each notice tone instead of overriding contrast", () => {
  assert.equal(selectorDeclaration(".privacy-detail", "color"), "inherit");

  for (const selector of [
    ".privacy-notice.local",
    ".privacy-notice.cloud",
    ".privacy-notice.warning",
  ]) {
    const foreground = resolvedColor(selectorDeclaration(selector, "color"));
    const background = resolvedColor(
      selectorDeclaration(selector, "background"),
    );
    assert.ok(
      contrast(foreground, background) >= 4.5,
      `${selector} inherited detail color is below 4.5:1`,
    );
  }
});
